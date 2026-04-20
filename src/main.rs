use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use clap::{CommandFactory, Parser};

mod app;
mod command_builder;
mod components;
mod theme;
mod ui;

use app::App;

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

fn current_terminal_size(
    terminal: &mut ratatui::DefaultTerminal,
) -> color_eyre::Result<ratatui::layout::Size> {
    let size = terminal.size()?;
    Ok(ratatui::layout::Size {
        width: size.width,
        height: size.height,
    })
}

fn execute_current_command(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> color_eyre::Result<()> {
    let terminal_size = current_terminal_size(terminal)?;
    if let Err(e) = app.spawn_execution(terminal_size) {
        eprintln!("Failed to execute command: {}", e);
    }
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
                        app.resize_execution_to_terminal(ratatui::layout::Size {
                            width,
                            height,
                        });
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
                    app::Action::Execute => execute_current_command(terminal, app)?,
                }
            }
            Event::Mouse(mouse) => match app.handle_mouse(mouse) {
                app::Action::None => {}
                app::Action::Quit => return Ok(()),
                app::Action::Execute => execute_current_command(terminal, app)?,
            },
            Event::Resize(_, _) => {
                // Terminal will be redrawn on next loop iteration
            }
            _ => {}
        }
    }
}
