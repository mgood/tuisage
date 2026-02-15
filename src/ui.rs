use std::sync::atomic::Ordering;

use ratatui::{
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
    Frame,
};
use ratatui_themes::ThemeName;
use tui_term::widget::PseudoTerminal;

#[cfg(test)]
extern crate insta;

use crate::app::{flatten_command_tree, App, AppMode, FlagValue, Focus};
use crate::widgets::{
    build_help_line, panel_block, panel_title, push_edit_cursor, push_highlighted_name,
    push_selection_cursor, render_help_overlays, selection_bg, CommandPreview, HelpBar,
    ItemContext, PanelState, SelectList, UiColors,
};

/// Render the full UI: command panel, flag panel, arg panel, preview, help bar.
pub fn render(frame: &mut Frame, app: &mut App) {
    let palette = app.palette();
    let colors = UiColors::from_palette(&palette);

    if app.mode == AppMode::Executing {
        render_execution_view(frame, app, &colors);
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

    // Clear click regions each frame so they're rebuilt from current layout
    app.click_regions.clear();

    render_preview(frame, app, outer[0], &colors);
    render_main_content(frame, app, outer[1], &colors);
    render_help_bar(frame, app, outer[2], &colors);

    // Register preview area for click hit-testing
    app.click_regions.register(outer[0], Focus::Preview);

    // Render choice select overlay on top of everything
    if app.choice_select.is_some() {
        render_choice_select(frame, app, area, &colors);
    }

    // Render theme picker overlay on top of everything
    if app.theme_picker.is_some() {
        render_theme_picker(frame, app, area, &colors);
    }
}

/// Render the execution view: command at top, terminal output in middle, status at bottom.
fn render_execution_view(frame: &mut Frame, app: &App, colors: &UiColors) {
    let area = frame.area();

    // Layout: [command display] [terminal output] [status bar]
    let outer = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // command display
            Constraint::Min(4),    // terminal output
            Constraint::Length(1), // status bar
        ])
        .split(area);

    // --- Command display at top ---
    let command_display = app
        .execution
        .as_ref()
        .map(|e| e.command_display.clone())
        .unwrap_or_default();

    let cmd_block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(colors.active_border))
        .title(" Command ")
        .title_style(Style::default().fg(colors.active_border).bold());

    let cmd_spans = vec![
        Span::styled("$ ", Style::default().fg(colors.command)),
        Span::styled(
            command_display,
            Style::default()
                .fg(colors.preview_cmd)
                .add_modifier(Modifier::BOLD),
        ),
    ];
    let cmd_paragraph = Paragraph::new(Line::from(cmd_spans))
        .block(cmd_block)
        .wrap(Wrap { trim: false });
    frame.render_widget(cmd_paragraph, outer[0]);

    // --- Terminal output ---
    let exited = app
        .execution
        .as_ref()
        .map(|e| e.exited.load(Ordering::Relaxed))
        .unwrap_or(false);

    let term_block = Block::default().borders(Borders::NONE);

    if let Some(ref exec) = app.execution {
        if let Ok(parser) = exec.parser.read() {
            let pseudo_term = PseudoTerminal::new(parser.screen())
                .block(term_block)
                .style(Style::default().fg(colors.preview_cmd).bg(colors.bg));
            frame.render_widget(pseudo_term, outer[1]);
        } else {
            // Fallback if lock is poisoned
            let fallback = Paragraph::new("(terminal output unavailable)")
                .block(term_block)
                .style(Style::default().fg(colors.help));
            frame.render_widget(fallback, outer[1]);
        }
    } else {
        let fallback = Paragraph::new("(no execution state)")
            .block(term_block)
            .style(Style::default().fg(colors.help));
        frame.render_widget(fallback, outer[1]);
    }

    // --- Status bar at bottom ---
    let status_text = if exited {
        let exit_code = app.execution_exit_status().unwrap_or_default();
        format!(
            " Exited ({}) — press Esc/Enter/q to close ",
            if exit_code.is_empty() {
                "unknown".to_string()
            } else {
                exit_code
            }
        )
    } else {
        " Running… (input is forwarded to the process) ".to_string()
    };

    let status_style = if exited {
        Style::default()
            .fg(colors.help)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    } else {
        Style::default()
            .fg(colors.command)
            .add_modifier(Modifier::BOLD | Modifier::REVERSED)
    };

    let status = Paragraph::new(status_text).style(status_style);
    frame.render_widget(status, outer[2]);
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

    let ps = {
        let base = PanelState::from_app(app, Focus::Commands, colors);
        let scores = if !base.filter_text.is_empty() {
            app.compute_tree_match_scores()
        } else {
            std::collections::HashMap::new()
        };
        base.with_scores(scores)
    };

    // Flatten the tree for display
    let flat_commands = flatten_command_tree(&app.command_tree_nodes);

    let title = panel_title("Commands", &ps);
    let block = panel_block(title, &ps);

    // Calculate inner height for scroll offset (area minus borders)
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Commands, inner_height);

    // Get selected index
    let selected_index = app.command_index();

    // Build list items with indentation markers
    let mut help_entries: Vec<(usize, Line<'static>)> = Vec::new();
    let items: Vec<ListItem> = flat_commands
        .iter()
        .enumerate()
        .map(|(i, cmd)| {
            let is_selected = i == selected_index;
            let ctx = ItemContext::new(&cmd.id, is_selected, &ps);

            let mut spans = Vec::new();

            // Selection cursor
            push_selection_cursor(&mut spans, is_selected, colors);

            // Depth-based indentation with vertical line prefix
            if cmd.depth > 0 {
                let indent = "  ".repeat(cmd.depth - 1);
                spans.push(Span::styled(indent, Style::default().fg(colors.help)));
                spans.push(Span::styled("│ ", Style::default().fg(colors.help)));
            }

            // Command name (with aliases)
            let name_text = if !cmd.aliases.is_empty() {
                format!("{} ({})", cmd.name, cmd.aliases.join(", "))
            } else {
                cmd.name.clone()
            };

            push_highlighted_name(
                &mut spans,
                &name_text,
                colors.command,
                &ctx,
                &ps,
                colors,
            );

            // Collect help text for overlay rendering
            if let Some(help) = &cmd.help {
                help_entries.push((i, build_help_line(help, &ctx, &ps, colors)));
            }

            let mut item = ListItem::new(Line::from(spans));
            if is_selected {
                item = item.style(selection_bg(false, colors));
            }
            item
        })
        .collect();

    let selected_index = app.command_index();
    let mut state = ListState::default()
        .with_selected(if ps.is_focused {
            Some(selected_index)
        } else {
            None
        })
        .with_offset(app.command_scroll());

    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut state);

    // Render right-aligned help text overlays (no padding on Commands panel)
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    render_help_overlays(frame.buffer_mut(), &help_entries, app.command_scroll(), inner);
}
fn render_flag_list(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Register area for click hit-testing
    app.click_regions.register(area, Focus::Flags);

    let ps = {
        let base = PanelState::from_app(app, Focus::Flags, colors);
        let scores = if !base.filter_text.is_empty() {
            app.compute_flag_match_scores()
        } else {
            std::collections::HashMap::new()
        };
        base.with_scores(scores)
    };

    // Compute index for scroll visibility
    let flag_index = app.flag_index();

    let title = panel_title("Flags", &ps);
    let block = panel_block(title, &ps);

    // Calculate inner height for scroll offset
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Flags, inner_height);

    // Re-fetch data after mutable borrow ends
    let flags = app.visible_flags();
    let flag_values = app.current_flag_values();

    // Pre-compute default values for each flag so we can show "(default)" indicator
    let flag_defaults: Vec<Option<String>> =
        flags.iter().map(|f| f.default.first().cloned()).collect();

    let mut help_entries: Vec<(usize, Line<'static>)> = Vec::new();
    let items: Vec<ListItem> = flags
        .iter()
        .enumerate()
        .map(|(i, flag)| {
            let is_selected = ps.is_focused && i == flag_index;
            let is_editing = is_selected && app.editing;
            let value = flag_values.iter().find(|(n, _)| n == &flag.name);
            let default_val = flag_defaults.get(i).and_then(|d| d.as_ref());

            let ctx = ItemContext::new(&flag.name, is_selected, &ps);

            let mut spans = Vec::new();

            // Selection cursor indicator
            push_selection_cursor(&mut spans, is_selected, colors);

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

            push_highlighted_name(
                &mut spans,
                &flag_display,
                colors.flag,
                &ctx,
                &ps,
                colors,
            );

            // Global indicator
            if flag.global {
                let global_style = if !ctx.is_match && !ps.match_scores.is_empty() {
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(colors.help)
                };
                spans.push(Span::styled(" [G]", global_style));
            }

            // Required indicator
            if flag.required {
                let required_style = if !ctx.is_match && !ps.match_scores.is_empty() {
                    Style::default().fg(colors.help).add_modifier(Modifier::DIM)
                } else {
                    Style::default().fg(colors.required)
                };
                spans.push(Span::styled(" *", required_style));
            }

            // Value display for string flags
            if let Some((_, FlagValue::String(s))) = value {
                spans.push(Span::styled(" = ", Style::default().fg(colors.help)));

                // Check if the choice select box is open for this flag
                let is_choice_selecting = app.choice_select.as_ref().is_some_and(|cs| {
                    cs.source_panel == Focus::Flags && cs.source_index == i
                });

                if is_choice_selecting || is_editing {
                    // Show the edit cursor — when choice selecting, the text input is also active
                    let before_cursor = app.edit_input.text_before_cursor();
                    let after_cursor = app.edit_input.text_after_cursor();
                    push_edit_cursor(&mut spans, before_cursor, after_cursor, colors);
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
                    spans.push(Span::styled(s.to_string(), Style::default().fg(colors.value)));
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

            // Collect help text for overlay rendering
            if let Some(help) = &flag.help {
                help_entries.push((i, build_help_line(help, &ctx, &ps, colors)));
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                item = item.style(selection_bg(is_editing, colors));
            }
            item
        })
        .collect();

    let mut state = ListState::default()
        .with_selected(if ps.is_focused { Some(flag_index) } else { None })
        .with_offset(app.flag_scroll());
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut state);

    // Render right-aligned help text overlays (2 = border + horizontal padding)
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    render_help_overlays(frame.buffer_mut(), &help_entries, app.flag_scroll(), inner);
}

fn render_arg_list(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    // Register area for click hit-testing
    app.click_regions.register(area, Focus::Args);

    let ps = {
        let base = PanelState::from_app(app, Focus::Args, colors);
        let scores = if !base.filter_text.is_empty() {
            app.compute_arg_match_scores()
        } else {
            std::collections::HashMap::new()
        };
        base.with_scores(scores)
    };

    let arg_index = app.arg_index();

    let title = panel_title("Arguments", &ps);
    let block = panel_block(title, &ps);

    // Calculate inner height for scroll offset (must happen before borrowing app.arg_values)
    let inner_height = area.height.saturating_sub(2) as usize;
    app.ensure_visible(Focus::Args, inner_height);

    let mut help_entries: Vec<(usize, Line<'static>)> = Vec::new();
    let items: Vec<ListItem> = app
        .arg_values
        .iter()
        .enumerate()
        .map(|(i, arg_val)| {
            let is_selected = ps.is_focused && i == arg_index;
            let is_editing = is_selected && app.editing;

            let ctx = ItemContext::new(&arg_val.name, is_selected, &ps);

            let mut spans = Vec::new();

            // Selection cursor indicator
            push_selection_cursor(&mut spans, is_selected, colors);

            // Required/optional indicator
            if arg_val.required {
                spans.push(Span::styled("● ", Style::default().fg(colors.required)));
            } else {
                spans.push(Span::styled("○ ", Style::default().fg(colors.help)));
            }

            // Arg name with highlighting
            let bracket = if arg_val.required { "<>" } else { "[]" };
            let arg_display = format!("{}{}{}", &bracket[..1], arg_val.name, &bracket[1..]);

            push_highlighted_name(
                &mut spans,
                &arg_display,
                colors.arg,
                &ctx,
                &ps,
                colors,
            );

            // Value display
            spans.push(Span::styled(" = ", Style::default().fg(colors.help)));

            // Check if the choice select box is open for this arg
            let is_choice_selecting = app.choice_select.as_ref().is_some_and(|cs| {
                cs.source_panel == Focus::Args && cs.source_index == i
            });

            if is_choice_selecting || is_editing {
                // Show the edit cursor — when choice selecting, the text input is also active
                let before_cursor = app.edit_input.text_before_cursor();
                let after_cursor = app.edit_input.text_after_cursor();
                push_edit_cursor(&mut spans, before_cursor, after_cursor, colors);
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
                    arg_val.value.clone(),
                    Style::default().fg(colors.value),
                ));
            }

            // Show choices if arg has them and we're not editing (and not choice-selecting)
            if !arg_val.choices.is_empty() && !arg_val.value.is_empty() && !is_editing && !is_choice_selecting {
                spans.push(Span::styled(
                    format!(" [{}]", arg_val.choices.join("|")),
                    Style::default().fg(colors.choice),
                ));
            }

            // Collect help text for overlay rendering
            if let Some(ref help) = arg_val.help {
                if !help.is_empty() {
                    help_entries.push((i, build_help_line(help, &ctx, &ps, colors)));
                }
            }

            let line = Line::from(spans);
            let mut item = ListItem::new(line);
            if is_selected {
                item = item.style(selection_bg(is_editing, colors));
            }
            item
        })
        .collect();

    let mut state = ListState::default()
        .with_selected(if ps.is_focused { Some(arg_index) } else { None })
        .with_offset(app.arg_scroll());
    let list = List::new(items).block(block);
    frame.render_stateful_widget(list, area, &mut state);

    // Render right-aligned help text overlays (2 = border + horizontal padding)
    let inner = area.inner(ratatui::layout::Margin::new(1, 1));
    render_help_overlays(frame.buffer_mut(), &help_entries, app.arg_scroll(), inner);
}

/// Render the choice select box as an overlay.
fn render_choice_select(frame: &mut Frame, app: &mut App, terminal_area: Rect, colors: &UiColors) {
    let Some(ref cs) = app.choice_select else {
        return;
    };

    // Extract values before calling filtered_choices (which borrows app)
    let source_panel = cs.source_panel;
    let source_index = cs.source_index;
    let value_column = cs.value_column;
    let selected_index = cs.selected_index;

    let filtered = app.filtered_choices();

    // Determine the panel area and item position
    let panel_area = match source_panel {
        Focus::Flags => app
            .click_regions
            .regions()
            .iter()
            .find(|r| r.data == Focus::Flags)
            .map(|r| r.area),
        Focus::Args => app
            .click_regions
            .regions()
            .iter()
            .find(|r| r.data == Focus::Args)
            .map(|r| r.area),
        _ => None,
    };

    let Some(panel_area) = panel_area else {
        return;
    };

    // Calculate the y position of the item within the panel
    let scroll_offset = match source_panel {
        Focus::Flags => app.flag_scroll(),
        Focus::Args => app.arg_scroll(),
        _ => 0,
    };

    let inner_y = panel_area.y + 1; // skip border
    let item_y = inner_y + (source_index as u16).saturating_sub(scroll_offset as u16);

    // Position the overlay one row BELOW the item
    let overlay_y = item_y + 1;

    // Calculate dimensions
    let max_choice_len = filtered
        .iter()
        .map(|(_, c)| c.chars().count())
        .max()
        .unwrap_or(10) as u16;
    let max_visible = 10u16;
    let visible_count = if filtered.is_empty() {
        1
    } else {
        (filtered.len() as u16).min(max_visible)
    };
    let overlay_height = visible_count + 1; // bottom border only

    // Collect labels and descriptions for the SelectList widget
    let labels: Vec<String> = filtered.iter().map(|(_, c)| c.clone()).collect();
    let descs: Vec<Option<String>> = filtered
        .iter()
        .map(|(orig_idx, _)| app.choice_description(*orig_idx).map(|s| s.to_string()))
        .collect();

    // Account for description width in overlay sizing
    let max_desc_len = descs
        .iter()
        .map(|d| d.as_ref().map(|s| s.chars().count() + 2).unwrap_or(0))
        .max()
        .unwrap_or(0) as u16;
    let overlay_width =
        (max_choice_len + max_desc_len + 4).min(terminal_area.width.saturating_sub(2));

    // Position x one character left of value column to align with text output
    let overlay_x = (panel_area.x + value_column.saturating_sub(1))
        .min(terminal_area.width.saturating_sub(overlay_width));

    // Clamp to terminal bounds
    let overlay_y = overlay_y.min(terminal_area.height.saturating_sub(overlay_height));

    let overlay_rect = Rect::new(overlay_x, overlay_y, overlay_width, overlay_height);

    // Store overlay_rect for mouse hit-testing
    if let Some(ref mut cs) = app.choice_select {
        cs.overlay_rect = Some(overlay_rect);
    }

    let widget = SelectList::new(
        String::new(),
        &labels,
        selected_index,
        colors.choice,
        colors.choice,
        colors,
    )
    .with_descriptions(&descs)
    .with_borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM);
    frame.render_widget(widget, overlay_rect);
}

/// Render the theme picker overlay, positioned above the help bar, right-aligned.
fn render_theme_picker(frame: &mut Frame, app: &mut App, terminal_area: Rect, colors: &UiColors) {
    let Some(ref tp) = app.theme_picker else {
        return;
    };
    let selected_index = tp.selected_index;
    let all = ThemeName::all();

    // Dimensions
    let max_name_len = all
        .iter()
        .map(|t| t.display_name().len())
        .max()
        .unwrap_or(10) as u16;
    let overlay_width = max_name_len + 6; // "▶ " prefix (2) + padding (2) + borders (2)
    let num_themes = all.len() as u16;
    let overlay_height = (num_themes + 2).min(terminal_area.height.saturating_sub(2)); // +2 for borders

    // Position: right-aligned, above the help bar (last row)
    let overlay_x = terminal_area
        .right()
        .saturating_sub(overlay_width)
        .max(terminal_area.x);
    let help_bar_y = terminal_area.bottom().saturating_sub(1);
    let overlay_y = help_bar_y.saturating_sub(overlay_height).max(terminal_area.y);

    let overlay_rect = Rect::new(overlay_x, overlay_y, overlay_width, overlay_height);

    // Store overlay_rect for mouse hit-testing
    if let Some(ref mut tp) = app.theme_picker {
        tp.overlay_rect = Some(overlay_rect);
    }

    // Collect labels for the SelectList widget
    let labels: Vec<String> = all.iter().map(|t| t.display_name().to_string()).collect();

    let widget = SelectList::new(
        " Theme ".to_string(),
        &labels,
        Some(selected_index),
        colors.choice,
        colors.value,
        colors,
    )
    .with_cursor();
    frame.render_widget(widget, overlay_rect);
}

fn render_help_bar(frame: &mut Frame, app: &mut App, area: Rect, colors: &UiColors) {
    let keybinds = if app.is_theme_picking() {
        "↑↓: navigate  Enter: confirm  Esc: cancel"
    } else if app.is_choosing() {
        "↑↓: select  Enter: confirm  Esc: keep text"
    } else if app.editing {
        "Enter: confirm  Esc: cancel"
    } else if app.filtering {
        "Enter: apply  Esc: clear  ↑↓: navigate"
    } else if app.filter_active() {
        "↑↓/jk: next match  /: new filter  Esc: clear filter"
    } else {
        match app.focus() {
            Focus::Commands => {
                "↑↓: navigate  Tab: next  /: filter  Ctrl+R: run  q: quit"
            }
            Focus::Flags => {
                "Enter/Space: toggle  ↑↓: navigate  Tab: next  /: filter  Ctrl+R: run  q: quit"
            }
            Focus::Args => {
                "Enter: edit  ↑↓: navigate  Tab: next  /: filter  Ctrl+R: run  q: quit"
            }
            Focus::Preview => "Enter: run  Tab: next  q: quit",
        }
    };

    let theme_display = app.theme_name.display_name();
    let widget = HelpBar::new(keybinds, theme_display, colors);

    // Store the theme indicator rect for mouse click detection
    app.theme_indicator_rect = Some(widget.theme_indicator_rect(area));

    frame.render_widget(widget, area);
}

/// Render the command preview bar at the bottom with colorized parts.
fn render_preview(frame: &mut Frame, app: &App, area: Rect, colors: &UiColors) {
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
        app.filtering = true;
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
        app.filtering = true;
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
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();

        // Type "hello"
        app.edit_input.insert_char('h');
        app.edit_input.insert_char('e');
        app.edit_input.insert_char('l');
        app.edit_input.insert_char('l');
        app.edit_input.insert_char('o');

        // Render with cursor at end
        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("hello▎"),
            "Should show cursor at end of text"
        );

        // Move cursor to beginning
        app.edit_input.move_home();
        let output = render_to_string(&mut app, 100, 24);
        assert!(
            output.contains("▎hello"),
            "Should show cursor at beginning of text"
        );

        // Move cursor to middle
        app.edit_input.move_right();
        app.edit_input.move_right();
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
            output.contains("Enter: confirm") && output.contains("Esc: cancel"),
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
        assert!(app.theme_indicator_rect.is_none());
        let _output = render_to_string(&mut app, 100, 24);
        assert!(
            app.theme_indicator_rect.is_some(),
            "theme_indicator_rect should be set after rendering"
        );
    }
}
