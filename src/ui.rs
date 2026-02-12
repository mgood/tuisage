use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame,
};
use ratatui_interact::components::{
    Breadcrumb, BreadcrumbItem, BreadcrumbState, BreadcrumbStyle, Input, InputStyle,
};
use ratatui_themes::ThemePalette;

#[cfg(test)]
extern crate insta;

use crate::app::{App, FlagValue, Focus};

/// Derive a semantic color palette for our UI elements from the theme palette.
/// This maps semantic theme colors to our specific UI roles.
struct UiColors {
    command: Color,
    flag: Color,
    arg: Color,
    value: Color,
    required: Color,
    help: Color,
    active_border: Color,
    inactive_border: Color,
    selected_bg: Color,
    editing_bg: Color,
    preview_cmd: Color,
    breadcrumb: Color,
    filter: Color,
    choice: Color,
    default_val: Color,
    count: Color,
    bg: Color,
    bar_bg: Color,
}

impl UiColors {
    fn from_palette(p: &ThemePalette) -> Self {
        // Derive a slightly lighter/darker shade for bar backgrounds
        let bar_bg = match p.bg {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(10),
                g.saturating_add(10),
                b.saturating_add(15),
            ),
            _ => Color::Rgb(30, 30, 40),
        };

        // Derive selection background from the theme selection color
        let selected_bg = match p.selection {
            Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
            _ => Color::Rgb(40, 40, 60),
        };

        // Derive editing background: a warm-tinted version of selection
        let editing_bg = match p.selection {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(15),
                g.saturating_sub(5),
                b.saturating_sub(10),
            ),
            _ => Color::Rgb(50, 30, 30),
        };

        Self {
            command: p.info,   // Commands are informational navigation targets
            flag: p.warning,   // Flags draw attention like warnings
            arg: p.success,    // Arguments are green/positive inputs
            value: p.accent,   // Values are the primary highlighted data
            required: p.error, // Required indicators are critical/red
            help: p.muted,     // Help text is secondary/muted
            active_border: p.accent,
            inactive_border: p.muted,
            selected_bg,
            editing_bg,
            preview_cmd: p.fg,
            breadcrumb: p.accent,
            filter: p.warning,
            choice: p.info,
            default_val: p.muted,
            count: p.secondary,
            bg: p.bg,
            bar_bg,
        }
    }
}

/// Main render function called from the event loop.
pub fn render(frame: &mut Frame, app: &mut App) {
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

    // Clear click regions each frame so they're rebuilt from current layout
    app.click_regions.clear();

    let palette = app.palette();
    let colors = UiColors::from_palette(&palette);

    render_breadcrumb(frame, app, outer[0], &colors);
    render_main_content(frame, app, outer[1], &colors);
    render_help_bar(frame, app, outer[2], &colors);
    render_preview(frame, app, outer[3], &colors);

    // Register preview area for click hit-testing
    app.click_regions.register(outer[3], Focus::Preview);
}

/// Render the breadcrumb bar using the ratatui-interact Breadcrumb widget.
fn render_breadcrumb(frame: &mut Frame, app: &App, area: Rect, colors: &UiColors) {
    let bin = if app.spec.bin.is_empty() {
        &app.spec.name
    } else {
        &app.spec.bin
    };

    // Build breadcrumb items: root binary + command path
    let mut items = vec![BreadcrumbItem::new("root", bin)];
    for name in &app.command_path {
        items.push(BreadcrumbItem::new(name.as_str(), name.as_str()));
    }

    let bc_state = BreadcrumbState::new(items);

    // Style the breadcrumb with theme colors
    let bc_style = BreadcrumbStyle::chevron()
        .item_style(
            Style::default()
                .fg(colors.breadcrumb)
                .add_modifier(Modifier::BOLD),
        )
        .last_item_style(
            Style::default()
                .fg(colors.breadcrumb)
                .add_modifier(Modifier::BOLD),
        )
        .separator_style(Style::default().fg(colors.help))
        .padding(1, 0);

    let breadcrumb = Breadcrumb::new(&bc_state).style(bc_style);

    // If filtering is active, we split the breadcrumb area to show filter input
    if app.filtering {
        let bc_width = breadcrumb
            .calculate_width()
            .min(area.width.saturating_sub(20));
        let filter_width = area.width.saturating_sub(bc_width).saturating_sub(3);

        let bc_area = Rect::new(area.x, area.y, bc_width, area.height);
        let separator_area = Rect::new(area.x + bc_width, area.y, 3, area.height);
        let filter_area = Rect::new(area.x + bc_width + 3, area.y, filter_width, area.height);

        // Render background
        let bg = Paragraph::new("").style(Style::default().bg(colors.bar_bg));
        frame.render_widget(bg, area);

        // Render breadcrumb
        breadcrumb.render_stateful(bc_area, frame.buffer_mut());

        // Render filter separator
        let sep = Paragraph::new(Span::styled("  /", Style::default().fg(colors.filter)));
        frame.render_widget(sep, separator_area);

        // Render filter input using ratatui-interact Input widget
        let input_style = InputStyle::default()
            .text_fg(colors.filter)
            .cursor_fg(colors.filter)
            .placeholder_fg(colors.help);
        let input = Input::new(&app.filter_input)
            .style(input_style)
            .with_border(false)
            .placeholder("type to filter...");
        input.render_stateful(frame, filter_area);
    } else {
        // Render background
        let bg = Paragraph::new("").style(Style::default().bg(colors.bar_bg));
        frame.render_widget(bg, area);

        // Render breadcrumb only
        breadcrumb.render_stateful(area, frame.buffer_mut());
    }
}

/// Render the main content area with panels for commands, flags, and args.
fn render_main_content(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
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

            render_command_list(frame, app, h_split[0], colors);

            if has_flags && has_args {
                let v_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .split(h_split[1]);
                render_flag_list(frame, app, v_split[0], colors);
                render_arg_list(frame, app, v_split[1], colors);
            } else if has_flags {
                render_flag_list(frame, app, h_split[1], colors);
            } else {
                render_arg_list(frame, app, h_split[1], colors);
            }
        }
        (true, false) => {
            render_command_list(frame, app, area, colors);
        }
        (false, true) => {
            if has_flags && has_args {
                let v_split = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
                    .split(area);
                render_flag_list(frame, app, v_split[0], colors);
                render_arg_list(frame, app, v_split[1], colors);
            } else if has_flags {
                render_flag_list(frame, app, area, colors);
            } else {
                render_arg_list(frame, app, area, colors);
            }
        }
        (false, false) => {
            let block = Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(colors.inactive_border))
                .title(" No options ")
                .padding(Padding::horizontal(1));
            let inner = Paragraph::new("This command has no subcommands, flags, or arguments.")
                .style(Style::default().fg(colors.help).bg(colors.bg))
                .wrap(Wrap { trim: true })
                .block(block);
            frame.render_widget(inner, area);
        }
    }
}

/// Render the subcommand list panel.
fn render_command_list(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Register area for click hit-testing
    app.click_regions.register(area, Focus::Commands);

    let is_focused = app.focus() == Focus::Commands;
    let border_color = if is_focused {
        colors.active_border
    } else {
        colors.inactive_border
    };

    // Compute counts for title before mutable borrow
    let total = app.visible_subcommands().len();
    let command_index = app.command_index();

    let title = if app.filtering && app.focus() == Focus::Commands {
        format!(
            " Commands (/{}) [{}/{}] ",
            app.filter(),
            command_index + 1,
            total
        )
    } else if total > 0 && is_focused {
        format!(" Commands [{}/{}] ", command_index + 1, total)
    } else {
        format!(" Commands ({}) ", total)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    // Calculate inner height for scroll offset (area minus borders)
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Commands, inner_height);

    // Re-fetch subcommands after mutable borrow ends
    let subs = app.visible_subcommands();
    let items: Vec<ListItem> = subs
        .iter()
        .enumerate()
        .map(|(i, (name, cmd))| {
            let is_selected = is_focused && i == command_index;
            let has_children = !cmd.subcommands.is_empty();

            let mut spans = Vec::new();

            // Selection cursor indicator
            if is_selected {
                spans.push(Span::styled(
                    "▸ ",
                    Style::default().fg(colors.active_border),
                ));
            } else {
                spans.push(Span::styled("  ", Style::default()));
            }

            // Command name
            let name_style = if is_selected {
                Style::default()
                    .fg(colors.command)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.command)
            };
            spans.push(Span::styled(name.as_str(), name_style));

            // Subcommand indicator
            if has_children {
                spans.push(Span::styled(" ▸", Style::default().fg(colors.help)));
            }

            // Aliases
            if !cmd.aliases.is_empty() {
                let aliases = cmd.aliases.join(", ");
                spans.push(Span::styled(
                    format!(" ({})", aliases),
                    Style::default().fg(colors.help),
                ));
            }

            // Help text (truncated to fit)
            if let Some(help) = &cmd.help {
                spans.push(Span::styled(" — ", Style::default().fg(colors.help)));
                spans.push(Span::styled(
                    help.as_str(),
                    Style::default().fg(colors.help),
                ));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                item = item.style(Style::default().bg(colors.selected_bg));
            }
            item
        })
        .collect();

    let mut state = ListState::default()
        .with_selected(if is_focused {
            Some(command_index)
        } else {
            None
        })
        .with_offset(app.command_scroll());
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut state);
}

/// Render the flag list panel.
fn render_flag_list(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Register area for click hit-testing
    app.click_regions.register(area, Focus::Flags);

    let is_focused = app.focus() == Focus::Flags;
    let border_color = if is_focused {
        colors.active_border
    } else {
        colors.inactive_border
    };

    // Compute counts for title before mutable borrow
    let flag_index = app.flag_index();
    let visible_count = app.visible_flags().len();
    let total_count = app.current_flag_values().len();

    let title = if app.filtering && app.focus() == Focus::Flags {
        format!(
            " Flags (/{}) [{}/{}] ",
            app.filter(),
            flag_index + 1,
            visible_count
        )
    } else if total_count > 0 && is_focused {
        format!(" Flags [{}/{}] ", flag_index + 1, total_count)
    } else {
        format!(" Flags ({}) ", total_count)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    // Calculate inner height for scroll offset
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Flags, inner_height);

    // Re-fetch data after mutable borrow ends
    let flags = app.visible_flags();
    let flag_values = app.current_flag_values();

    // Pre-compute default values for each flag so we can show "(default)" indicator
    let flag_defaults: Vec<Option<String>> =
        flags.iter().map(|f| f.default.first().cloned()).collect();

    let items: Vec<ListItem> = flags
        .iter()
        .enumerate()
        .map(|(i, flag)| {
            let is_selected = is_focused && i == flag_index;
            let is_editing = is_selected && app.editing;
            let value = flag_values.iter().find(|(n, _)| n == &flag.name);
            let default_val = flag_defaults.get(i).and_then(|d| d.as_ref());

            let mut spans = Vec::new();

            // Selection cursor indicator
            if is_selected {
                spans.push(Span::styled(
                    "▸ ",
                    Style::default().fg(colors.active_border),
                ));
            } else {
                spans.push(Span::styled("  ", Style::default()));
            }

            // Checkbox / toggle indicator using Unicode checkboxes
            let indicator = match value.map(|(_, v)| v) {
                Some(FlagValue::Bool(true)) => Span::styled("☑ ", Style::default().fg(colors.arg)),
                Some(FlagValue::Bool(false)) => {
                    Span::styled("☐ ", Style::default().fg(colors.help))
                }
                Some(FlagValue::Count(n)) => {
                    if *n > 0 {
                        Span::styled(format!("[{}] ", n), Style::default().fg(colors.count))
                    } else {
                        Span::styled("[0] ", Style::default().fg(colors.help))
                    }
                }
                Some(FlagValue::String(s)) => {
                    if s.is_empty() {
                        Span::styled("[·] ", Style::default().fg(colors.help))
                    } else {
                        Span::styled("[•] ", Style::default().fg(colors.arg))
                    }
                }
                None => Span::styled("☐ ", Style::default().fg(colors.help)),
            };
            spans.push(indicator);

            // Flag display (short + long)
            let flag_display = flag_display_string(flag);
            let flag_style = if is_selected {
                Style::default()
                    .fg(colors.flag)
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.flag)
            };
            spans.push(Span::styled(flag_display, flag_style));

            // Global indicator
            if flag.global {
                spans.push(Span::styled(" [G]", Style::default().fg(colors.help)));
            }

            // Required indicator
            if flag.required {
                spans.push(Span::styled(" *", Style::default().fg(colors.required)));
            }

            // Value display for string flags
            if let Some((_, FlagValue::String(s))) = value {
                spans.push(Span::styled(" = ", Style::default().fg(colors.help)));

                if is_editing {
                    // Show the edit_input text with cursor
                    let edit_text = app.edit_input.text();
                    spans.push(Span::styled(
                        edit_text,
                        Style::default()
                            .fg(colors.value)
                            .add_modifier(Modifier::UNDERLINED),
                    ));
                    spans.push(Span::styled(
                        "▎",
                        Style::default()
                            .fg(colors.value)
                            .add_modifier(Modifier::SLOW_BLINK),
                    ));
                } else if s.is_empty() {
                    // Show choices hint or default
                    if let Some(ref arg) = flag.arg {
                        if let Some(ref choices) = arg.choices {
                            let hint = choices.choices.join("|");
                            spans.push(Span::styled(
                                format!("<{}>", hint),
                                Style::default().fg(colors.choice),
                            ));
                        } else {
                            spans.push(Span::styled(
                                format!("<{}>", arg.name),
                                Style::default().fg(colors.default_val),
                            ));
                        }
                    }
                } else {
                    spans.push(Span::styled(s.as_str(), Style::default().fg(colors.value)));
                    // Show "(default)" if value matches the spec default
                    if let Some(def) = default_val {
                        if s == def {
                            spans.push(Span::styled(
                                " (default)",
                                Style::default().fg(colors.default_val).italic(),
                            ));
                        }
                    }
                }
            }

            // Help text
            if let Some(help) = &flag.help {
                spans.push(Span::styled(" — ", Style::default().fg(colors.help)));
                spans.push(Span::styled(
                    help.as_str(),
                    Style::default().fg(colors.help),
                ));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                let bg = if is_editing {
                    colors.editing_bg
                } else {
                    colors.selected_bg
                };
                item = item.style(Style::default().bg(bg));
            }
            item
        })
        .collect();

    let mut state = ListState::default()
        .with_selected(if is_focused { Some(flag_index) } else { None })
        .with_offset(app.flag_scroll());
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_arg_list(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Register area for click hit-testing
    app.click_regions.register(area, Focus::Args);

    let is_focused = app.focus() == Focus::Args;
    let border_color = if is_focused {
        colors.active_border
    } else {
        colors.inactive_border
    };

    let arg_total = app.arg_values.len();
    let arg_index = app.arg_index();

    let title = if arg_total > 0 && is_focused {
        format!(" Arguments [{}/{}] ", arg_index + 1, arg_total)
    } else {
        format!(" Arguments ({}) ", arg_total)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(title)
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    // Calculate inner height for scroll offset (must happen before borrowing app.arg_values)
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Args, inner_height);

    let items: Vec<ListItem> = app
        .arg_values
        .iter()
        .enumerate()
        .map(|(i, arg_val)| {
            let is_selected = is_focused && i == arg_index;
            let is_editing = is_selected && app.editing;

            let mut spans = Vec::new();

            // Selection cursor indicator
            if is_selected {
                spans.push(Span::styled(
                    "▸ ",
                    Style::default().fg(colors.active_border),
                ));
            } else {
                spans.push(Span::styled("  ", Style::default()));
            }

            // Required/optional indicator
            if arg_val.required {
                spans.push(Span::styled("● ", Style::default().fg(colors.required)));
            } else {
                spans.push(Span::styled("○ ", Style::default().fg(colors.help)));
            }

            // Arg name
            let name_style = if is_selected {
                Style::default().fg(colors.arg).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(colors.arg)
            };
            let bracket = if arg_val.required { "<>" } else { "[]" };
            spans.push(Span::styled(
                format!("{}{}{}", &bracket[..1], arg_val.name, &bracket[1..]),
                name_style,
            ));

            // Value display
            spans.push(Span::styled(" = ", Style::default().fg(colors.help)));

            if is_editing {
                // Show edit_input text with cursor
                let edit_text = app.edit_input.text();
                spans.push(Span::styled(
                    edit_text,
                    Style::default()
                        .fg(colors.value)
                        .add_modifier(Modifier::UNDERLINED),
                ));
                spans.push(Span::styled(
                    "▎",
                    Style::default()
                        .fg(colors.value)
                        .add_modifier(Modifier::SLOW_BLINK),
                ));
            } else if arg_val.value.is_empty() {
                if !arg_val.choices.is_empty() {
                    let hint = arg_val.choices.join("|");
                    spans.push(Span::styled(
                        format!("<{}>", hint),
                        Style::default().fg(colors.choice),
                    ));
                } else {
                    spans.push(Span::styled(
                        "(empty)",
                        Style::default().fg(colors.default_val),
                    ));
                }
            } else {
                spans.push(Span::styled(
                    &arg_val.value,
                    Style::default().fg(colors.value),
                ));
            }

            // Show choices if arg has them and we're not editing
            if !arg_val.choices.is_empty() && !arg_val.value.is_empty() && !is_editing {
                spans.push(Span::styled(
                    format!(" [{}]", arg_val.choices.join("|")),
                    Style::default().fg(colors.choice),
                ));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                let bg = if is_editing {
                    colors.editing_bg
                } else {
                    colors.selected_bg
                };
                item = item.style(Style::default().bg(bg));
            }
            item
        })
        .collect();

    let mut state = ListState::default()
        .with_selected(if is_focused { Some(arg_index) } else { None })
        .with_offset(app.arg_scroll());
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut state);
}

fn render_help_bar(frame: &mut Frame, app: &App, area: Rect, colors: &UiColors) {
    let help_text = app.current_help().unwrap_or_default();

    let keybinds = if app.editing {
        "Enter: confirm  Esc: cancel"
    } else if app.filtering {
        "Enter: apply  Esc: clear  ↑↓: navigate"
    } else {
        match app.focus() {
            Focus::Commands => {
                "Enter/→: select  ↑↓: navigate  Tab: next  /: filter  T: theme  Esc: back  q: quit"
            }
            Focus::Flags => {
                "Enter/Space: toggle  ↑↓: navigate  Tab: next  /: filter  T: theme  q: quit"
            }
            Focus::Args => "Enter: edit  ↑↓: navigate  Tab: next  T: theme  Esc: back  q: quit",
            Focus::Preview => "Enter: accept  Tab: next  T: theme  Esc: back  q: quit",
        }
    };

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Help text for current item
    let help = Paragraph::new(Line::from(vec![
        Span::styled(" 💡 ", Style::default()),
        Span::styled(help_text, Style::default().fg(colors.help).italic()),
    ]));
    frame.render_widget(help, layout[0]);

    // Keybinding hints with theme name
    let theme_indicator = format!(" [{}]", app.theme_name.display_name());
    let hints = Paragraph::new(Line::from(vec![
        Span::styled(format!(" {keybinds}"), Style::default().fg(colors.help)),
        Span::styled(
            theme_indicator,
            Style::default().fg(colors.active_border).italic(),
        ),
    ]))
    .style(Style::default().bg(colors.bar_bg));
    frame.render_widget(hints, layout[1]);
}

/// Render the command preview bar at the bottom with colorized parts.
fn render_preview(frame: &mut Frame, app: &App, area: Rect, colors: &UiColors) {
    let is_focused = app.focus() == Focus::Preview;
    let border_color = if is_focused {
        colors.active_border
    } else {
        colors.inactive_border
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color))
        .title(" Command ")
        .title_style(Style::default().fg(border_color).bold())
        .padding(Padding::horizontal(1));

    let prefix = if is_focused { "▶ " } else { "$ " };
    let bold = if is_focused {
        Modifier::BOLD
    } else {
        Modifier::empty()
    };

    // Build colorized command by tokenizing the built command string
    let command = app.build_command();
    let mut spans = vec![Span::styled(prefix, Style::default().fg(colors.command))];
    colorize_command(&command, app, &mut spans, colors, bold);

    let paragraph = Paragraph::new(Line::from(spans))
        .block(block)
        .wrap(Wrap { trim: false });

    frame.render_widget(paragraph, area);
}

/// Colorize a built command string by categorizing each token as bin, subcommand, flag, or arg.
fn colorize_command(
    command: &str,
    app: &App,
    spans: &mut Vec<Span<'static>>,
    colors: &UiColors,
    bold: Modifier,
) {
    let bin = if app.spec.bin.is_empty() {
        &app.spec.name
    } else {
        &app.spec.bin
    };

    // Collect known subcommand names from the path
    let subcommand_names: std::collections::HashSet<&str> =
        app.command_path.iter().map(|s| s.as_str()).collect();

    let tokens: Vec<&str> = command.split_whitespace().collect();
    let mut i = 0;
    let mut expect_flag_value = false;

    while i < tokens.len() {
        if i > 0 {
            spans.push(Span::raw(" "));
        }

        let token = tokens[i];

        if i == 0 && token == bin {
            // Binary name — bold with primary text color
            spans.push(Span::styled(
                token.to_string(),
                Style::default()
                    .fg(colors.preview_cmd)
                    .add_modifier(bold | Modifier::BOLD),
            ));
        } else if expect_flag_value {
            // Value following a flag like --tag
            spans.push(Span::styled(
                token.to_string(),
                Style::default().fg(colors.value).add_modifier(bold),
            ));
            expect_flag_value = false;
        } else if token.starts_with('-') {
            // Flag token
            spans.push(Span::styled(
                token.to_string(),
                Style::default().fg(colors.flag).add_modifier(bold),
            ));
            // Check if this flag expects a value (next token is not a flag and not a subcommand)
            if let Some(&next) = tokens.get(i + 1) {
                if !next.starts_with('-') && !subcommand_names.contains(next) {
                    // Heuristic: flags like --tag expect a value; boolean flags like --rollback don't
                    // We peek ahead: if the next token doesn't look like a subcommand, treat it as a value
                    expect_flag_value = true;
                }
            }
        } else if subcommand_names.contains(token) {
            // Subcommand name
            spans.push(Span::styled(
                token.to_string(),
                Style::default().fg(colors.command).add_modifier(bold),
            ));
        } else {
            // Positional argument value
            spans.push(Span::styled(
                token.to_string(),
                Style::default().fg(colors.arg).add_modifier(bold),
            ));
        }

        i += 1;
    }
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
    use crate::app::FlagValue;
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
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_deploy_leaf() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_run_command() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let idx = subs.iter().position(|(n, _)| n.as_str() == "run").unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flags_toggled() {
        let mut app = App::new(sample_spec());
        // Navigate to deploy
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

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
        let subs = app.visible_subcommands();
        let idx = subs.iter().position(|(n, _)| n.as_str() == "init").unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        // Focus on args and start editing
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();
        app.edit_input.set_text("my-project");
        app.arg_values[0].value = "my-project".to_string();

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_filter_active() {
        let mut app = App::new(sample_spec());
        app.filtering = true;
        app.filter_input.set_text("pl");
        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_preview_focused() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let idx = subs.iter().position(|(n, _)| n.as_str() == "run").unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        app.arg_values[0].value = "lint".to_string();
        app.set_focus(Focus::Preview);

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_deep_navigation() {
        let mut app = App::new(sample_spec());

        // Navigate to plugin > install
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "plugin")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "install")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

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
        app.set_command_index(0);
        app.navigate_into_selected();
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
        // Navigate to deploy (has multiple flags)
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        // Focus on flags and start filtering
        app.set_focus(Focus::Flags);
        app.filtering = true;
        app.filter_input.set_text("roll");

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flag_filter_verbose() {
        let mut app = App::new(sample_spec());
        // Navigate to deploy
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        // Focus on flags and filter for "verb" (should match --verbose)
        app.set_focus(Focus::Flags);
        app.filtering = true;
        app.filter_input.set_text("verb");

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
        let subs = app.visible_subcommands();
        let config_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.set_command_index(config_idx);
        app.navigate_into_selected();

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
        let subs = app.visible_subcommands();
        let deploy_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(deploy_idx);
        app.navigate_into_selected();

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
        let subs = app.visible_subcommands();
        let idx = subs.iter().position(|(n, _)| n.as_str() == "init").unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        app.arg_values[0].value = "hello".to_string();

        let output = render_to_string(&mut app, 100, 24);
        assert!(output.contains("mycli init hello"));
    }

    #[test]
    fn test_render_aliases_shown() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

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
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        let output = render_to_string(&mut app, 100, 24);
        // Required args should be shown with <> brackets
        assert!(output.contains("<environment>"));
    }

    #[test]
    fn test_render_choices_display() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

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
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        // Focus flags and toggle yes flag
        app.set_focus(Focus::Flags);
        let flag_values = app.current_flag_values();
        let yes_idx = flag_values.iter().position(|(n, _)| n == "yes").unwrap();
        app.set_flag_index(yes_idx);

        // Toggle via space key
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

        let output = render_to_string(&mut app, 100, 24);
        // After toggling, should show checkbox checked indicator
        assert!(output.contains("☑"));
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

        // Shift+T cycles the theme
        let key = KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE);
        app.handle_key(key);
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);
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

    // ── Breadcrumb widget tests ─────────────────────────────────────────

    #[test]
    fn test_breadcrumb_shows_binary_name() {
        let mut app = App::new(sample_spec());
        let output = render_to_string(&mut app, 100, 24);
        // The breadcrumb should display the binary name
        assert!(output.contains("mycli"));
    }

    #[test]
    fn test_breadcrumb_shows_path() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.set_command_index(idx);
        app.navigate_into_selected();

        let output = render_to_string(&mut app, 100, 24);
        // Should show both mycli and config in the breadcrumb
        assert!(output.contains("mycli"));
        assert!(output.contains("config"));
    }
}
