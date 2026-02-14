//! Reusable UI widget helpers for TuiSage panels.
//!
//! These helpers extract common patterns from the command, flag, and argument
//! list renderers to ensure consistent behavior across panels.

use std::collections::HashMap;

use ratatui::{
    style::{Color, Modifier, Style},
    text::Span,
    widgets::{Block, Borders, Padding},
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
