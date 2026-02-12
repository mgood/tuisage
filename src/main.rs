use std::path::PathBuf;
use std::process::Command as ProcessCommand;

use clap::{CommandFactory, Parser};

mod app;
mod ui;

use app::App;

/// TUI application for interactively building CLI commands from usage specs
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Base command to build (e.g., "mise run")
    #[arg(long)]
    cmd: Option<String>,

    /// Command to run to get the usage spec (e.g., "mise tasks ls --usage")
    #[arg(long)]
    spec_cmd: Option<String>,

    /// Path to a usage spec file
    #[arg(long)]
    spec_file: Option<PathBuf>,

    /// Generate usage spec for this tool
    #[arg(long)]
    usage: bool,
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
    let has_spec_cmd = args.spec_cmd.is_some();
    let has_spec_file = args.spec_file.is_some();

    if has_spec_cmd && has_spec_file {
        return Err(color_eyre::eyre::eyre!(
            "Cannot specify both --spec-cmd and --spec-file. Use --help for usage information."
        ));
    }

    if !has_spec_cmd && !has_spec_file {
        return Err(color_eyre::eyre::eyre!(
            "Must specify either --spec-cmd or --spec-file. Use --help for usage information."
        ));
    }

    let mut spec = if let Some(ref spec_cmd) = args.spec_cmd {
        // Run the command and capture its stdout as a usage spec
        let output = run_spec_command(spec_cmd)?;
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

    match result {
        Ok(Some(command)) => {
            println!("{command}");
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(e) => Err(e),
    }
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

fn run_event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    app: &mut App,
) -> color_eyre::Result<Option<String>> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    loop {
        terminal.draw(|frame| ui::render(frame, app))?;

        match event::read()? {
            Event::Key(key) => {
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                // Global quit shortcuts
                if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
                    return Ok(None);
                }

                match app.handle_key(key) {
                    app::Action::None => {}
                    app::Action::Quit => return Ok(None),
                    app::Action::Accept => {
                        let cmd = app.build_command();
                        if !cmd.is_empty() {
                            return Ok(Some(cmd));
                        }
                    }
                }
            }
            Event::Mouse(mouse) => match app.handle_mouse(mouse) {
                app::Action::None => {}
                app::Action::Quit => return Ok(None),
                app::Action::Accept => {
                    let cmd = app.build_command();
                    if !cmd.is_empty() {
                        return Ok(Some(cmd));
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
