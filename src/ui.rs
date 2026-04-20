use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    Frame,
};

#[cfg(test)]
extern crate insta;

use crate::app::{collect_visible_flags, App, AppMode, Focus, UiLayout};
use crate::components::arg_panel::ArgRenderData;
use crate::components::flag_panel::FlagRenderData;
use crate::components::help_bar::{HelpBar, Keybind};
use crate::components::preview::CommandPreview;
use crate::components::{Component, RenderableComponent};
use crate::theme::UiColors;

/// Render the full UI: command panel, flag panel, arg panel, preview, help bar.
pub fn render(frame: &mut Frame, app: &mut App) {
    let palette = app.palette();
    let colors = UiColors::from_palette(&palette);

    if app.mode == AppMode::Executing {
        if let Some(ref mut exec) = app.execution {
            exec.render(frame.area(), frame.buffer_mut(), &colors);
        }
        return;
    }

    let area = frame.area();

    // Top-level vertical layout:
    //   [command preview]
    //   [main content area]
    //   [help / status bar]
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // command preview
            Constraint::Min(6),    // main content
            Constraint::Length(1), // help text
        ])
        .split(area);

    let mut layout = UiLayout::new();

    render_preview(frame, app, outer[0], &colors, &mut layout);
    render_main_content(frame, app, outer[1], &colors, &mut layout);
    render_help_bar(frame, app, outer[2], &colors, &mut layout);

    // Register preview area for click hit-testing
    layout.click_regions.register(outer[0], Focus::Preview);

    // Viewport for overlays: main content area (excludes preview + help bar)
    let overlay_viewport = outer[1];

    // Render flag panel overlays (from ChoiceSelectComponent)
    {
        app.flag_panel
            .set_choice_select_mouse_position(app.mouse_position);
        let overlays = app.flag_panel.collect_overlays();
        for req in overlays {
            let overlay_area =
                crate::components::clamp_overlay(req.anchor, req.size, overlay_viewport);
            req.content.render(overlay_area, frame.buffer_mut(), &colors);
            layout.flag_overlay_rect = Some(overlay_area);
        }
    }

    // Render arg panel overlays (from ChoiceSelectComponent)
    {
        app.arg_panel
            .set_choice_select_mouse_position(app.mouse_position);
        let overlays = app.arg_panel.collect_overlays();
        for req in overlays {
            let overlay_area =
                crate::components::clamp_overlay(req.anchor, req.size, overlay_viewport);
            req.content.render(overlay_area, frame.buffer_mut(), &colors);
            layout.arg_overlay_rect = Some(overlay_area);
        }
    }

    // Render theme picker overlays (on top of everything)
    {
        app.theme_picker.set_viewport(area);
        let overlays = app.theme_picker.collect_overlays();
        for req in overlays {
            let overlay_area =
                crate::components::clamp_overlay(req.anchor, req.size, area);
            req.content.render(overlay_area, frame.buffer_mut(), &colors);
            layout.theme_overlay_rect = Some(overlay_area);
        }
    }

    app.layout = layout;
}

/// Render the main content area with panels for commands, flags, and args.
fn render_main_content(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    colors: &UiColors,
    layout: &mut UiLayout,
) {
    // Commands tree should always be visible (shows entire tree, not just subcommands)
    let has_commands = app.command_panel.total_visible() > 0;

    // Always show Flags and Args panes, even if empty
    if has_commands {
        // Split horizontally: commands on left, flags+args on right
        let h_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        render_command_list(frame, app, h_split[0], colors, layout);

        // Always split vertically for flags and args
        let v_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(h_split[1]);
        render_flag_list(frame, app, v_split[0], colors, layout);
        render_arg_list(frame, app, v_split[1], colors, layout);
    } else {
        // No commands - still show flags and args
        let v_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        render_flag_list(frame, app, v_split[0], colors, layout);
        render_arg_list(frame, app, v_split[1], colors, layout);
    }
}

/// Render the command tree panel using the CommandPanelComponent.
fn render_command_list(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    colors: &UiColors,
    layout: &mut UiLayout,
) {
    // Register area for click hit-testing
    layout.click_regions.register(area, Focus::Commands);

    // Push display state into the component before rendering
    let focused = app.focus() == Focus::Commands;
    app.command_panel.set_focused(focused);
    app.command_panel.set_mouse_position(app.mouse_position);

    app.command_panel.render(area, frame.buffer_mut(), colors);
}
fn render_flag_list(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    colors: &UiColors,
    layout: &mut UiLayout,
) {
    // Register area for click hit-testing
    layout.click_regions.register(area, Focus::Flags);

    // Push display state into the component before rendering
    let focused = app.focus() == Focus::Flags;
    app.flag_panel.set_focused(focused);
    app.flag_panel.set_mouse_position(app.mouse_position);

    // Build external data. Use direct field access for spec/command_path/flag_values
    // so that the immutable borrows are on individual fields, not the whole App.
    // This allows the mutable borrow of flag_panel for render_with_data().
    let mut cmd = &app.spec.cmd;
    for name in &app.command_path {
        if let Some(sub) = cmd.find_subcommand(name) {
            cmd = sub;
        } else {
            break;
        }
    }
    let flags = collect_visible_flags(cmd, &app.spec);
    let key = app.command_path.join(" ");
    let flag_values: Vec<(String, crate::app::FlagValue)> = app
        .flag_values
        .get(&key)
        .cloned()
        .unwrap_or_default();
    let flag_defaults: Vec<Option<String>> =
        flags.iter().map(|f| f.default.first().cloned()).collect();

    let data = FlagRenderData {
        flags: &flags,
        flag_values: &flag_values,
        flag_defaults: &flag_defaults,
    };

    app.flag_panel
        .render_with_data(area, frame.buffer_mut(), colors, data);
}

fn render_arg_list(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    colors: &UiColors,
    layout: &mut UiLayout,
) {
    // Register area for click hit-testing
    layout.click_regions.register(area, Focus::Args);

    // Push display state into the component before rendering
    let focused = app.focus() == Focus::Args;
    app.arg_panel.set_focused(focused);
    app.arg_panel.set_mouse_position(app.mouse_position);

    let data = ArgRenderData {
        arg_values: &app.arg_values,
    };

    app.arg_panel
        .render_with_data(area, frame.buffer_mut(), colors, data);
}

fn render_help_bar(
    frame: &mut Frame,
    app: &mut App,
    area: Rect,
    colors: &UiColors,
    layout: &mut UiLayout,
) {
    let keybinds: &[Keybind] = if app.is_theme_picking() {
        &[
            Keybind { key: "↑↓", desc: "navigate" },
            Keybind { key: "⏎", desc: "confirm" },
            Keybind { key: "Esc", desc: "cancel" },
        ]
    } else if app.is_choosing() {
        &[
            Keybind { key: "↑↓", desc: "select" },
            Keybind { key: "⏎", desc: "confirm" },
            Keybind { key: "Esc", desc: "keep text" },
        ]
    } else if app.is_editing() {
        &[
            Keybind { key: "⏎", desc: "confirm" },
            Keybind { key: "Esc", desc: "cancel" },
        ]
    } else if app.is_filtering() {
        &[
            Keybind { key: "⏎", desc: "apply" },
            Keybind { key: "Esc", desc: "clear" },
            Keybind { key: "↑↓", desc: "navigate" },
        ]
    } else if app.filter_active() {
        &[
            Keybind { key: "↑↓/jk", desc: "next match" },
            Keybind { key: "/", desc: "new filter" },
            Keybind { key: "Esc", desc: "clear filter" },
        ]
    } else {
        match app.focus() {
            Focus::Commands => &[
                Keybind { key: "↑↓", desc: "navigate" },
                Keybind { key: "⇥", desc: "next" },
                Keybind { key: "/", desc: "filter" },
                Keybind { key: "^r", desc: "run" },
                Keybind { key: "q", desc: "quit" },
            ],
            Focus::Flags => &[
                Keybind { key: "⏎/Space", desc: "toggle" },
                Keybind { key: "↑↓", desc: "navigate" },
                Keybind { key: "⇥", desc: "next" },
                Keybind { key: "/", desc: "filter" },
                Keybind { key: "^r", desc: "run" },
                Keybind { key: "q", desc: "quit" },
            ],
            Focus::Args => &[
                Keybind { key: "⏎", desc: "edit" },
                Keybind { key: "↑↓", desc: "navigate" },
                Keybind { key: "⇥", desc: "next" },
                Keybind { key: "/", desc: "filter" },
                Keybind { key: "^r", desc: "run" },
                Keybind { key: "q", desc: "quit" },
            ],
            Focus::Preview => &[
                Keybind { key: "⏎", desc: "run" },
                Keybind { key: "⇥", desc: "next" },
                Keybind { key: "q", desc: "quit" },
            ],
        }
    };

    let theme_display = app.theme_name.display_name();
    let widget = HelpBar::new(keybinds, theme_display, colors);

    // Store the theme indicator rect for mouse click detection
    layout.theme_indicator_rect = Some(widget.theme_indicator_rect(area));

    frame.render_widget(widget, area);
}

/// Render the command preview bar at the bottom with colorized parts.
fn render_preview(
    frame: &mut Frame,
    app: &App,
    area: Rect,
    colors: &UiColors,
    _layout: &mut UiLayout,
) {
    let is_focused = app.focus() == Focus::Preview;
    let command = app.build_command();
    let bin = if app.spec.bin.is_empty() {
        &app.spec.name
    } else {
        &app.spec.bin
    };

    let widget = CommandPreview::new(&command, bin, &app.command_path, is_focused, colors);
    frame.render_widget(widget, area);
}

/// Format a flag's display string (e.g., "-f, --force" or "--verbose").
#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::FlagValue;
    use crate::components::flag_panel::flag_display_string;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{backend::TestBackend, Terminal};
    use ratatui_themes::ThemeName;

    fn sample_spec() -> usage::Spec {
        let input = include_str!("../fixtures/sample.usage.kdl");
        input
            .parse::<usage::Spec>()
            .expect("Failed to parse sample spec")
    }

    /// Parse a custom usage spec string into a Spec for targeted tests.
    fn parse_spec(input: &str) -> usage::Spec {
        input.parse::<usage::Spec>().expect("Failed to parse spec")
    }

    fn render_to_string(app: &mut App, width: u16, height: u16) -> String {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| render(frame, app)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut output = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                let cell = &buffer[(x, y)];
                output.push_str(cell.symbol());
            }
            // Trim trailing whitespace per line for cleaner snapshots
            let trimmed = output.trim_end();
            output = trimmed.to_string();
            output.push('\n');
        }
        output
    }

    // ── Snapshot tests ──────────────────────────────────────────────────

    #[test]
    fn snapshot_root_view() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_config_subcommands() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        // Expand config to show its subcommands
        app.command_panel.expand("config");
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_deploy_leaf() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_run_command() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flags_toggled() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Toggle --rollback (find its index)
        app.set_focus(Focus::Flags);
        let flag_values = app.current_flag_values();
        let rollback_idx = flag_values
            .iter()
            .position(|(n, _)| n == "rollback")
            .unwrap();
        app.set_flag_index(rollback_idx);
        // Toggle it
        let fidx = app.flag_index();
        let vals = app.current_flag_values_mut();
        if let Some((_, FlagValue::Bool(ref mut b))) = vals.get_mut(fidx) {
            *b = true;
        }

        // Set --tag value
        let tag_idx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "tag")
            .unwrap();
        app.set_flag_index(tag_idx);
        let fidx = app.flag_index();
        let vals = app.current_flag_values_mut();
        if let Some((_, FlagValue::String(ref mut s))) = vals.get_mut(fidx) {
            *s = "v1.2.3".to_string();
        }

        // Set arg <environment> = prod
        app.arg_values[0].value = "prod".to_string();

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_editing_arg() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        // Focus on args and start editing
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();
        // Type via handle_key to set editing text and sync value
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for ch in "my-project".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_filter_active() {
        let mut app = App::new(sample_spec());

        // Activate filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        // Type "pl" to filter - this will trigger auto-select
        for c in "pl".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_preview_focused() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        app.arg_values[0].value = "lint".to_string();
        app.set_focus(Focus::Preview);

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_deep_navigation() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["plugin", "install"]);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_choice_select_open() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <environment> with choices dev, staging, prod

        // Open the choice select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_choice_select_filtered() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open the choice select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        // Type "st" to filter
        for c in "st".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_choice_select_with_descriptions() {
        // plugin > update has completions with descriptions=#true
        // Each line in the output is "value:description", which should be
        // displayed right-aligned in the choice select dropdown.
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["plugin", "update"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open the choice select box (triggers completion command)
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        // Only render if completion succeeded (skips if printf not available)
        if !app.is_choosing() {
            return;
        }

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    // ── Minimal spec tests ──────────────────────────────────────────────

    #[test]
    fn snapshot_simple_flags_only() {
        let spec = parse_spec(
            r#"
bin "mytool"
flag "-v --verbose" help="Verbose output"
flag "-f --force" help="Force operation"
flag "--dry-run" help="Show what would happen"
arg "<input>" help="Input file"
arg "[output]" help="Output file"
        "#,
        );
        let mut app = App::new(spec);
        let output = render_to_string(&mut app, 80, 20);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_choices_flag() {
        let spec = parse_spec(
            r#"
bin "mycli"
cmd "format" help="Format code" {
    flag "--style <style>" help="Code style" {
        arg "<style>" {
            choices "compact" "expanded" "default"
        }
    }
    arg "<file>" help="File to format"
}
        "#,
        );
        let mut app = App::new(spec);
        app.navigate_to_command(&["format"]);
        let output = render_to_string(&mut app, 80, 20);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_no_subcommands() {
        let spec = parse_spec(
            r#"
bin "simple"
about "A simple tool"
arg "<file>" help="File to process"
flag "-o --output <path>" help="Output path"
        "#,
        );
        let mut app = App::new(spec);
        let output = render_to_string(&mut app, 80, 20);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_count_flag_incremented() {
        let spec = parse_spec(
            r#"
bin "mycli"
flag "-v --verbose" help="Increase verbosity" count=#true
flag "-q --quiet" help="Quiet mode"
        "#,
        );
        let mut app = App::new(spec);
        // Increment verbose 3 times
        let key = app.command_path.join(" ");
        if let Some(flags) = app.flag_values.get_mut(&key) {
            for (name, value) in flags.iter_mut() {
                if name == "verbose" {
                    *value = FlagValue::Count(3);
                }
            }
        }
        let output = render_to_string(&mut app, 80, 20);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flag_filter_active() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus on flags and start filtering
        app.set_focus(Focus::Flags);
        app.flag_panel.start_filtering();
        app.flag_panel.set_filter_text("roll");

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flag_filter_verbose() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus on flags and filter for "verb" (should match --verbose)
        app.set_focus(Focus::Flags);
        app.flag_panel.start_filtering();
        app.flag_panel.set_filter_text("verb");

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flag_filter_selected_item() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus on flags and filter for "tag" (matches --tag)
        app.set_focus(Focus::Flags);
        app.flag_panel.start_filtering();
        app.flag_panel.set_filter_text("tag");

        // Move selection to the matching flag (tag is at index 0)
        app.flag_panel.select(0);

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    // ── Theme-specific snapshot tests ───────────────────────────────────

    #[test]
    fn snapshot_nord_theme() {
        let mut app = App::with_theme(sample_spec(), ThemeName::Nord);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_catppuccin_mocha_theme() {
        let mut app = App::with_theme(sample_spec(), ThemeName::CatppuccinMocha);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_tokyo_night_theme() {
        let mut app = App::with_theme(sample_spec(), ThemeName::TokyoNight);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_gruvbox_dark_theme() {
        let mut app = App::with_theme(sample_spec(), ThemeName::GruvboxDark);
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    // ── Assertion-based rendering tests ─────────────────────────────────

    #[test]
    fn test_render_root() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 100, 30);
        assert!(output.contains("mycli"));
        assert!(output.contains("init"));
        assert!(output.contains("config"));
        assert!(output.contains("run"));
        assert!(output.contains("deploy"));
        assert!(output.contains("Command"));
    }

    #[test]
    fn test_render_with_subcommand() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        // Expand config to show children in the tree
        app.command_panel.expand("config");

        let output = render_to_string(&mut app, 100, 30);
        assert!(output.contains("config"));
        assert!(output.contains("set"));
        assert!(output.contains("get"));
        assert!(output.contains("list"));
        assert!(output.contains("remove"));
    }

    #[test]
    fn test_render_leaf_command() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        let output = render_to_string(&mut app, 100, 30);
        assert!(output.contains("Flags"));
        assert!(output.contains("Arguments"));
        assert!(output.contains("environment"));
    }

    #[test]
    fn test_render_flag_display() {
        let flag = usage::SpecFlag {
            name: "verbose".to_string(),
            short: vec!['v'],
            long: vec!["verbose".to_string()],
            ..Default::default()
        };
        assert_eq!(flag_display_string(&flag), "-v, --verbose");

        let flag2 = usage::SpecFlag {
            name: "force".to_string(),
            short: vec!['f'],
            long: vec!["force".to_string()],
            ..Default::default()
        };
        assert_eq!(flag_display_string(&flag2), "-f, --force");

        let flag3 = usage::SpecFlag {
            name: "json".to_string(),
            short: vec![],
            long: vec!["json".to_string()],
            ..Default::default()
        };
        assert_eq!(flag_display_string(&flag3), "--json");
    }

    #[test]
    fn test_render_command_preview_shows_built_command() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        app.arg_values[0].value = "hello".to_string();

        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("mycli init hello"));
    }

    #[test]
    fn test_render_aliases_shown() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        // Expand config to show children with aliases
        app.command_panel.expand("config");

        let output = render_to_string(&mut app, 100, 30);
        // "set" has alias "add", "list" has alias "ls", "remove" has alias "rm"
        assert!(output.contains("add"));
        assert!(output.contains("ls"));
        assert!(output.contains("rm"));
    }

    #[test]
    fn test_render_global_flag_indicator() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 100, 30);
        // Global flags should show [G] indicator
        assert!(output.contains("[G]"));
    }

    #[test]
    fn test_render_required_arg_indicator() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        let output = render_to_string(&mut app, 100, 24);
        // Required args should be shown with <> brackets
        assert!(output.contains("<environment>"));
    }

    #[test]
    fn test_render_choices_display() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        let output = render_to_string(&mut app, 100, 24);
        // Choices should be displayed
        assert!(output.contains("dev"));
        assert!(output.contains("staging"));
        assert!(output.contains("prod"));
    }

    // ── Keyboard interaction rendering tests ────────────────────────────

    #[test]
    fn test_render_after_flag_toggle() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus flags and toggle yes flag
        app.set_focus(Focus::Flags);
        let flag_values = app.current_flag_values();
        let yes_idx = flag_values.iter().position(|(n, _)| n == "yes").unwrap();
        app.set_flag_index(yes_idx);

        // Toggle via space key
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        // After toggling, should show checkmark indicator
        assert!(output.contains("✓"));
        // Preview should include --yes
        assert!(output.contains("--yes"));
    }

    #[test]
    fn test_render_narrow_terminal() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 60, 16);
        // Should still render without panicking
        assert!(output.contains("mycli"));
        assert!(output.contains("Commands"));
    }

    // ── Theme cycling tests ─────────────────────────────────────────────

    #[test]
    fn test_theme_cycling() {
        let mut app = App::new(sample_spec());
        assert_eq!(app.theme_name, ThemeName::Dracula);

        app.next_theme();
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);

        app.next_theme();
        assert_eq!(app.theme_name, ThemeName::Nord);

        app.prev_theme();
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);
    }

    #[test]
    fn test_theme_key_binding() {
        let mut app = App::new(sample_spec());
        assert_eq!(app.theme_name, ThemeName::Dracula);

        // T opens the theme picker instead of cycling
        let key = KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE);
        app.handle_key(key);
        assert!(app.is_theme_picking(), "T should open theme picker");
        assert_eq!(app.theme_name, ThemeName::Dracula, "Theme should not change yet");
    }

    #[test]
    fn test_theme_name_displayed_in_status_bar() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("Dracula"));
    }

    #[test]
    fn test_with_theme_constructor() {
        let app = App::with_theme(sample_spec(), ThemeName::Nord);
        assert_eq!(app.theme_name, ThemeName::Nord);
        assert_eq!(app.palette().accent, ThemeName::Nord.palette().accent);
    }

    // ── Command path visibility tests ───────────────────────────────────

    #[test]
    fn test_binary_name_visible_in_preview() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 100, 24);
        // The binary name should be visible in the command preview
        assert!(output.contains("mycli"));
    }

    #[test]
    fn test_command_path_visible_in_ui() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);

        let output = render_to_string(&mut app, 100, 24);
        // Both mycli (in preview) and config (in tree + preview) should be visible
        assert!(output.contains("mycli"));
        assert!(output.contains("config"));
    }

    // ── Colorized preview tests ─────────────────────────────────────────

    #[test]
    fn test_preview_shows_flags_and_args() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Set flag and arg values
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "tag")
            .unwrap();
        app.set_flag_index(fidx);
        let vals = app.current_flag_values_mut();
        if let Some((_, FlagValue::String(ref mut s))) = vals.get_mut(fidx) {
            *s = "latest".to_string();
        }
        app.arg_values[0].value = "prod".to_string();

        let output = render_to_string(&mut app, 100, 24);
        // Preview should contain the full colorized command
        assert!(output.contains("mycli"));
        assert!(output.contains("deploy"));
        assert!(output.contains("--tag"));
        assert!(output.contains("latest"));
        assert!(output.contains("prod"));
    }

    #[test]
    fn test_preview_focused_shows_run_indicator() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Preview);
        let output = render_to_string(&mut app, 100, 24);
        // Focused preview uses ▶ prefix instead of $
        assert!(output.contains("▶"));
    }

    #[test]
    fn test_preview_unfocused_shows_dollar() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        let output = render_to_string(&mut app, 100, 24);
        // Unfocused preview uses $ prefix
        assert!(output.contains("$ mycli"));
    }

    // ── Panel count display tests ───────────────────────────────────────

    #[test]
    fn test_focused_panel_shows_title() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        app.command_panel.select_item(2);
        app.sync_command_path_from_tree();
        let output = render_to_string(&mut app, 100, 24);
        // Panel headers no longer show counts
        assert!(output.contains("Commands"));
    }

    #[test]
    fn test_unfocused_panel_shows_title() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        let output = render_to_string(&mut app, 100, 24);
        // Unfocused panels just show their name, no counts
        assert!(output.contains("Flags"));
        // Arguments panel only shown when the selected command has args
        // Navigate to init which has args
        app.navigate_to_command(&["init"]);
        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("Arguments"));
    }

    #[test]
    fn test_selection_cursor_visible() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        let output = render_to_string(&mut app, 100, 24);
        // Selected item should have ▶ cursor
        assert!(output.contains("▶"));
    }

    #[test]
    fn test_filter_shows_query_in_title() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        app.set_focus(Focus::Flags);
        app.flag_panel.start_filtering();
        app.flag_panel.set_filter_text("roll");

        let output = render_to_string(&mut app, 100, 24);
        // Should show filter query in title with emoji and query text
        // Note: there appears to be extra spacing after emoji in terminal rendering
        assert!(
            output.contains("Flags 🔍") && output.contains("roll"),
            "Expected 'Flags 🔍' and 'roll' in output"
        );
    }

    #[test]
    fn test_filter_mode_shows_slash_immediately() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        app.command_panel.start_filtering();
        // No text typed yet — filter_input is empty

        let output = render_to_string(&mut app, 100, 24);
        // Title should show the magnifying glass emoji even with an empty query
        assert!(
            output.contains("Commands 🔍"),
            "Panel title should show '🔍' immediately when filter mode is activated, got:\n{output}"
        );
    }

    #[test]
    fn test_filter_mode_shows_slash_in_flags_panel() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);
        app.flag_panel.start_filtering();
        // No text typed yet

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("Flags 🔍"),
            "Flags title should show '🔍' immediately when filter mode is activated, got:\n{output}"
        );
    }

    // ── Unicode checkbox tests ──────────────────────────────────────────

    #[test]
    fn test_checked_flag_shows_checked_box() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Toggle rollback flag on
        app.set_focus(Focus::Flags);
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "rollback")
            .unwrap();
        app.set_flag_index(fidx);
        let vals = app.current_flag_values_mut();
        if let Some((_, FlagValue::Bool(ref mut b))) = vals.get_mut(fidx) {
            *b = true;
        }

        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("✓"));
    }

    #[test]
    fn test_unchecked_flag_shows_empty_box() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        let output = render_to_string(&mut app, 100, 24);
        // Unchecked boolean flags should show ○
        assert!(output.contains("○"));
    }

    // ── Theme visual consistency tests ──────────────────────────────────

    #[test]
    fn test_all_themes_render_without_panic() {
        for theme in ThemeName::all() {
            let mut app = App::with_theme(sample_spec(), *theme);
            app.navigate_to_command(&["deploy"]);

            // Should render without panicking for any theme
            let output = render_to_string(&mut app, 100, 24);
            assert!(
                output.contains("mycli"),
                "Theme {:?} failed to render binary name",
                theme
            );
            assert!(
                output.contains("deploy"),
                "Theme {:?} failed to render subcommand",
                theme
            );
            assert!(
                output.contains(theme.display_name()),
                "Theme {:?} name not shown in status bar",
                theme
            );
        }
    }

    #[test]
    fn test_light_theme_renders() {
        // Test a light theme to ensure we handle light backgrounds
        let mut app = App::with_theme(sample_spec(), ThemeName::CatppuccinLatte);
        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("mycli"));
        assert!(output.contains("Catppuccin Latte"));
    }

    #[test]
    fn test_ui_colors_from_palette_consistency() {
        // Verify UiColors derives correctly from different palettes
        let dracula_palette = ThemeName::Dracula.palette();
        let nord_palette = ThemeName::Nord.palette();

        let dracula_colors = super::UiColors::from_palette(&dracula_palette);
        let nord_colors = super::UiColors::from_palette(&nord_palette);

        // command uses info color, which should differ between themes
        assert_eq!(dracula_colors.command, dracula_palette.info);
        assert_eq!(nord_colors.command, nord_palette.info);

        // flag uses warning color
        assert_eq!(dracula_colors.flag, dracula_palette.warning);
        assert_eq!(nord_colors.flag, nord_palette.warning);

        // required uses error color
        assert_eq!(dracula_colors.required, dracula_palette.error);
        assert_eq!(nord_colors.required, nord_palette.error);
    }

    // ── Phase 10: UX Bug Fix Tests ──────────────────────────────────────

    #[test]
    fn test_checkmark_style_checked() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Toggle --rollback on
        app.set_focus(Focus::Flags);
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "rollback")
            .unwrap();
        app.set_flag_index(fidx);
        let vals = app.current_flag_values_mut();
        if let Some((_, FlagValue::Bool(ref mut b))) = vals.get_mut(fidx) {
            *b = true;
        }

        let output = render_to_string(&mut app, 100, 24);
        // Checked flags should use checkmark ✓ (not ☑)
        assert!(
            output.contains('✓'),
            "Checked flag should show ✓ checkmark symbol"
        );
        assert!(
            !output.contains('☑'),
            "Should NOT use small ☑ symbol anymore"
        );
    }

    #[test]
    fn test_checkmark_style_unchecked() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        let output = render_to_string(&mut app, 100, 24);
        // Unchecked flags should use ○ (not ☐)
        assert!(
            output.contains('○'),
            "Unchecked flag should show ○ circle symbol"
        );
        assert!(
            !output.contains('☐'),
            "Should NOT use small ☐ symbol anymore"
        );
    }

    #[test]
    fn test_prominent_selection_caret() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        let output = render_to_string(&mut app, 100, 24);
        // Selected item should have prominent ▶ cursor (not thin ▸)
        assert!(
            output.contains('▶'),
            "Selection caret should be the prominent ▶ triangle"
        );
        // The old thin ▸ should only appear as a subcommand indicator, not as a selection caret
        // (subcommand indicator "▸" is still used for has-children indicator)
    }

    #[test]
    fn test_prominent_caret_in_flags_panel() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains('▶'),
            "Flags panel should use prominent ▶ caret for selected item"
        );
    }

    #[test]
    fn test_prominent_caret_in_args_panel() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        app.set_focus(Focus::Args);

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains('▶'),
            "Args panel should use prominent ▶ caret for selected item"
        );
    }

    #[test]
    fn test_count_flag_decrement_renders() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Flags);

        // Find verbose (count flag)
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "verbose")
            .unwrap();
        app.set_flag_index(fidx);

        // Increment to 3
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        }

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("[3]"),
            "Count flag at 3 should show [3] in the UI"
        );

        // Decrement via backspace
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("[2]"),
            "After backspace, count flag should show [2]"
        );

        // Decrement to zero
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("[0]"), "Count flag at 0 should show [0]");

        // One more backspace — should stay at 0
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("[0]"), "Count flag should not go below 0");
    }

    #[test]
    fn test_editing_arg_then_switching_does_not_bleed() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        // Focus on args and edit <task>
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();
        // Type via handle_key which calls sync_edit_to_value internally
        for c in ['h', 'e', 'l', 'l', 'o'] {
            app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }

        // Render while editing first arg
        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("hello"),
            "Editing first arg should show 'hello'"
        );

        // Finish editing and switch to second arg
        app.finish_editing();
        app.set_arg_index(1);

        // Render: second arg should NOT show 'hello'
        let output = render_to_string(&mut app, 100, 24);
        // The second arg row should show (empty) not 'hello'
        // Split output into lines, find the line with [args...]
        let args_line = output.lines().find(|l| l.contains("args")).unwrap_or("");
        assert!(
            !args_line.contains("hello"),
            "Second arg should not contain text from first arg. Line: {}",
            args_line
        );
    }

    #[test]
    fn test_count_flag_preview_after_decrement() {
        let mut app = App::new(sample_spec());
        // Stay at root where verbose flag is
        app.set_focus(Focus::Flags);

        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "verbose")
            .unwrap();
        app.set_flag_index(fidx);

        // Increment to 2
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("-vv"),
            "Preview should show -vv for count 2"
        );

        // Decrement to 1
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("-v") && !output.contains("-vv"),
            "Preview should show -v for count 1"
        );

        // Decrement to 0
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        assert!(
            !output.contains("-v ") && !output.contains("-vv"),
            "Preview should not contain -v when count is 0"
        );
    }

    #[test]
    fn test_cursor_position_in_editing() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();

        // Type "hello" via handle_key
        for ch in "hello".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        // Render with cursor at end
        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("hello▎"),
            "Should show cursor at end of text"
        );

        // Move cursor to beginning with Home key
        app.handle_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("▎hello"),
            "Should show cursor at beginning of text"
        );

        // Move cursor right twice
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE));
        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("he▎llo"),
            "Should show cursor in middle of text"
        );
    }

    // ── Theme picker rendering tests ────────────────────────────────────

    #[test]
    fn snapshot_theme_picker_open() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_theme_picker_navigated() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        // Navigate down to Nord (index 2)
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(down);
        app.handle_key(down);

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn test_theme_picker_shows_all_themes() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();
        let output = render_to_string(&mut app, 100, 24);

        // All theme display names should appear in the overlay
        for theme in ThemeName::all() {
            assert!(
                output.contains(theme.display_name()),
                "Theme picker should show '{}' but output was:\n{}",
                theme.display_name(),
                output
            );
        }
    }

    #[test]
    fn test_theme_picker_help_bar_shows_picker_keybinds() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();
        let output = render_to_string(&mut app, 100, 24);

        assert!(
            output.contains("⏎ confirm") && output.contains("Esc cancel"),
            "Help bar should show theme picker keybinds"
        );
    }

    #[test]
    fn test_theme_picker_previews_theme() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        // Navigate to Nord
        let down = KeyEvent::new(KeyCode::Down, KeyModifiers::NONE);
        app.handle_key(down);
        app.handle_key(down);

        let output = render_to_string(&mut app, 100, 24);
        // The help bar should show [Nord] since theme_name changed to preview
        assert!(
            output.contains("[Nord]"),
            "Theme indicator should show the previewed theme"
        );
    }

    #[test]
    fn test_theme_indicator_rect_set_after_render() {
        let mut app = App::new(sample_spec());
        assert!(app.layout.theme_indicator_rect.is_none());
        let _output = render_to_string(&mut app, 100, 24);
        assert!(
            app.layout.theme_indicator_rect.is_some(),
            "theme_indicator_rect should be set after rendering"
        );
    }

    #[test]
    fn snapshot_commands_scrollbar() {
        // Use sample spec which has 15 commands (when flat)
        // Render in a small height (12 lines) to force scrolling
        let mut app = App::new(sample_spec());
        // Navigate down to show scrollbar thumb moved from top
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        for _ in 0..7 {
            app.handle_key(down);
        }
        let output = render_to_string(&mut app, 100, 12);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_select_scrollbar() {
        // Create a spec with many static choices to test scrollbar
        let spec_text = r#"
name "test"
bin "test"
cmd "test" {
    arg "<item>" {
        choices "opt01" "opt02" "opt03" "opt04" "opt05" "opt06" "opt07" "opt08" "opt09" "opt10" "opt11" "opt12" "opt13" "opt14" "opt15"
    }
}
        "#;
        let mut app = App::new(parse_spec(spec_text));

        // Focus on Args and open the select box
        app.set_focus(Focus::Args);
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        // Navigate down in the select to show scrollbar thumb moved from top
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        for _ in 0..5 {
            app.handle_key(down);
        }
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }
}
