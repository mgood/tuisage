//! Shared component types and panel rendering helpers.
//!
//! This module provides the building blocks used by all panel components:
//! `PanelState` for per-panel context, item highlighting helpers,
//! and common rendering utilities.
//!
//! Standalone widgets:
//! - [`preview`] — Command preview display
//! - [`help_bar`] — Context-sensitive keyboard shortcuts
//! - [`select_list`] — Bordered selectable list overlay
//!
//! Stateful components (own state + event handling):
//! - [`command_panel`] — Command tree panel
//! - [`flag_panel`] — Flag list panel with choice select
//! - [`arg_panel`] — Argument list panel with choice select
//! - [`choice_select`] — Filtered choice selection overlay
//! - [`theme_picker`] — Theme picker overlay
//! - [`execution`] — Embedded terminal for command execution

pub mod arg_panel;
pub mod choice_select;
pub mod command_panel;
pub mod execution;
pub mod filterable;
pub mod flag_panel;
pub mod help_bar;
pub mod list_panel_base;
pub mod preview;
pub mod select_list;
pub mod theme_picker;

use std::collections::HashMap;

use crossterm::event::{KeyEvent, MouseEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget},
};

use nucleo_matcher::{Config, Matcher};

use crate::app::{fuzzy_match_indices, MatchScores};
use crate::theme::UiColors;

// ── Overlay support ─────────────────────────────────────────────────

/// Content that can be rendered as an overlay above all other UI.
///
/// Implemented by overlay components (e.g. ChoiceSelectComponent,
/// ThemePickerComponent). The coordinator calls `render()` after
/// computing the final clamped position.
pub trait OverlayContent {
    /// Render this overlay's content into the given area.
    /// Signature matches RenderableComponent::render — colors provided by coordinator.
    fn render(&self, area: Rect, buf: &mut Buffer, colors: &UiColors);
}

/// Describes a pending overlay that a component wants to render.
/// The component provides the content; the coordinator handles positioning.
pub struct OverlayRequest {
    /// Where to anchor the overlay relative to the viewport.
    pub anchor: Rect,
    /// Preferred size (width, height). Will be clamped to viewport.
    pub size: (u16, u16),
    /// The overlay content — renders into the final computed area.
    pub content: Box<dyn OverlayContent>,
}

/// Clamp an overlay's preferred position and size to fit within the viewport.
/// Positions the overlay just below the anchor and shrinks height to fit the
/// available space below rather than pushing the overlay upward.
pub fn clamp_overlay(anchor: Rect, size: (u16, u16), viewport: Rect) -> Rect {
    let (pref_w, pref_h) = size;
    let w = pref_w.min(viewport.width);

    // Place just below the anchor
    let y = anchor.y + anchor.height;

    // Shrink height to fit available space below (minimum 2 rows for usability)
    let space_below = viewport.bottom().saturating_sub(y);
    let h = pref_h.min(space_below).max(2);

    let x = anchor.x.min(viewport.right().saturating_sub(w)).max(viewport.x);
    let y = y.min(viewport.bottom().saturating_sub(h)).max(viewport.y);

    Rect::new(x, y, w, h)
}

// ── Component trait ─────────────────────────────────────────────────

/// Result of handling an event. Generic over the action type `A`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventResult<A> {
    /// Event was consumed, no further action needed.
    Consumed,
    /// Event was not handled — bubble up to parent.
    NotHandled,
    /// Event produced an action the parent should handle.
    Action(A),
}

impl<A> EventResult<A> {
    /// Transform the action type, preserving Consumed/NotHandled.
    #[allow(dead_code)]
    pub fn map<B>(self, f: impl FnOnce(A) -> B) -> EventResult<B> {
        match self {
            EventResult::Consumed => EventResult::Consumed,
            EventResult::NotHandled => EventResult::NotHandled,
            EventResult::Action(a) => EventResult::Action(f(a)),
        }
    }

    /// Transform the action into a different EventResult.
    #[allow(dead_code)]
    pub fn and_then<B>(self, f: impl FnOnce(A) -> EventResult<B>) -> EventResult<B> {
        match self {
            EventResult::Consumed => EventResult::Consumed,
            EventResult::NotHandled => EventResult::NotHandled,
            EventResult::Action(a) => f(a),
        }
    }
}

/// A UI component that owns its interaction state and event handling.
///
/// Each component defines its own `Action` type — only the actions it can
/// actually produce. The parent maps child actions to its own context.
pub trait Component {
    /// The type of actions this component can emit.
    type Action;

    /// Notify the component that it gained focus.
    fn handle_focus_gained(&mut self) -> EventResult<Self::Action> {
        EventResult::NotHandled
    }

    /// Notify the component that it lost focus.
    fn handle_focus_lost(&mut self) -> EventResult<Self::Action> {
        EventResult::NotHandled
    }

    /// Handle a keyboard event.
    fn handle_key(&mut self, key: KeyEvent) -> EventResult<Self::Action>;

    /// Handle a mouse event within the given area.
    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> EventResult<Self::Action>;

    /// Collect any overlays that should be rendered above all other content.
    /// Called after render(). Wrappers collect from their children and
    /// optionally append their own. Default: no overlays.
    fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        Vec::new()
    }
}

/// A component that can render its own primary content area.
pub trait RenderableComponent: Component {
    /// Render into the given buffer area.
    fn render(&mut self, area: Rect, buf: &mut Buffer, colors: &UiColors);
}

// ── PanelState ──────────────────────────────────────────────────────

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
    /// Whether any filter is visually active (typing or applied).
    pub fn filter_visible(&self) -> bool {
        self.is_filtering || self.has_filter
    }
}

// ── Panel chrome helpers ────────────────────────────────────────────

/// Build the panel title string with optional filter indicator.
pub fn panel_title(name: &str, ps: &PanelState) -> String {
    if ps.filter_visible() {
        format!(" {} 🔍 {} ", name, ps.filter_text)
    } else {
        format!(" {} ", name)
    }
}

/// Build a styled `Block` for a panel with consistent border and title styling.
pub fn panel_block(title: String, ps: &PanelState) -> ratatui::widgets::Block<'static> {
    ratatui::widgets::Block::default()
        .borders(ratatui::widgets::Borders::ALL)
        .border_style(Style::default().fg(ps.border_color))
        .title(title)
        .title_style(Style::default().fg(ps.border_color).bold())
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

// ── Item context & highlighting ─────────────────────────────────────

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

/// Push highlighted (or plain) name text onto spans, applying the correct styling
/// based on selection state, filter match state, and whether this field matched.
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
/// Build a styled `Line` for help text, suitable for right-aligned overlay rendering.
pub fn build_help_line(
    help: &str,
    ctx: &ItemContext,
    ps: &PanelState,
    colors: &UiColors,
) -> Line<'static> {
    let has_scores = !ps.match_scores.is_empty();

    let spans = if !ctx.is_match && has_scores {
        // Non-matching → dim help text
        vec![Span::styled(
            help.to_string(),
            Style::default().fg(colors.help).add_modifier(Modifier::DIM),
        )]
    } else if has_scores && ctx.help_matches {
        // Help text matches the filter → highlight matched characters
        let (normal, highlight) = highlight_styles(colors.help, colors.bg, ctx.is_selected);
        build_highlighted_text(help, &ps.filter_text, normal, highlight)
    } else {
        vec![Span::styled(
            help.to_string(),
            Style::default().fg(colors.help),
        )]
    };

    Line::from(spans)
}

/// Render right-aligned help text overlays on top of a rendered List.
pub fn render_help_overlays(
    buf: &mut Buffer,
    help_entries: &[(usize, Line<'static>)],
    scroll_offset: usize,
    inner: Rect,
) {
    let visible_rows = inner.height as usize;
    if inner.width < 2 {
        return;
    }

    for &(item_idx, ref help_line) in help_entries {
        // Skip items above the scroll viewport
        if item_idx < scroll_offset {
            continue;
        }
        let row_in_view = item_idx - scroll_offset;
        if row_in_view >= visible_rows {
            continue;
        }

        let y = inner.y + row_in_view as u16;
        let help_width = help_line.width() as u16;
        if help_width == 0 || help_width >= inner.width {
            continue;
        }

        // Position the help text right-aligned with a 1-char gap before it.
        let total_width = help_width + 1;
        if total_width >= inner.width {
            continue;
        }
        let help_x = inner.x + inner.width - total_width;

        // Find rightmost non-empty cell in this row to avoid overwriting content
        let mut content_end = inner.x;
        for x in (inner.x..inner.x + inner.width).rev() {
            let cell = &buf[(x, y)];
            if cell.symbol() != " " {
                content_end = x + 1;
                break;
            }
        }
        // Skip if help text would overlap existing content
        if help_x < content_end {
            continue;
        }

        let help_rect = Rect::new(help_x, y, total_width, 1);
        let spaced_line = Line::from(
            std::iter::once(Span::raw(" "))
                .chain(help_line.spans.iter().cloned())
                .collect::<Vec<_>>(),
        );
        let para = Paragraph::new(spaced_line);
        ratatui::widgets::Widget::render(para, help_rect, buf);
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

// ── Scrollbar ───────────────────────────────────────────────────────

/// Render a vertical scrollbar for a panel.
///
/// Shared by all panel components — renders only when content overflows.
pub fn render_panel_scrollbar(
    buf: &mut Buffer,
    area: Rect,
    total_items: usize,
    scroll_offset: usize,
    colors: &UiColors,
) {
    let inner_height = area.height.saturating_sub(2) as usize;
    if total_items <= inner_height || inner_height == 0 {
        return;
    }
    let inner = area.inner(ratatui::layout::Margin::new(0, 1));
    let mut scrollbar_state =
        ScrollbarState::new(total_items.saturating_sub(inner_height)).position(scroll_offset);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(Some("│"))
        .thumb_symbol("┃")
        .track_style(Style::default().fg(colors.inactive_border))
        .thumb_style(Style::default().fg(colors.active_border));
    StatefulWidget::render(scrollbar, inner, buf, &mut scrollbar_state);
}

// ── Shared filter-navigation helpers ────────────────────────────────

/// Find the next or previous matching item in a scored list, wrapping around.
/// `keys` are the score lookup keys for each item.
/// `forward` controls the search direction.
/// Returns the index of the match, or `None` if no match is found.
pub(crate) fn find_adjacent_match(
    keys: &[String],
    scores: &HashMap<String, MatchScores>,
    current: usize,
    forward: bool,
) -> Option<usize> {
    let total = keys.len();
    if total == 0 {
        return None;
    }
    for offset in 1..total {
        let idx = if forward {
            (current + offset) % total
        } else {
            (current + total - offset) % total
        };
        if let Some(key) = keys.get(idx) {
            if scores.get(key).map(|s| s.overall()).unwrap_or(0) > 0 {
                return Some(idx);
            }
        }
    }
    None
}

/// Find the first matching item in a scored list, keeping the current selection
/// if it already matches. Returns the current index if it matches, or the first
/// matching index, or `None` if nothing matches.
pub(crate) fn find_first_match(
    keys: &[String],
    scores: &HashMap<String, MatchScores>,
    current: usize,
) -> Option<usize> {
    if let Some(key) = keys.get(current) {
        if scores.get(key).map(|s| s.overall()).unwrap_or(0) > 0 {
            return Some(current);
        }
    }
    for (idx, key) in keys.iter().enumerate() {
        if scores.get(key).map(|s| s.overall()).unwrap_or(0) > 0 {
            return Some(idx);
        }
    }
    None
}
