use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Padding, Paragraph, Wrap},
    Frame,
};
use ratatui_themes::ThemePalette;

#[cfg(test)]
extern crate insta;

use crate::app::{flatten_command_tree, App, FlagValue, Focus};

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
            choice: p.info,
            default_val: p.muted,
            count: p.secondary,
            bg: p.bg,
            bar_bg,
        }
    }
}

/// Build spans with highlighted characters based on fuzzy match indices.
/// Returns a Vec<Span> where matching characters are styled with highlight_style.
fn build_highlighted_text(
    text: &str,
    pattern: &str,
    normal_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    use crate::app::fuzzy_match_indices;
    use nucleo_matcher::{Config, Matcher};

    let mut matcher = Matcher::new(Config::DEFAULT);
    let (_score, indices) = fuzzy_match_indices(text, pattern, &mut matcher);

    if indices.is_empty() {
        // No matches, return whole text with normal style
        return vec![Span::styled(text.to_string(), normal_style)];
    }

    let mut spans = Vec::new();
    let chars: Vec<char> = text.chars().collect();
    let mut last_idx = 0;

    for &match_idx in &indices {
        let idx = match_idx as usize;
        if idx >= chars.len() {
            continue;
        }

        // Add normal text before this match
        if last_idx < idx {
            let before: String = chars[last_idx..idx].iter().collect();
            if !before.is_empty() {
                spans.push(Span::styled(before, normal_style));
            }
        }

        // Add highlighted match character
        spans.push(Span::styled(chars[idx].to_string(), highlight_style));
        last_idx = idx + 1;
    }

    // Add remaining text after last match
    if last_idx < chars.len() {
        let after: String = chars[last_idx..].iter().collect();
        if !after.is_empty() {
            spans.push(Span::styled(after, normal_style));
        }
    }

    spans
}

/// Render the full UI: command panel, flag panel, arg panel, preview, help bar.
pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    // Top-level vertical layout:
    //   [main content area]
    //   [help / status bar]
    //   [command preview]
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(6),    // main content
            Constraint::Length(2), // help text
            Constraint::Length(3), // command preview
        ])
        .split(area);

    // Clear click regions each frame so they're rebuilt from current layout
    app.click_regions.clear();

    let palette = app.palette();
    let colors = UiColors::from_palette(&palette);

    render_main_content(frame, app, outer[0], &colors);
    render_help_bar(frame, app, outer[1], &colors);
    render_preview(frame, app, outer[2], &colors);

    // Register preview area for click hit-testing
    app.click_regions.register(outer[2], Focus::Preview);
}

/// Render the main content area with panels for commands, flags, and args.
fn render_main_content(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Commands tree should always be visible (shows entire tree, not just subcommands)
    let has_commands = !app.command_tree_nodes.is_empty();

    // Always show Flags and Args panes, even if empty
    if has_commands {
        // Split horizontally: commands on left, flags+args on right
        let h_split = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(area);

        render_command_list(frame, app, h_split[0], colors);

        // Always split vertically for flags and args
        let v_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(h_split[1]);
        render_flag_list(frame, app, v_split[0], colors);
        render_arg_list(frame, app, v_split[1], colors);
    } else {
        // No commands - still show flags and args
        let v_split = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(area);
        render_flag_list(frame, app, v_split[0], colors);
        render_arg_list(frame, app, v_split[1], colors);
    }
}

/// Render the command tree panel using the TreeView widget.
fn render_command_list(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Register area for click hit-testing
    app.click_regions.register(area, Focus::Commands);

    let is_focused = app.focus() == Focus::Commands;
    let border_color = if is_focused {
        colors.active_border
    } else {
        colors.inactive_border
    };

    // Flatten the tree for display
    let flat_commands = flatten_command_tree(&app.command_tree_nodes);

    let title = if app.filtering && !app.filter().is_empty() {
        format!(" Commands (/{})", app.filter())
    } else {
        " Commands ".to_string()
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_style(Style::default().fg(border_color));

    // Calculate inner height for scroll offset (area minus borders)
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Commands, inner_height);

    // Compute match scores if filtering
    let match_scores =
        if app.filtering && !app.filter().is_empty() && app.focus() == Focus::Commands {
            app.compute_tree_match_scores()
        } else {
            std::collections::HashMap::new()
        };

    // Get selected index
    let selected_index = app.command_index();

    // Build list items with indentation markers
    let items: Vec<ListItem> = flat_commands
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let is_selected = i == selected_index;
            let is_match = match_scores.get(&cmd.id).copied().unwrap_or(0) > 0;

            let mut spans = Vec::new();

            // Selection cursor
            if is_selected {
                spans.push(Span::styled(
                    "▶ ",
                    Style::default()
                        .fg(colors.active_border)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::raw("  "));
            }

            // Depth-based indentation (2 spaces per level)
            if cmd.depth > 0 {
                let indent = "  ".repeat(cmd.depth);
                spans.push(Span::styled(indent, Style::default().fg(colors.help)));
            }

            // Command name (without help text for now)
            let name_text = if !cmd.aliases.is_empty() {
                format!("{} ({})", cmd.name, cmd.aliases.join(", "))
            } else {
                cmd.name.clone()
            };

            // Apply styling based on selection and match
            if is_selected {
                // Selected - apply highlighting if filtering
                if !match_scores.is_empty() {
                    let pattern = app.filter();
                    let normal_style = Style::default()
                        .fg(colors.command)
                        .add_modifier(Modifier::BOLD);
                    let highlight_style = Style::default()
                        .fg(colors.bg)
                        .bg(colors.command)
                        .add_modifier(Modifier::BOLD);
                    let highlighted =
                        build_highlighted_text(&name_text, pattern, normal_style, highlight_style);
                    spans.extend(highlighted);
                } else {
                    spans.push(Span::styled(
                        name_text.clone(),
                        Style::default()
                            .fg(colors.command)
                            .add_modifier(Modifier::BOLD),
                    ));
                }
            } else if !is_match && !match_scores.is_empty() {
                // Non-matching when filtering - dim
                spans.push(Span::styled(
                    name_text.clone(),
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM),
                ));
            } else if !match_scores.is_empty() {
                // Matching - highlight characters
                let pattern = app.filter();
                let normal_style = Style::default().fg(colors.command);
                let highlight_style = Style::default()
                    .fg(colors.command)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                let highlighted =
                    build_highlighted_text(&name_text, pattern, normal_style, highlight_style);
                spans.extend(highlighted);
            } else {
                // Normal display
                spans.push(Span::styled(
                    name_text.clone(),
                    Style::default().fg(colors.command),
                ));
            }

            // Right-align help text if present
            if let Some(help) = &cmd.help {
                // Calculate padding to right-align the description
                // Area width minus borders (2) minus current content length
                let current_len = spans.iter().map(|s| s.content.len()).sum::<usize>();
                let help_with_sep = format!(" — {}", help);
                let available_width = area.width.saturating_sub(2) as usize;

                if current_len + help_with_sep.len() < available_width {
                    let padding = available_width.saturating_sub(current_len + help_with_sep.len());
                    spans.push(Span::raw(" ".repeat(padding)));
                } else {
                    spans.push(Span::raw(" "));
                }

                let help_style = if !is_match && !match_scores.is_empty() {
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(colors.help)
                };
                spans.push(Span::styled("— ", help_style));
                spans.push(Span::styled(help.as_str(), help_style));
            }

            ListItem::new(Line::from(spans))
        })
        .collect();

    let selected_index = app.command_index();
    let mut state = ListState::default()
        .with_selected(if is_focused {
            Some(selected_index)
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

    // Compute index for scroll visibility
    let flag_index = app.flag_index();

    // Compute match scores for filtering
    let match_scores = if app.filtering && !app.filter().is_empty() && app.focus() == Focus::Flags {
        app.compute_flag_match_scores()
    } else {
        std::collections::HashMap::new()
    };

    let title = if app.filtering && !app.filter().is_empty() && app.focus() == Focus::Flags {
        format!(" Flags (/{}) ", app.filter())
    } else {
        " Flags ".to_string()
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

            // Check if this flag matches the filter
            let score = match_scores.get(&flag.name).copied().unwrap_or(1);
            let is_match = score > 0 || match_scores.is_empty();

            let mut spans = Vec::new();

            // Selection cursor indicator (prominent triangle)
            if is_selected {
                spans.push(Span::styled(
                    "▶ ",
                    Style::default()
                        .fg(colors.active_border)
                        .add_modifier(Modifier::BOLD),
                ));
            } else {
                spans.push(Span::styled("  ", Style::default()));
            }

            // Checkbox / toggle indicator using checkmark style (✓/○)
            let indicator = match value.map(|(_, v)| v) {
                Some(FlagValue::Bool(true)) => Span::styled(
                    "✓ ",
                    Style::default().fg(colors.arg).add_modifier(Modifier::BOLD),
                ),
                Some(FlagValue::Bool(false)) => {
                    Span::styled("○ ", Style::default().fg(colors.help))
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
                None => Span::styled("○ ", Style::default().fg(colors.help)),
            };
            spans.push(indicator);

            // Flag display (short + long) with highlighting
            let flag_display = flag_display_string(flag);

            if is_selected {
                // Selected flag - apply highlighting with high-contrast colors for visibility against selection background
                if !match_scores.is_empty() {
                    let pattern = app.filter();
                    let normal_style = Style::default()
                        .fg(colors.flag)
                        .add_modifier(Modifier::BOLD);
                    // Use inverted colors for matched characters to stand out against selection bg
                    let highlight_style = Style::default()
                        .fg(colors.bg)
                        .bg(colors.flag)
                        .add_modifier(Modifier::BOLD);
                    let highlighted_spans = build_highlighted_text(
                        &flag_display,
                        pattern,
                        normal_style,
                        highlight_style,
                    );
                    spans.extend(highlighted_spans);
                } else {
                    // No filter active - just bold
                    let flag_style = Style::default()
                        .fg(colors.flag)
                        .add_modifier(Modifier::BOLD);
                    spans.push(Span::styled(flag_display, flag_style));
                }
            } else if !is_match {
                // Non-matching flag - subdued/dimmed
                let flag_style = Style::default().fg(colors.help).add_modifier(Modifier::DIM);
                spans.push(Span::styled(flag_display, flag_style));
            } else if !match_scores.is_empty() {
                // Matching flag - highlight matched characters
                let pattern = app.filter();
                let normal_style = Style::default().fg(colors.flag);
                let highlight_style = Style::default()
                    .fg(colors.flag)
                    .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
                let highlighted_spans =
                    build_highlighted_text(&flag_display, pattern, normal_style, highlight_style);
                spans.extend(highlighted_spans);
            } else {
                // Normal display
                let flag_style = Style::default().fg(colors.flag);
                spans.push(Span::styled(flag_display, flag_style));
            }

            // Global indicator
            if flag.global {
                let global_style = if !is_match {
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(colors.help)
                };
                spans.push(Span::styled(" [G]", global_style));
            }

            // Required indicator
            if flag.required {
                let required_style = if !is_match {
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(colors.required)
                };
                spans.push(Span::styled(" *", required_style));
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

            // Right-align help text if present
            if let Some(help) = &flag.help {
                // Calculate padding to right-align the description
                let current_len = spans.iter().map(|s| s.content.len()).sum::<usize>();
                let help_with_sep = format!(" — {}", help);
                let available_width = area.width.saturating_sub(2).saturating_sub(2) as usize; // borders + padding

                if current_len + help_with_sep.len() < available_width {
                    let padding = available_width.saturating_sub(current_len + help_with_sep.len());
                    spans.push(Span::raw(" ".repeat(padding)));
                } else {
                    spans.push(Span::raw(" "));
                }

                let help_style = if !is_match {
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(colors.help)
                };
                spans.push(Span::styled("— ", help_style));
                spans.push(Span::styled(help.as_str(), help_style));
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

    let arg_index = app.arg_index();

    let title = " Arguments ".to_string();

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

            // Selection cursor indicator (prominent triangle)
            if is_selected {
                spans.push(Span::styled(
                    "▶ ",
                    Style::default()
                        .fg(colors.active_border)
                        .add_modifier(Modifier::BOLD),
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
        app.navigate_to_command(&["config"]);
        // Expand config to show its subcommands
        app.command_tree_state.expand("config");
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
        app.edit_input.set_text("my-project");
        app.arg_values[0].value = "my-project".to_string();

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
        app.filtering = true;
        app.filter_input.set_text("roll");

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flag_filter_verbose() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus on flags and filter for "verb" (should match --verbose)
        app.set_focus(Focus::Flags);
        app.filtering = true;
        app.filter_input.set_text("verb");

        let output = render_to_string(&mut app, 100, 24);
        insta::assert_snapshot!(output);
    }

    #[test]
    fn snapshot_flag_filter_selected_item() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus on flags and filter for "tag" (matches --tag)
        app.set_focus(Focus::Flags);
        app.filtering = true;
        app.filter_input.set_text("tag");

        // Move selection to the matching flag (tag is at index 0)
        app.flag_list_state.selected_index = 0;

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
        app.command_tree_state.expand("config");

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
        app.command_tree_state.expand("config");

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
        app.navigate_to_command(&["config"]);

        let output = render_to_string(&mut app, 100, 24);
        // Should show both mycli and config in the breadcrumb
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
        app.command_tree_state.selected_index = 2;
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
        app.filtering = true;
        app.filter_input.set_text("roll");

        let output = render_to_string(&mut app, 100, 24);
        // Should show filter query in title, no counts
        assert!(output.contains("Flags (/roll)"));
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
                "Theme {:?} failed to render subcommand in breadcrumb",
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
}
