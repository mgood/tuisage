use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use clap::{CommandFactory, Parser};

mod app;
mod ui;

use app::App;

/// TUI application for interacting with CLI commands defined by usage specs
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to usage spec file, or "-" to read from stdin
    spec_file: Option<String>,

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

    let spec = match args.spec_file.as_deref() {
        Some("-") => {
            // Explicit stdin: read usage spec from stdin
            let mut input = String::new();
            std::io::stdin().read_to_string(&mut input)?;
            input.parse::<usage::Spec>().map_err(|e| {
                color_eyre::eyre::eyre!("Failed to parse usage spec from stdin: {}", e)
            })?
        }
        Some(path) => {
            let file_path = PathBuf::from(path);
            usage::Spec::parse_file(&file_path).map_err(|e| {
                color_eyre::eyre::eyre!(
                    "Failed to parse usage spec '{}': {}",
                    file_path.display(),
                    e
                )
            })?
        }
        None => {
            // No argument: check if stdin is piped (not a TTY)
            if std::io::stdin().is_terminal() {
                // Let clap handle the error message
                return Err(color_eyre::eyre::eyre!(
                    "No spec file provided. Use --help for usage information."
                ));
            } else {
                let mut input = String::new();
                std::io::stdin().read_to_string(&mut input)?;
                input.parse::<usage::Spec>().map_err(|e| {
                    color_eyre::eyre::eyre!("Failed to parse usage spec from stdin: {}", e)
                })?
            }
        }
    };

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
