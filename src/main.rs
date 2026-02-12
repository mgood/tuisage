use std::path::PathBuf;

mod app;
mod ui;

use app::App;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let args: Vec<String> = std::env::args().collect();
    let file_path = match args.get(1) {
        Some(path) => PathBuf::from(path),
        None => {
            eprintln!("Usage: tuisage <spec-file.usage.kdl>");
            std::process::exit(1);
        }
    };

    let spec = usage::Spec::parse_file(&file_path).map_err(|e| {
        color_eyre::eyre::eyre!(
            "Failed to parse usage spec '{}': {}",
            file_path.display(),
            e
        )
    })?;

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
