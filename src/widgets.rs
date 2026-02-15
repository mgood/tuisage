//! Reusable UI widget helpers and custom widgets for TuiSage panels.
//!
//! These helpers extract common patterns from the command, flag, and argument
//! list renderers to ensure consistent behavior across panels.
//!
//! Custom widgets implement [`ratatui::widgets::Widget`] for self-contained,
//! composable rendering of UI sections like the command preview and help bar.

use std::collections::{HashMap, HashSet};

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Widget, Wrap},
};

use crate::app::{fuzzy_match_indices, App, Focus, MatchScores};

use nucleo_matcher::{Config, Matcher};
use ratatui_themes::ThemePalette;

/// Semantic color palette derived from the active theme.
/// Maps abstract UI roles to concrete `Color` values.
pub struct UiColors {
    pub command: Color,
    pub flag: Color,
    pub arg: Color,
    pub value: Color,
    pub required: Color,
    pub help: Color,
    pub active_border: Color,
    pub inactive_border: Color,
    pub selected_bg: Color,
    pub editing_bg: Color,
    pub preview_cmd: Color,
    pub choice: Color,
    pub default_val: Color,
    pub count: Color,
    pub bg: Color,
    pub bar_bg: Color,
}

impl UiColors {
    pub fn from_palette(p: &ThemePalette) -> Self {
        let bar_bg = match p.bg {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(10),
                g.saturating_add(10),
                b.saturating_add(15),
            ),
            _ => Color::Rgb(30, 30, 40),
        };

        let selected_bg = match p.selection {
            Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
            _ => Color::Rgb(40, 40, 60),
        };

        let editing_bg = match p.selection {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(15),
                g.saturating_sub(5),
                b.saturating_sub(10),
            ),
            _ => Color::Rgb(50, 30, 30),
        };

        Self {
            command: p.info,
            flag: p.warning,
            arg: p.success,
            value: p.accent,
            required: p.error,
            help: p.muted,
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

/// State for computing panel-level styling decisions.
/// Built once per panel per render, then passed to item-level helpers.
pub struct PanelState {
    pub is_focused: bool,
    pub is_filtering: bool,
    pub has_filter: bool,
    pub border_color: Color,
    pub filter_text: String,
    pub match_scores: HashMap<String, MatchScores>,
}

impl PanelState {
    /// Build panel state from the app for a given focus panel.
    pub fn from_app(app: &App, panel: Focus, colors: &UiColors) -> Self {
        let is_focused = app.focus() == panel;
        let is_filtering = app.filtering && app.focus() == panel;
        let has_filter = app.filter_active() && app.focus() == panel;
        let border_color = if is_focused || is_filtering {
            colors.active_border
        } else {
            colors.inactive_border
        };
        let filter_text = if app.focus() == panel {
            app.filter().to_string()
        } else {
            String::new()
        };

        PanelState {
            is_focused,
            is_filtering,
            has_filter,
            border_color,
            filter_text,
            match_scores: HashMap::new(),
        }
    }

    /// Set match scores (call after construction with panel-specific scores).
    pub fn with_scores(mut self, scores: HashMap<String, MatchScores>) -> Self {
        self.match_scores = scores;
        self
    }

    /// Whether any filter is visually active (typing or applied).
    pub fn filter_visible(&self) -> bool {
        self.is_filtering || self.has_filter
    }
}

/// Build the panel title string with optional filter indicator.
pub fn panel_title(name: &str, ps: &PanelState) -> String {
    if ps.filter_visible() {
        format!(" {} 🔍 {} ", name, ps.filter_text)
    } else {
        format!(" {} ", name)
    }
}

/// Build a styled `Block` for a panel with consistent border and title styling.
/// The `with_padding` flag controls whether horizontal padding is added
/// (Flags and Args panels use padding; Commands panel does not).
pub fn panel_block(title: String, ps: &PanelState, with_padding: bool) -> Block<'static> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(ps.border_color))
        .title(title)
        .title_style(Style::default().fg(ps.border_color).bold());
    if with_padding {
        block = block.padding(Padding::horizontal(1));
    }
    block
}

/// Push the selection cursor indicator (`▶ ` or `  `) onto spans.
pub fn push_selection_cursor<'a>(spans: &mut Vec<Span<'a>>, is_selected: bool, colors: &UiColors) {
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
}

/// Compute the highlight styles for an item based on selection and match state.
/// Returns `(normal_style, highlight_style)` for use with `build_highlighted_text`.
fn highlight_styles(
    base_color: Color,
    bg_color: Color,
    is_selected: bool,
) -> (Style, Style) {
    if is_selected {
        (
            Style::default()
                .fg(base_color)
                .add_modifier(Modifier::BOLD),
            Style::default()
                .fg(bg_color)
                .bg(base_color)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            Style::default().fg(base_color),
            Style::default()
                .fg(base_color)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
        )
    }
}

/// Common context for rendering an item within a filtered panel.
/// Groups the match-state fields that are passed to multiple helpers.
pub struct ItemContext {
    pub is_selected: bool,
    pub is_match: bool,
    pub name_matches: bool,
    pub help_matches: bool,
}

impl ItemContext {
    /// Build from match state lookup.
    pub fn new(key: &str, is_selected: bool, ps: &PanelState) -> Self {
        let (is_match, name_matches, help_matches) = item_match_state(key, ps);
        Self {
            is_selected,
            is_match,
            name_matches,
            help_matches,
        }
    }
}

/// Push highlighted (or plain) name text onto spans, applying the correct styling
/// based on selection state, filter match state, and whether this field matched.
///
/// The 4 styling states:
/// 1. Selected + name matches filter → inverted highlight on matched chars
/// 2. Not selected + not matching at all → dimmed
/// 3. Not selected + name matches → bold+underlined on matched chars
/// 4. No filter or matched via other field only → normal (bold if selected)
pub fn push_highlighted_name(
    spans: &mut Vec<Span<'static>>,
    text: &str,
    base_color: Color,
    ctx: &ItemContext,
    ps: &PanelState,
    colors: &UiColors,
) {
    let has_scores = !ps.match_scores.is_empty();

    if ctx.is_selected {
        if has_scores && ctx.name_matches {
            let (normal, highlight) = highlight_styles(base_color, colors.bg, true);
            let highlighted = build_highlighted_text(text, &ps.filter_text, normal, highlight);
            spans.extend(highlighted);
        } else {
            spans.push(Span::styled(
                text.to_string(),
                Style::default()
                    .fg(base_color)
                    .add_modifier(Modifier::BOLD),
            ));
        }
    } else if !ctx.is_match && has_scores {
        // Non-matching item during filtering → dimmed
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(colors.help).add_modifier(Modifier::DIM),
        ));
    } else if has_scores && ctx.name_matches {
        let (normal, highlight) = highlight_styles(base_color, colors.bg, false);
        let highlighted = build_highlighted_text(text, &ps.filter_text, normal, highlight);
        spans.extend(highlighted);
    } else {
        // Normal display (no filter, or matched via help only)
        spans.push(Span::styled(
            text.to_string(),
            Style::default().fg(base_color),
        ));
    }
}

/// Push right-aligned help text onto spans with optional filter highlighting.
/// Handles padding calculation, dim for non-matches, and character highlighting.
pub fn push_help_text(
    spans: &mut Vec<Span<'static>>,
    help: &str,
    available_width: usize,
    ctx: &ItemContext,
    ps: &PanelState,
    colors: &UiColors,
) {
    // Calculate padding based on current span width
    let current_len: usize = spans.iter().map(|s| s.content.chars().count()).sum();

    if current_len + help.chars().count() + 1 < available_width {
        let padding = available_width.saturating_sub(current_len + help.chars().count());
        spans.push(Span::raw(" ".repeat(padding)));
    } else {
        spans.push(Span::raw(" "));
    }

    let has_scores = !ps.match_scores.is_empty();

    if !ctx.is_match && has_scores {
        // Non-matching → dim help text
        spans.push(Span::styled(
            help.to_string(),
            Style::default().fg(colors.help).add_modifier(Modifier::DIM),
        ));
    } else if has_scores && ctx.help_matches {
        // Help text matches the filter → highlight matched characters
        let (normal, highlight) = highlight_styles(colors.help, colors.bg, ctx.is_selected);
        let highlighted = build_highlighted_text(help, &ps.filter_text, normal, highlight);
        spans.extend(highlighted);
    } else {
        spans.push(Span::styled(help.to_string(), Style::default().fg(colors.help)));
    }
}

/// Push inline edit cursor spans (before_cursor + ▎ + after_cursor).
pub fn push_edit_cursor(
    spans: &mut Vec<Span<'static>>,
    before_cursor: &str,
    after_cursor: &str,
    colors: &UiColors,
) {
    spans.push(Span::styled(
        before_cursor.to_string(),
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
    spans.push(Span::styled(
        after_cursor.to_string(),
        Style::default()
            .fg(colors.value)
            .add_modifier(Modifier::UNDERLINED),
    ));
}

/// Return the background style for a selected item (editing vs normal selection).
pub fn selection_bg(is_editing: bool, colors: &UiColors) -> Style {
    let bg = if is_editing {
        colors.editing_bg
    } else {
        colors.selected_bg
    };
    Style::default().bg(bg)
}

/// Look up per-field match scores for a given item key.
/// Returns `(is_match, name_matches, help_matches)`.
pub fn item_match_state(key: &str, ps: &PanelState) -> (bool, bool, bool) {
    let scores = ps.match_scores.get(key);
    let is_match = scores.map(|s| s.overall()).unwrap_or(1) > 0 || ps.match_scores.is_empty();
    let name_matches = scores.map(|s| s.name_score).unwrap_or(0) > 0;
    let help_matches = scores.map(|s| s.help_score).unwrap_or(0) > 0;
    (is_match, name_matches, help_matches)
}

/// Build spans with highlighted characters based on fuzzy match indices.
pub fn build_highlighted_text(
    text: &str,
    pattern: &str,
    normal_style: Style,
    highlight_style: Style,
) -> Vec<Span<'static>> {
    let mut matcher = Matcher::new(Config::DEFAULT);
    let (_score, indices) = fuzzy_match_indices(text, pattern, &mut matcher);

    if indices.is_empty() {
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

        if last_idx < idx {
            let before: String = chars[last_idx..idx].iter().collect();
            if !before.is_empty() {
                spans.push(Span::styled(before, normal_style));
            }
        }

        spans.push(Span::styled(chars[idx].to_string(), highlight_style));
        last_idx = idx + 1;
    }

    if last_idx < chars.len() {
        let after: String = chars[last_idx..].iter().collect();
        if !after.is_empty() {
            spans.push(Span::styled(after, normal_style));
        }
    }

    spans
}

// ── Custom Widgets ──────────────────────────────────────────────────

/// A widget that renders the assembled command preview with colorized tokens.
///
/// Displays the command string in a bordered block, with syntax-aware coloring
/// for the binary name, subcommands, flags, and positional arguments.
pub struct CommandPreview<'a> {
    /// The full built command string.
    pub command: &'a str,
    /// Binary name from the spec.
    pub bin: &'a str,
    /// Subcommand names in the current command path.
    pub subcommands: &'a [String],
    /// Whether the preview panel currently has focus.
    pub is_focused: bool,
    pub colors: &'a UiColors,
}

impl<'a> CommandPreview<'a> {
    pub fn new(
        command: &'a str,
        bin: &'a str,
        subcommands: &'a [String],
        is_focused: bool,
        colors: &'a UiColors,
    ) -> Self {
        Self {
            command,
            bin,
            subcommands,
            is_focused,
            colors,
        }
    }

    /// Colorize the command string by categorizing each token.
    fn colorize(&self, bold: Modifier) -> Vec<Span<'static>> {
        let subcommand_names: HashSet<&str> =
            self.subcommands.iter().map(|s| s.as_str()).collect();

        let tokens: Vec<&str> = self.command.split_whitespace().collect();
        let mut spans = Vec::new();
        let mut i = 0;
        let mut expect_flag_value = false;

        while i < tokens.len() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }

            let token = tokens[i];

            if i == 0 && token == self.bin {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default()
                        .fg(self.colors.preview_cmd)
                        .add_modifier(bold | Modifier::BOLD),
                ));
            } else if expect_flag_value {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.value).add_modifier(bold),
                ));
                expect_flag_value = false;
            } else if token.starts_with('-') {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.flag).add_modifier(bold),
                ));
                if let Some(&next) = tokens.get(i + 1) {
                    if !next.starts_with('-') && !subcommand_names.contains(next) {
                        expect_flag_value = true;
                    }
                }
            } else if subcommand_names.contains(token) {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.command).add_modifier(bold),
                ));
            } else {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.arg).add_modifier(bold),
                ));
            }

            i += 1;
        }

        spans
    }
}

impl Widget for CommandPreview<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_color = if self.is_focused {
            self.colors.active_border
        } else {
            self.colors.inactive_border
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Command ")
            .title_style(Style::default().fg(border_color).bold())
            .padding(Padding::horizontal(1));

        let prefix = if self.is_focused { "▶ " } else { "$ " };
        let bold = if self.is_focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };

        let mut spans = vec![Span::styled(prefix, Style::default().fg(self.colors.command))];
        spans.extend(self.colorize(bold));

        let paragraph = Paragraph::new(Line::from(spans))
            .block(block)
            .wrap(Wrap { trim: false });

        paragraph.render(area, buf);
    }
}

/// A widget that renders the context-sensitive help/status bar.
///
/// Shows keyboard shortcuts for the current mode on the left and
/// the active theme indicator on the right.
pub struct HelpBar<'a> {
    /// The help text string to display.
    pub keybinds: &'a str,
    /// Theme display name for the right-aligned indicator.
    pub theme_display: &'a str,
    pub colors: &'a UiColors,
}

impl<'a> HelpBar<'a> {
    pub fn new(keybinds: &'a str, theme_display: &'a str, colors: &'a UiColors) -> Self {
        Self {
            keybinds,
            theme_display,
            colors,
        }
    }

    /// Returns the Rect where the theme indicator is rendered,
    /// for use in mouse click hit-testing.
    pub fn theme_indicator_rect(&self, area: Rect) -> Rect {
        let theme_indicator = format!("T: [{}] ", self.theme_display);
        let theme_indicator_len = theme_indicator.len() as u16;
        let indicator_x = area.x + area.width.saturating_sub(theme_indicator_len);
        Rect::new(indicator_x, area.y, theme_indicator_len, 1)
    }
}

impl Widget for HelpBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme_indicator = format!("T: [{}] ", self.theme_display);
        let theme_indicator_len = theme_indicator.len() as u16;

        let keybinds_text = format!(" {}", self.keybinds);
        let keybinds_len = keybinds_text.chars().count() as u16;
        let padding_len = area.width.saturating_sub(keybinds_len + theme_indicator_len);
        let padding = " ".repeat(padding_len as usize);

        let paragraph = Paragraph::new(Line::from(vec![
            Span::styled(keybinds_text, Style::default().fg(self.colors.help)),
            Span::styled(padding, Style::default()),
            Span::styled(
                theme_indicator,
                Style::default().fg(self.colors.active_border).italic(),
            ),
        ]))
        .style(Style::default().bg(self.colors.bar_bg));

        paragraph.render(area, buf);
    }
}

/// A widget that renders a bordered selectable list overlay.
///
/// Used for both the choice select dropdown and the theme picker.
/// Clears the area behind it, draws a bordered block with a title,
/// and renders selectable items with a `▶` cursor indicator.
/// Optionally shows descriptions alongside items.
pub struct SelectList<'a> {
    /// Title shown in the block border (empty string = no title).
    pub title: String,
    /// Item labels to display.
    pub items: &'a [String],
    /// Optional descriptions for each item (parallel to `items`).
    pub descriptions: &'a [Option<String>],
    /// Currently selected index (None = no selection).
    pub selected: Option<usize>,
    /// Whether to show the ▶ selection cursor prefix.
    pub show_cursor: bool,
    /// Which borders to show (defaults to ALL).
    pub borders: Borders,
    /// Style for unselected items.
    pub item_color: Color,
    /// Style for the selected item (text color).
    pub selected_color: Color,
    pub colors: &'a UiColors,
}

impl<'a> SelectList<'a> {
    pub fn new(
        title: String,
        items: &'a [String],
        selected: Option<usize>,
        item_color: Color,
        selected_color: Color,
        colors: &'a UiColors,
    ) -> Self {
        Self {
            title,
            items,
            descriptions: &[],
            selected,
            show_cursor: false,
            borders: Borders::ALL,
            item_color,
            selected_color,
            colors,
        }
    }

    /// Set descriptions to display alongside items.
    pub fn with_descriptions(mut self, descriptions: &'a [Option<String>]) -> Self {
        self.descriptions = descriptions;
        self
    }

    /// Show ▶ prefix cursor for the selected item.
    pub fn with_cursor(mut self) -> Self {
        self.show_cursor = true;
        self
    }

    /// Set which borders to show.
    pub fn with_borders(mut self, borders: Borders) -> Self {
        self.borders = borders;
        self
    }
}

impl Widget for SelectList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        // Clear area behind the overlay
        ratatui::widgets::Clear.render(area, buf);

        let mut block = Block::default()
            .borders(self.borders)
            .border_style(Style::default().fg(self.colors.active_border));
        if !self.title.is_empty() {
            block = block
                .title(self.title)
                .title_style(
                    Style::default()
                        .fg(self.colors.active_border)
                        .add_modifier(Modifier::BOLD),
                );
        }

        let items: Vec<ratatui::widgets::ListItem> = if self.items.is_empty() {
            vec![ratatui::widgets::ListItem::new(Line::from(Span::styled(
                "(no matches)",
                Style::default().fg(self.colors.help).italic(),
            )))]
        } else {
            self.items
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let is_selected = self.selected == Some(i);
                    let style = if is_selected {
                        Style::default()
                            .fg(self.selected_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.item_color)
                    };

                    let mut spans = Vec::new();
                    if self.show_cursor {
                        let prefix = if is_selected { "▶ " } else { "  " };
                        spans.push(Span::styled(
                            prefix,
                            if is_selected {
                                Style::default()
                                    .fg(self.colors.active_border)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ));
                    }
                    spans.push(Span::styled(label.clone(), style));

                    // Add description if present
                    if let Some(Some(desc)) = self.descriptions.get(i) {
                        spans.push(Span::styled(
                            format!("  {}", desc),
                            Style::default().fg(self.colors.help),
                        ));
                    }

                    let mut item = ratatui::widgets::ListItem::new(Line::from(spans));
                    if is_selected {
                        item = item.style(Style::default().bg(self.colors.selected_bg));
                    }
                    item
                })
                .collect()
        };

        let visible_items = area.height.saturating_sub(2) as usize;
        let mut state = ratatui::widgets::ListState::default().with_selected(
            if self.items.is_empty() {
                None
            } else {
                self.selected
            },
        );

        if let Some(sel) = self.selected {
            if sel >= visible_items {
                state = state.with_offset(sel.saturating_sub(visible_items - 1));
            }
        }

        let list = ratatui::widgets::List::new(items).block(block);
        ratatui::widgets::StatefulWidget::render(list, area, buf, &mut state);
    }
}
