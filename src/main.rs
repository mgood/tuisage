use std::io::Write;
use std::path::PathBuf;
use std::process::Command as ProcessCommand;
use std::time::Duration;

use clap::{CommandFactory, Parser};

mod app;
mod command_builder;
mod companion;
mod completion;
mod components;
mod defaults;
mod fields;
mod provider;
mod repeated;
mod theme;
mod ui;
mod validation;

use app::App;

type AppTerminal = ratatui::Terminal<ratatui::backend::CrosstermBackend<std::fs::File>>;

/// Restores the controlling terminal on return, error, or panic unwind.
struct TerminalGuard(std::fs::File);

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = crossterm::execute!(
            &mut self.0,
            crossterm::event::DisableMouseCapture,
            crossterm::cursor::Show,
            crossterm::terminal::LeaveAlternateScreen
        );
        let _ = crossterm::terminal::disable_raw_mode();
    }
}

#[derive(Debug, PartialEq, serde::Serialize)]
struct ComposedCommand {
    executable: String,
    argv: Vec<String>,
}

impl ComposedCommand {
    fn from_parts(parts: Vec<String>) -> color_eyre::Result<Self> {
        let mut parts = parts.into_iter();
        let executable = parts
            .next()
            .filter(|value| !value.is_empty())
            .ok_or_else(|| color_eyre::eyre::eyre!("No executable in constructed command"))?;
        Ok(Self {
            executable,
            argv: parts.collect(),
        })
    }
}

fn strip_clap_usage_metadata(nodes: &mut Vec<kdl::KdlNode>) {
    // clap_usage emits Clap settings that the Usage model does not consume.
    nodes.retain(|node| node.name().value() != "arg_required_else_help");
    for node in nodes {
        if node.name().value() == "flag" {
            node.entries_mut().retain(|entry| {
                !matches!(
                    entry.name().map(|name| name.value()),
                    Some("action" | "builtin")
                )
            });
        }
        if let Some(children) = node.children_mut() {
            strip_clap_usage_metadata(children.nodes_mut());
        }
    }
}

fn parse_generated_usage_spec(source: &str) -> color_eyre::Result<usage::Spec> {
    let mut document: kdl::KdlDocument = source.parse()?;
    // These Clap settings do not affect the form TuiSage builds.
    strip_clap_usage_metadata(document.nodes_mut());
    Ok(document.to_string().parse::<usage::Spec>()?)
}

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

    /// Additional validation executable using JSON stdin/stdout
    #[arg(long)]
    validate: Option<PathBuf>,

    /// Initial field values as JSON, or @PATH to a JSON file
    #[arg(long)]
    defaults: Option<String>,

    /// Query Presage (no value), or select a document file or search directory
    #[arg(long, num_args = 0..=1, default_missing_value = "")]
    presage: Vec<std::ffi::OsString>,

    /// Companion selector, selecting <command>.<selector>.tuisage.kdl
    #[arg(long)]
    selector: Option<String>,

    /// Return executable and ordered argv as JSON without executing
    #[arg(long, conflicts_with = "execute")]
    compose: bool,

    /// Execute the command even if the companion defaults to composition
    #[arg(long)]
    execute: bool,

    /// Startup theme name, or auto with --theme-light and --theme-dark
    #[arg(long)]
    theme: Option<String>,

    /// Named theme for automatic light appearance
    #[arg(long)]
    theme_light: Option<ratatui_themes::ThemeName>,

    /// Named theme for automatic dark appearance
    #[arg(long)]
    theme_dark: Option<ratatui_themes::ThemeName>,

    /// Command to run to get the usage spec (e.g., "mycli --usage")
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    spec_cmd: Vec<String>,
}

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;

    let args = Args::parse();
    let explicit_paths: Vec<_> = args
        .presage
        .iter()
        .filter(|path| !path.is_empty())
        .map(std::path::Path::new)
        .collect();
    if explicit_paths.len() > 1 {
        return Err(color_eyre::eyre::eyre!(
            "--presage accepts only one explicit file or directory"
        ));
    }
    let explicit = explicit_paths.first().copied();
    let query = args.presage.iter().any(|path| path.is_empty());
    if args.selector.is_some() && explicit.is_some_and(|path| !path.is_dir()) {
        return Err(color_eyre::eyre::eyre!(
            "--selector requires a directory, not an explicit --presage file"
        ));
    }

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
        parse_generated_usage_spec(&output).map_err(|e| {
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

    let base = if spec.bin.is_empty() {
        &spec.name
    } else {
        &spec.bin
    };
    let base_parts = shell_words::split(base)
        .map_err(|error| color_eyre::eyre::eyre!("Invalid base command quoting: {error}"))?;
    if base_parts.first().is_none_or(|part| part.is_empty()) {
        return Err(color_eyre::eyre::eyre!("Usage spec has no executable"));
    }

    let producer = args.spec_cmd.first().unwrap_or(&base_parts[0]);
    let executable =
        companion::executable(producer).unwrap_or_else(|| std::path::PathBuf::from(producer));
    let selectors = companion::discover_selectors_in(&executable, explicit)?;
    let companion_path = companion::discover(&executable, explicit, args.selector.as_deref())?;
    let companion = companion_path
        .as_deref()
        .map(|path| companion::Companion::load_with_selectors(path, &spec, &selectors))
        .transpose()?;
    if query {
        println!(
            "{}",
            serde_json::json!({
                "version": 1,
                "path": companion_path.as_ref().map(|path| path.to_string_lossy().to_string()),
                "selectors": selectors.iter().map(|(name, path)| (name, path.to_string_lossy().to_string())).collect::<std::collections::BTreeMap<_, _>>(),
            })
        );
        return Ok(());
    }

    let theme_name = args
        .theme
        .as_deref()
        .or_else(|| companion.as_ref().and_then(|doc| doc.theme.as_deref()));
    let light = args.theme_light.or_else(|| {
        companion.as_ref().and_then(|doc| {
            doc.theme_light
                .as_deref()
                .and_then(|name| name.parse().ok())
        })
    });
    let dark = args.theme_dark.or_else(|| {
        companion
            .as_ref()
            .and_then(|doc| doc.theme_dark.as_deref().and_then(|name| name.parse().ok()))
    });
    let theme_selection = if theme_name == Some("auto") {
        theme::ThemeSelection::parse(theme_name, light, dark)?
    } else {
        theme::ThemeSelection::parse(theme_name, args.theme_light, args.theme_dark)?
    };
    let compose = if args.execute {
        false
    } else if args.compose {
        true
    } else {
        companion
            .as_ref()
            .and_then(|doc| doc.compose)
            .unwrap_or(false)
    };
    let validator = args
        .validate
        .clone()
        .or_else(|| companion.as_ref().and_then(|doc| doc.validator.clone()));
    let initial = merge_defaults(&spec, companion.as_ref(), args.defaults.as_deref())?;

    let tty_path = if cfg!(windows) { "CONOUT$" } else { "/dev/tty" };
    let tty = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(tty_path)
        .map_err(|e| color_eyre::eyre::eyre!("Cannot open controlling terminal: {e}"))?;
    let guard = TerminalGuard(tty.try_clone()?);
    crossterm::terminal::enable_raw_mode()?;
    crossterm::execute!(
        &guard.0,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture,
        crossterm::cursor::Hide
    )?;
    let mut terminal = ratatui::Terminal::new(ratatui::backend::CrosstermBackend::new(tty))?;
    let mut app = App::with_theme(spec, theme_selection.initial());
    app.companion = companion;
    app.automatic_theme = theme_selection.is_automatic();
    if let Some(initial) = initial {
        app.configure_defaults(initial);
    }
    let appearance = theme_selection.watch();
    let result = run_event_loop(
        &mut terminal,
        &mut app,
        compose,
        validator.as_deref(),
        appearance.as_ref(),
    );
    drop(terminal);
    drop(guard);

    match result? {
        Some(command) => {
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer(&mut stdout, &command)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        }
        None if compose => std::process::exit(130),
        None => {}
    }
    Ok(())
}

fn merge_defaults(
    spec: &usage::Spec,
    companion: Option<&companion::Companion>,
    source: Option<&str>,
) -> color_eyre::Result<Option<Vec<defaults::InitialField>>> {
    let mut values = companion
        .map(|doc| doc.defaults.clone())
        .unwrap_or_default();
    if let Some(source) = source {
        let content = if let Some(path) = source.strip_prefix('@') {
            std::fs::read_to_string(path)?
        } else {
            source.to_owned()
        };
        let overrides: serde_json::Value = serde_json::from_str(&content)?;
        let overrides = overrides
            .as_object()
            .ok_or_else(|| color_eyre::eyre::eyre!("--defaults requires a JSON object"))?;
        for (id, value) in overrides {
            if let Some(existing) = values.get(id) {
                if existing["locked"] == true
                    && (existing["value"] != value["value"] || value["locked"] != true)
                {
                    return Err(color_eyre::eyre::eyre!(
                        "CLI default conflicts with locked companion field {id}"
                    ));
                }
            }
            values.insert(id.clone(), value.clone());
        }
    }
    if values.is_empty() {
        Ok(None)
    } else {
        Ok(Some(defaults::parse(
            &serde_json::to_string(&values)?,
            spec,
        )?))
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

fn current_terminal_size(terminal: &mut AppTerminal) -> color_eyre::Result<ratatui::layout::Size> {
    let size = terminal.size()?;
    Ok(ratatui::layout::Size {
        width: size.width,
        height: size.height,
    })
}

fn execute_current_command(terminal: &mut AppTerminal, app: &mut App) -> color_eyre::Result<()> {
    let terminal_size = current_terminal_size(terminal)?;
    if let Err(e) = app.spawn_execution(terminal_size) {
        eprintln!("Failed to execute command: {}", e);
    }
    Ok(())
}

fn apply_appearance_updates(
    app: &mut App,
    appearance: &std::sync::mpsc::Receiver<ratatui_themes::ThemeName>,
) {
    for name in appearance.try_iter() {
        if app.automatic_theme && !app.is_theme_picking() {
            app.theme_name = name;
        }
    }
}

fn run_event_loop(
    terminal: &mut AppTerminal,
    app: &mut App,
    compose: bool,
    validator: Option<&std::path::Path>,
    appearance: Option<&std::sync::mpsc::Receiver<ratatui_themes::ThemeName>>,
) -> color_eyre::Result<Option<ComposedCommand>> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    loop {
        app.poll_completion();
        if let Some(appearance) = appearance {
            apply_appearance_updates(app, appearance);
        }
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
                        app.resize_execution_to_terminal(ratatui::layout::Size { width, height });
                    }
                    _ => {}
                }
            }
            // Continue the loop to redraw (polling-based refresh for terminal output)
            continue;
        }

        // Poll while completion or automatic appearance work is pending.
        if (app.pending_completion.is_some() || appearance.is_some())
            && !event::poll(Duration::from_millis(100))?
        {
            continue;
        }
        // Normal builder mode
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
                    app::Action::Execute => {
                        if !app.validate_submission(validator) {
                            continue;
                        }
                        if compose {
                            return Ok(Some(ComposedCommand::from_parts(
                                app.build_command_parts(),
                            )?));
                        }
                        execute_current_command(terminal, app)?;
                    }
                }
            }
            Event::Mouse(mouse) => match app.handle_mouse(mouse) {
                app::Action::None => {}
                app::Action::Quit => return Ok(None),
                app::Action::Execute => {
                    if !app.validate_submission(validator) {
                        continue;
                    }
                    if compose {
                        return Ok(Some(ComposedCommand::from_parts(
                            app.build_command_parts(),
                        )?));
                    }
                    execute_current_command(terminal, app)?;
                }
            },
            Event::Resize(_, _) => {
                // Terminal will be redrawn on next loop iteration
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    #[test]
    fn presage_query_and_explicit_directory_leave_spec_arguments_intact() {
        let query = Args::try_parse_from([
            "tuisage",
            "--presage",
            "--selector",
            "proxmoxlxc",
            "--cmd",
            "lcc",
            "lcc",
            "--usage",
        ])
        .unwrap();
        assert_eq!(query.presage, vec![std::ffi::OsString::new()]);
        assert_eq!(query.selector.as_deref(), Some("proxmoxlxc"));
        assert_eq!(query.spec_cmd, ["lcc", "--usage"]);

        let directory_query = Args::try_parse_from([
            "tuisage",
            "--presage",
            "/tmp/forms",
            "--presage",
            "--selector",
            "staging",
            "--",
            "demo",
            "--usage",
        ])
        .unwrap();
        assert_eq!(
            directory_query.presage,
            vec![
                std::ffi::OsString::from("/tmp/forms"),
                std::ffi::OsString::new()
            ]
        );
        assert_eq!(directory_query.spec_cmd, ["demo", "--usage"]);
    }

    #[test]
    fn cli_defaults_override_unlocked_and_reject_locked_companion_values() {
        let spec: usage::Spec = "name \"demo\"\narg \"[region]\"".parse().unwrap();
        let mut document = companion::Companion::default();
        document.defaults.insert(
            "root/args/region".into(),
            serde_json::json!({"value": "Sydney", "locked": false}),
        );
        let merged = merge_defaults(
            &spec,
            Some(&document),
            Some(r#"{"root/args/region":{"value":"Zürich"}}"#),
        )
        .unwrap()
        .unwrap();
        assert_eq!(merged[0].value, "Zürich");

        document.defaults.insert(
            "root/args/region".into(),
            serde_json::json!({"value": "Sydney", "locked": true}),
        );
        assert!(merge_defaults(
            &spec,
            Some(&document),
            Some(r#"{"root/args/region":{"value":"Zürich","locked":true}}"#)
        )
        .is_err());
    }

    #[test]
    fn compose_serialization_preserves_exact_arguments() {
        let parts = vec![
            "fake".into(),
            "".into(),
            "two words".into(),
            "$HOME".into(),
            "世界".into(),
        ];
        let command = ComposedCommand::from_parts(parts.clone()).unwrap();
        let output: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&command).unwrap()).unwrap();
        assert_eq!(output["executable"], "fake");
        assert_eq!(output["argv"], serde_json::json!(&parts[1..]));
    }

    #[test]
    fn compose_options_parse_before_spec_command() {
        let args = Args::try_parse_from(["tuisage", "--compose", "demo", "--usage"]).unwrap();
        assert!(args.compose);
        assert_eq!(args.spec_cmd, ["demo", "--usage"]);
    }

    #[test]
    fn compose_keeps_an_explicit_empty_argument() {
        let spec = "name \"demo\"\narg \"[value]\"".parse().unwrap();
        let mut app = App::new(spec);
        app.arg_values[0].supplied = true;
        let command = ComposedCommand::from_parts(app.build_command_parts()).unwrap();
        assert_eq!(command.executable, "demo");
        assert_eq!(command.argv, vec![String::new()]);
    }
}

#[cfg(test)]
mod appearance_tests {
    use super::*;
    use ratatui_themes::ThemeName;

    #[test]
    fn automatic_appearance_updates_theme_until_manual_override() {
        let spec = "name \"fake\"".parse().expect("valid test spec");
        let mut app = App::new(spec);
        app.automatic_theme = true;
        let (sender, receiver) = std::sync::mpsc::channel();

        sender.send(ThemeName::CatppuccinLatte).unwrap();
        apply_appearance_updates(&mut app, &receiver);
        assert_eq!(app.theme_name, ThemeName::CatppuccinLatte);

        app.automatic_theme = false;
        sender.send(ThemeName::Nord).unwrap();
        apply_appearance_updates(&mut app, &receiver);
        assert_eq!(app.theme_name, ThemeName::CatppuccinLatte);
    }
}
