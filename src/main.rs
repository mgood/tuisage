use std::io::Read;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use clap::{CommandFactory, Parser};
use portable_pty::{CommandBuilder, NativePtySystem, PtySize, PtySystem};

mod app;
mod ui;
mod widgets;

use app::{App, ExecutionState};

/// TUI application for interactively building CLI commands from usage specs
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Base command to build (e.g., "mise run")
    #[arg(long)]
    cmd: Option<String>,

    /// Path to a usage spec file
    #[arg(long)]
    spec_file: Option<PathBuf>,

    /// Generate usage spec for this tool
    #[arg(long)]
    usage: bool,

    /// Command to run to get the usage spec (e.g., "mycli --usage")
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    spec_cmd: Vec<String>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let args = Args::parse();

    // Handle --usage flag to output usage spec
    if args.usage {
        let mut cmd = Args::command();
        let bin_name = std::env::args()
            .next()
            .unwrap_or_else(|| "tuisage".to_string());
        let mut buf = Vec::new();
        clap_usage::generate(&mut cmd, bin_name, &mut buf);
        print!("{}", String::from_utf8_lossy(&buf));
        return Ok(());
    }

    // Determine the usage spec source
    let has_spec_cmd = !args.spec_cmd.is_empty();
    let has_spec_file = args.spec_file.is_some();

    if has_spec_cmd && has_spec_file {
        return Err(color_eyre::eyre::eyre!(
            "Cannot specify both a spec command and --spec-file. Use --help for usage information."
        ));
    }

    if !has_spec_cmd && !has_spec_file {
        return Err(color_eyre::eyre::eyre!(
            "Must specify either a spec command or --spec-file. Use --help for usage information."
        ));
    }

    let mut spec = if has_spec_cmd {
        // Join the arguments into a single command string and run it
        let spec_cmd = args.spec_cmd.join(" ");
        let output = run_spec_command(&spec_cmd)?;
        output.parse::<usage::Spec>().map_err(|e| {
            color_eyre::eyre::eyre!(
                "Failed to parse usage spec from command '{}': {}",
                spec_cmd,
                e
            )
        })?
    } else if let Some(ref spec_file) = args.spec_file {
        usage::Spec::parse_file(spec_file).map_err(|e| {
            color_eyre::eyre::eyre!(
                "Failed to parse usage spec '{}': {}",
                spec_file.display(),
                e
            )
        })?
    } else {
        unreachable!()
    };

    // Override the bin name if --cmd is provided
    if let Some(ref cmd) = args.cmd {
        spec.bin = cmd.clone();
    }

    // Enable mouse capture before initializing the terminal
    crossterm::execute!(std::io::stderr(), crossterm::event::EnableMouseCapture)?;

    let mut terminal = ratatui::init();
    let mut app = App::new(spec);
    let result = run_event_loop(&mut terminal, &mut app);

    // Restore terminal and disable mouse capture
    ratatui::restore();
    crossterm::execute!(std::io::stderr(), crossterm::event::DisableMouseCapture)?;

    result
}

/// Run a shell command and return its stdout as a string.
fn run_spec_command(cmd: &str) -> color_eyre::Result<String> {
    let output = if cfg!(target_os = "windows") {
        ProcessCommand::new("cmd")
            .args(["/C", cmd])
            .output()
            .map_err(|e| color_eyre::eyre::eyre!("Failed to run spec command '{}': {}", cmd, e))?
    } else {
        ProcessCommand::new("sh")
            .args(["-c", cmd])
            .output()
            .map_err(|e| color_eyre::eyre::eyre!("Failed to run spec command '{}': {}", cmd, e))?
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(color_eyre::eyre::eyre!(
            "Spec command '{}' failed with status {}{}",
            cmd,
            output.status,
            if stderr.is_empty() {
                String::new()
            } else {
                format!(": {}", stderr.trim())
            }
        ));
    }

    String::from_utf8(output.stdout).map_err(|e| {
        color_eyre::eyre::eyre!(
            "Spec command '{}' produced invalid UTF-8 output: {}",
            cmd,
            e
        )
    })
}

/// Spawn the built command in a PTY and set up the execution state in the app.
fn spawn_command(app: &mut App, terminal_size: ratatui::layout::Size) -> color_eyre::Result<()> {
    let parts = app.build_command_parts();
    if parts.is_empty() {
        return Err(color_eyre::eyre::eyre!("No command to execute"));
    }

    let command_display = app.build_command();

    // Build the PTY command with separate arguments
    let mut cmd = CommandBuilder::new(&parts[0]);
    for arg in &parts[1..] {
        cmd.arg(arg);
    }
    if let Ok(cwd) = std::env::current_dir() {
        cmd.cwd(cwd);
    }

    // Size the PTY to fit the terminal area (no border around output pane)
    // Layout: 3 rows for command display + 1 row for status bar
    let pty_rows = terminal_size.height.saturating_sub(4).max(4);
    let pty_cols = terminal_size.width.max(20);

    let pty_system = NativePtySystem::default();
    let pair = pty_system
        .openpty(PtySize {
            rows: pty_rows,
            cols: pty_cols,
            pixel_width: 0,
            pixel_height: 0,
        })
        .map_err(|e| color_eyre::eyre::eyre!("Failed to open PTY: {}", e))?;

    let parser = Arc::new(RwLock::new(vt100::Parser::new(pty_rows, pty_cols, 0)));
    let exited = Arc::new(AtomicBool::new(false));
    let exit_status: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));

    // Spawn the child process on the slave side
    let child_result = pair.slave.spawn_command(cmd);
    // Drop the slave immediately — the child owns it now
    drop(pair.slave);

    let mut child = child_result
        .map_err(|e| color_eyre::eyre::eyre!("Failed to spawn command '{}': {}", parts[0], e))?;

    // Spawn a thread to wait for the child to exit
    {
        let exited = exited.clone();
        let exit_status = exit_status.clone();
        std::thread::spawn(move || {
            match child.wait() {
                Ok(status) => {
                    if let Ok(mut s) = exit_status.lock() {
                        *s = Some(format!("{}", status));
                    }
                }
                Err(e) => {
                    if let Ok(mut s) = exit_status.lock() {
                        *s = Some(format!("error: {}", e));
                    }
                }
            }
            exited.store(true, Ordering::Relaxed);
        });
    }

    // Spawn a thread to read PTY output and feed it to the vt100 parser
    let mut reader = pair
        .master
        .try_clone_reader()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to clone PTY reader: {}", e))?;

    {
        let parser = parser.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break, // EOF
                    Ok(size) => {
                        if let Ok(mut p) = parser.write() {
                            p.process(&buf[..size]);
                        }
                    }
                    Err(_) => break,
                }
            }
        });
    }

    // Get the writer for forwarding keyboard input
    let writer = pair
        .master
        .take_writer()
        .map_err(|e| color_eyre::eyre::eyre!("Failed to take PTY writer: {}", e))?;

    let pty_writer: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
        Arc::new(Mutex::new(Some(writer)));

    // Store the master for resizing
    let pty_master: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>> =
        Arc::new(Mutex::new(Some(pair.master)));

    // Set up a thread to drop the writer and master when the child exits
    {
        let exited = exited.clone();
        let pty_writer = pty_writer.clone();
        let pty_master = pty_master.clone();
        std::thread::spawn(move || {
            // Wait for the child to exit
            while !exited.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(50));
            }
            // Give a moment for final output to be read
            std::thread::sleep(Duration::from_millis(100));
            // Drop the writer so the reader thread can finish
            if let Ok(mut w) = pty_writer.lock() {
                *w = None;
            }
            // Drop the master to clean up
            if let Ok(mut m) = pty_master.lock() {
                *m = None;
            }
        });
    }

    let state = ExecutionState {
        command_display,
        parser,
        pty_writer,
        pty_master,
        exited,
        exit_status,
    };

    app.start_execution(state);
    Ok(())
}

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> color_eyre::Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        // Use polling when in execution mode so we can refresh the terminal output
        if app.is_executing() {
            if event::poll(Duration::from_millis(16))? {
                match event::read()? {
                    Event::Key(key) => {
                        if key.kind != KeyEventKind::Press {
                            continue;
                        }

                        // Ctrl-C during execution: forward to PTY (handled in app)
                        // But if the process has exited, just close
                        app.handle_key(key);
                    }
                    Event::Resize(width, height) => {
                        // Resize the PTY to match the new terminal size
                        // Layout: 3 rows for command display + 1 row for status bar
                        let pty_rows = height.saturating_sub(4).max(4);
                        let pty_cols = width.max(20);
                        app.resize_pty(pty_rows, pty_cols);
                    }
                    _ => {}
                }
            }
            // Continue the loop to redraw (polling-based refresh for terminal output)
            continue;
        }

        // Normal builder mode: blocking event read
        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global quit shortcuts
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(());
                }

                match app.handle_key(key) {
                    app::Action::None => {}
                    app::Action::Quit => return Ok(()),
                    app::Action::Execute => {
                        let size = terminal.size()?;
                        let term_size = ratatui::layout::Size {
                            width: size.width,
                            height: size.height,
                        };
                        if let Err(e) = spawn_command(app, term_size) {
                            // If spawning fails, we could show an error but for now just log it
                            // The user will see the error in the terminal
                            eprintln!("Failed to execute command: {}", e);
                        }
                    }
                }
            }
            Event::Mouse(mouse) => match app.handle_mouse(mouse) {
                app::Action::None => {}
                app::Action::Quit => return Ok(()),
                app::Action::Execute => {
                    let size = terminal.size()?;
                    let term_size = ratatui::layout::Size {
                        width: size.width,
                        height: size.height,
                    };
                    if let Err(e) = spawn_command(app, term_size) {
                        eprintln!("Failed to execute command: {}", e);
                    }
                }
            },
            Event::Resize(_, _) => {
                // Terminal will be redrawn on next loop iteration
            }
            _ => {}
        }
    }
}
