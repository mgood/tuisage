use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Padding, Paragraph, Wrap},
    Frame,
};

use crate::app::{App, FlagValue, Focus};

/// Color palette for the TUI.
mod colors {
    use ratatui::style::Color;

    pub const COMMAND: Color = Color::Cyan;
    pub const FLAG: Color = Color::Yellow;
    pub const ARG: Color = Color::Green;
    pub const VALUE: Color = Color::Magenta;
    pub const REQUIRED: Color = Color::Red;
    pub const HELP: Color = Color::DarkGray;
    pub const ACTIVE_BORDER: Color = Color::Cyan;
    pub const INACTIVE_BORDER: Color = Color::DarkGray;
    pub const SELECTED_BG: Color = Color::Rgb(40, 40, 60);
    pub const EDITING_BG: Color = Color::Rgb(50, 30, 30);
    pub const PREVIEW_CMD: Color = Color::White;
    pub const BREADCRUMB: Color = Color::Cyan;
    pub const FILTER: Color = Color::Yellow;
    pub const CHOICE: Color = Color::Blue;
    pub const DEFAULT_VAL: Color = Color::DarkGray;
    pub const COUNT: Color = Color::Magenta;
}

/// Main render function called from the event loop.
pub fn render(frame: &mut Frame, app: &App) {
    let area = frame.area();

    // Top-level vertical layout:
    //   [breadcrumb bar]
    //   [main content area]
    //   [help / status bar]
    //   [command preview]
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // breadcrumb
            Constraint::Min(6),    // main content
            Constraint::Length(2), // help text
            Constraint::Length(3), // command preview
        ])
        .split(area);

    render_breadcrumb(frame, app, outer[0]);
    render_main_content(frame, app, outer[1]);
    render_help_bar(frame, app, outer[2]);
    render_preview(frame, app, outer[3]);
}

/// Render the breadcrumb bar showing the current command path.
fn render_breadcrumb(frame: &mut Frame, app: &App, area: Rect) {
    let mut spans = vec![Span::styled(" ", Style::default())];

    let bin = if app.spec.bin.is_empty() {
        &app.spec.name
    } else {
        &app.spec.bin
    };

    spans.push(Span::styled(
        bin,
        Style::default()
            .fg(colors::BREADCRUMB)
            .add_modifier(Modifier::BOLD),
    ));

    for name in &app.command_path {
        spans.push(Span::styled(" > ", Style::default().fg(colors::HELP)));
        spans.push(Span::styled(
            name.as_str(),
            Style::default()
                .fg(colors::BREADCRUMB)
                .add_modifier(Modifier::BOLD),
        ));
    }

    // Show filter if active
    if app.filtering {
        spans.push(Span::styled("  /", Style::default().fg(colors::FILTER)));
        spans.push(Span::styled(
            &app.filter,
            Style::default()
                .fg(colors::FILTER)
                .add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(
            "▎",
            Style::default()
                .fg(colors::FILTER)
                .add_modifier(Modifier::SLOW_BLINK),
        ));
    }

    let line = Line::from(spans);
    let paragraph = Paragraph::new(line).style(Style::default().bg(Color::Rgb(30, 30, 40)));
    frame.render_widget(paragraph, area);
}

/// Render the main content area with panels for commands, flags, and args.
fn render_main_content(frame: &mut Frame, app: &App, area: Rect) {
    let has_commands = !app.visible_subcommands().is_empty();
    let has_flags = !app.visible_flags().is_empty();
    let has_args = !app.arg_values.is_empty();

    // Decide layout based on what's available
    match (has_commands, has_flags || has_args) {
        (true, true) => {
            // Split horizontally: commands on left, flags+args on right
            let h_split = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(area);

            render_command_list(frame, app, h_split[0]);

            if has_flags && has_args {
                let v_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .split(h_split[1]);
                render_flag_list(frame, app, v_split[0]);
                render_arg_list(frame, app, v_split[1]);
            } else if has_flags {
                render_flag_list(frame, app, h_split[1]);
            } else {
                render_arg_list(frame, app, h_split[1]);
            }
        }
        (true, false) => {
            render_command_list(frame, app, area);
        }
        (false, true) => {
            if has_flags && has_args {
                let v_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .split(area);
                render_flag_list(frame, app, v_split[0]);
                render_arg_list(frame, app, v_split[1]);
            } else if has_flags {
                render_flag_list(frame, app, area);
            } else {
                render_arg_list(frame, app, area);
            }
        }
        (false, false) => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors::INACTIVE_BORDER))
                .title(" No options ")
                .padding(Padding::horizontal(1));
            let inner = Paragraph::new("This command has no subcommands, flags, or arguments.")
                .style(Style::default().fg(colors::HELP))
                .wrap(Wrap { trim: true })
                .block(block);
            frame.render_widget(inner, area);
        }
    }
}

/// Render the subcommand list panel.
fn render_command_list(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == Focus::Commands;
    let border_color = if is_focused {
        colors::ACTIVE_BORDER
    } else {
        colors::INACTIVE_BORDER
    };

    let title = if app.filtering && is_focused {
        format!(" Commands (/{}) ", app.filter)
    } else {
        " Commands ".to_string()
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    let subs = app.visible_subcommands();
    let items: Vec<ListItem> = subs
        .iter()
        .enumerate()
        .map(|(i, (name, cmd))| {
            let is_selected = is_focused && i == app.command_index;
            let has_children = !cmd.subcommands.is_empty();

            let mut spans = Vec::new();

            // Command name
            let name_style = if is_selected {
                Style::default()
                    .fg(colors::COMMAND)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::COMMAND)
            };
            spans.push(Span::styled(name.as_str(), name_style));

            // Subcommand indicator
            if has_children {
                spans.push(Span::styled(" ▸", Style::default().fg(colors::HELP)));
            }

            // Aliases
            if !cmd.aliases.is_empty() {
                let aliases = cmd.aliases.join(", ");
                spans.push(Span::styled(
                    format!(" ({})", aliases),
                    Style::default().fg(colors::HELP),
                ));
            }

            // Help text (truncated to fit)
            if let Some(help) = &cmd.help {
                spans.push(Span::styled(" — ", Style::default().fg(colors::HELP)));
                spans.push(Span::styled(
                    help.as_str(),
                    Style::default().fg(colors::HELP),
                ));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                item = item.style(Style::default().bg(colors::SELECTED_BG));
            }
            item
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the flag list panel.
fn render_flag_list(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == Focus::Flags;
    let border_color = if is_focused {
        colors::ACTIVE_BORDER
    } else {
        colors::INACTIVE_BORDER
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Flags ")
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    let flags = app.visible_flags();
    let flag_values = app.current_flag_values();

    let items: Vec<ListItem> = flags
        .iter()
        .enumerate()
        .map(|(i, flag)| {
            let is_selected = is_focused && i == app.flag_index;
            let is_editing = is_selected && app.editing;
            let value = flag_values.iter().find(|(n, _)| n == &flag.name);

            let mut spans = Vec::new();

            // Checkbox / toggle indicator
            let indicator = match value.map(|(_, v)| v) {
                Some(FlagValue::Bool(true)) => {
                    Span::styled("[✓] ", Style::default().fg(Color::Green))
                }
                Some(FlagValue::Bool(false)) => {
                    Span::styled("[ ] ", Style::default().fg(colors::HELP))
                }
                Some(FlagValue::Count(n)) => {
                    let display = if *n > 0 {
                        Span::styled(format!("[{}] ", n), Style::default().fg(colors::COUNT))
                    } else {
                        Span::styled("[0] ", Style::default().fg(colors::HELP))
                    };
                    display
                }
                Some(FlagValue::String(s)) => {
                    if s.is_empty() {
                        Span::styled("[·] ", Style::default().fg(colors::HELP))
                    } else {
                        Span::styled("[•] ", Style::default().fg(Color::Green))
                    }
                }
                None => Span::styled("[ ] ", Style::default().fg(colors::HELP)),
            };
            spans.push(indicator);

            // Flag display (short + long)
            let flag_display = flag_display_string(flag);
            let flag_style = if is_selected {
                Style::default()
                    .fg(colors::FLAG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::FLAG)
            };
            spans.push(Span::styled(flag_display, flag_style));

            // Global indicator
            if flag.global {
                spans.push(Span::styled(" [G]", Style::default().fg(colors::HELP)));
            }

            // Required indicator
            if flag.required {
                spans.push(Span::styled(" *", Style::default().fg(colors::REQUIRED)));
            }

            // Value display
            if let Some((_, val)) = value {
                match val {
                    FlagValue::String(s) => {
                        spans.push(Span::styled(" = ", Style::default().fg(colors::HELP)));

                        if is_editing {
                            spans.push(Span::styled(
                                s.as_str(),
                                Style::default()
                                    .fg(colors::VALUE)
                                    .add_modifier(Modifier::UNDERLINED),
                            ));
                            spans.push(Span::styled(
                                "▎",
                                Style::default()
                                    .fg(colors::VALUE)
                                    .add_modifier(Modifier::SLOW_BLINK),
                            ));
                        } else if s.is_empty() {
                            // Show choices hint or default
                            if let Some(ref arg) = flag.arg {
                                if let Some(ref choices) = arg.choices {
                                    let hint = choices.choices.join("|");
                                    spans.push(Span::styled(
                                        format!("<{}>", hint),
                                        Style::default().fg(colors::CHOICE),
                                    ));
                                } else {
                                    spans.push(Span::styled(
                                        format!("<{}>", arg.name),
                                        Style::default().fg(colors::DEFAULT_VAL),
                                    ));
                                }
                            }
                        } else {
                            spans
                                .push(Span::styled(s.as_str(), Style::default().fg(colors::VALUE)));
                        }
                    }
                    _ => {} // Bool and Count already shown via indicator
                }
            }

            // Help text
            if let Some(help) = &flag.help {
                spans.push(Span::styled(" — ", Style::default().fg(colors::HELP)));
                spans.push(Span::styled(
                    help.as_str(),
                    Style::default().fg(colors::HELP),
                ));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                let bg = if is_editing {
                    colors::EDITING_BG
                } else {
                    colors::SELECTED_BG
                };
                item = item.style(Style::default().bg(bg));
            }
            item
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the positional argument list panel.
fn render_arg_list(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == Focus::Args;
    let border_color = if is_focused {
        colors::ACTIVE_BORDER
    } else {
        colors::INACTIVE_BORDER
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Arguments ")
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    let items: Vec<ListItem> = app
        .arg_values
        .iter()
        .enumerate()
        .map(|(i, arg_val)| {
            let is_selected = is_focused && i == app.arg_index;
            let is_editing = is_selected && app.editing;

            let mut spans = Vec::new();

            // Required/optional indicator
            if arg_val.required {
                spans.push(Span::styled("● ", Style::default().fg(colors::REQUIRED)));
            } else {
                spans.push(Span::styled("○ ", Style::default().fg(colors::HELP)));
            }

            // Arg name
            let name_style = if is_selected {
                Style::default()
                    .fg(colors::ARG)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors::ARG)
            };
            let bracket = if arg_val.required { "<>" } else { "[]" };
            spans.push(Span::styled(
                format!("{}{}{}", &bracket[..1], arg_val.name, &bracket[1..]),
                name_style,
            ));

            // Value display
            spans.push(Span::styled(" = ", Style::default().fg(colors::HELP)));

            if is_editing {
                spans.push(Span::styled(
                    &arg_val.value,
                    Style::default()
                        .fg(colors::VALUE)
                        .add_modifier(Modifier::UNDERLINED),
                ));
                spans.push(Span::styled(
                    "▎",
                    Style::default()
                        .fg(colors::VALUE)
                        .add_modifier(Modifier::SLOW_BLINK),
                ));
            } else if arg_val.value.is_empty() {
                if !arg_val.choices.is_empty() {
                    let hint = arg_val.choices.join("|");
                    spans.push(Span::styled(
                        format!("<{}>", hint),
                        Style::default().fg(colors::CHOICE),
                    ));
                } else {
                    spans.push(Span::styled(
                        "(empty)",
                        Style::default().fg(colors::DEFAULT_VAL),
                    ));
                }
            } else {
                spans.push(Span::styled(
                    &arg_val.value,
                    Style::default().fg(colors::VALUE),
                ));
            }

            // Show choices if arg has them and we're not editing
            if !arg_val.choices.is_empty() && !arg_val.value.is_empty() && !is_editing {
                spans.push(Span::styled(
                    format!(" [{}]", arg_val.choices.join("|")),
                    Style::default().fg(colors::CHOICE),
                ));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                let bg = if is_editing {
                    colors::EDITING_BG
                } else {
                    colors::SELECTED_BG
                };
                item = item.style(Style::default().bg(bg));
            }
            item
        })
        .collect();

    let list = List::new(items).block(block);
    frame.render_widget(list, area);
}

/// Render the help/status bar.
fn render_help_bar(frame: &mut Frame, app: &App, area: Rect) {
    let help_text = app.current_help().unwrap_or_default();

    let keybinds = if app.editing {
        "Enter: confirm  Esc: cancel"
    } else if app.filtering {
        "Enter: apply  Esc: clear  ↑↓: navigate"
    } else {
        match app.focus {
            Focus::Commands => {
                "Enter/→: select  ↑↓: navigate  Tab: next panel  /: filter  Esc: back  q: quit"
            }
            Focus::Flags => {
                "Enter/Space: toggle  ↑↓: navigate  Tab: next panel  Esc: back  q: quit"
            }
            Focus::Args => "Enter: edit  ↑↓: navigate  Tab: next panel  Esc: back  q: quit",
            Focus::Preview => "Enter: accept command  Tab: next panel  Esc: back  q: quit",
        }
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Help text for current item
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" 💡 ", Style::default()),
        Span::styled(help_text, Style::default().fg(colors::HELP).italic()),
    ]));
    frame.render_widget(help, layout[0]);

    // Keybinding hints
    let hints = Paragraph::new(Line::from(vec![Span::styled(
        format!(" {keybinds}"),
        Style::default().fg(Color::DarkGray),
    )]))
    .style(Style::default().bg(Color::Rgb(25, 25, 35)));
    frame.render_widget(hints, layout[1]);
}

/// Render the command preview bar at the bottom.
fn render_preview(frame: &mut Frame, app: &App, area: Rect) {
    let is_focused = app.focus == Focus::Preview;
    let border_color = if is_focused {
        colors::ACTIVE_BORDER
    } else {
        colors::INACTIVE_BORDER
    };

    let command = app.build_command();

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Command ")
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    let style = if is_focused {
        Style::default()
            .fg(colors::PREVIEW_CMD)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(colors::PREVIEW_CMD)
    };

    let prefix = if is_focused { "▶ " } else { "$ " };

    let paragraph = Paragraph::new(Line::from(vec![
        Span::styled(prefix, Style::default().fg(colors::COMMAND)),
        Span::styled(command, style),
    ]))
    .block(block)
    .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Format a flag's display string (e.g., "-f, --force" or "--verbose").
fn flag_display_string(flag: &usage::SpecFlag) -> String {
    let mut parts = Vec::new();
    for s in &flag.short {
        parts.push(format!("-{s}"));
    }
    for l in &flag.long {
        parts.push(format!("--{l}"));
    }
    if parts.is_empty() {
        flag.name.clone()
    } else {
        parts.join(", ")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{backend::TestBackend, Terminal};

    fn sample_spec() -> usage::Spec {
        let input = include_str!("../fixtures/sample.usage.kdl");
        input
            .parse::<usage::Spec>()
            .expect("Failed to parse sample spec")
    }

    fn render_to_string(app: &App, width: u16, height: u16) -> String {
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
            output.push('\n');
        }
        output
    }

    #[test]
    fn test_render_root() {
        let app = App::new(sample_spec());
        let output = render_to_string(&app, 100, 30);
        // Should show breadcrumb
        assert!(output.contains("mycli"));
        // Should show subcommands
        assert!(output.contains("init"));
        assert!(output.contains("config"));
        assert!(output.contains("run"));
        assert!(output.contains("deploy"));
        // Should show command preview
        assert!(output.contains("Command"));
    }

    #[test]
    fn test_render_with_subcommand() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let config_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.command_index = config_idx;
        app.navigate_into_selected();

        let output = render_to_string(&app, 100, 30);
        // Should show breadcrumb with config
        assert!(output.contains("config"));
        // Should show config's subcommands
        assert!(output.contains("set"));
        assert!(output.contains("get"));
        assert!(output.contains("list"));
        assert!(output.contains("remove"));
    }

    #[test]
    fn test_render_leaf_command() {
        let mut app = App::new(sample_spec());
        // Navigate to deploy
        let subs = app.visible_subcommands();
        let deploy_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.command_index = deploy_idx;
        app.navigate_into_selected();

        let output = render_to_string(&app, 100, 30);
        // Should show flags
        assert!(output.contains("Flags"));
        // Should show arguments
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
}
