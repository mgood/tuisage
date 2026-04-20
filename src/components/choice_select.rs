//! Self-contained choice select overlay component.
//!
//! Manages its own open/close lifecycle, selection, filtering, scroll,
//! and text input. Reports [`ChoiceSelectAction::Selected`] or
//! [`ChoiceSelectAction::Cancelled`] — the parent interprets the result.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use nucleo_matcher::{Config, Matcher};
use ratatui::{buffer::Buffer, layout::Rect, widgets::Borders};
use ratatui_interact::components::InputState;

use crate::app::fuzzy_match_score;
use crate::theme::UiColors;

use super::select_list::{SelectList, SelectListScrollState};
use super::{Component, EventResult, OverlayContent, OverlayRequest};

/// Compute the scroll offset for a list with the given selected index and
/// visible item count. Matches the formula in `SelectList::render()`.
fn compute_scroll_offset(selected: Option<usize>, visible_items: usize) -> usize {
    if let Some(sel) = selected {
        if visible_items > 0 && sel >= visible_items {
            sel.saturating_sub(visible_items - 1)
        } else {
            0
        }
    } else {
        0
    }
}

// ── Actions ────────────────────────────────────────────────────────

/// Actions emitted by the choice select component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChoiceSelectAction {
    /// User confirmed a selection. Contains the chosen text.
    Selected(String),
    /// User cancelled (Esc). Contains the typed text to keep.
    Cancelled(String),
}

// ── Inner state ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct ChoiceSelectInner {
    choices: Vec<String>,
    descriptions: Vec<Option<String>>,
    selected_index: Option<usize>,
    filter_active: bool,
    edit_input: InputState,
    /// Anchor point for overlay positioning (set by parent panel).
    anchor: Rect,
}

impl ChoiceSelectInner {
    /// Get filtered choices as (original_index, label) pairs.
    fn filtered_choices(&self) -> Vec<(usize, String)> {
        let filter = self.edit_input.text();
        if filter.is_empty() || !self.filter_active {
            return self
                .choices
                .iter()
                .enumerate()
                .map(|(i, c)| (i, c.clone()))
                .collect();
        }
        let mut matcher = Matcher::new(Config::DEFAULT);
        self.choices
            .iter()
            .enumerate()
            .filter(|(_, c)| fuzzy_match_score(c, filter, &mut matcher) > 0)
            .map(|(i, c)| (i, c.clone()))
            .collect()
    }

    /// Resolve the value to emit: selected choice text or typed input.
    fn resolve_value(&self) -> String {
        let filtered = self.filtered_choices();
        if let Some(idx) = self.selected_index {
            if let Some((_, text)) = filtered.get(idx) {
                return text.clone();
            }
        }
        self.edit_input.text().to_string()
    }

    fn description(&self, original_index: usize) -> Option<&str> {
        self.descriptions
            .get(original_index)
            .and_then(|d| d.as_deref())
    }
}

// ── Component ──────────────────────────────────────────────────────

/// Self-contained choice/completion select overlay.
/// Owns its own open/close state, selection index, scroll, and filter.
#[derive(Debug, Clone, Default)]
pub struct ChoiceSelectComponent {
    state: Option<ChoiceSelectInner>,
    /// Mouse position from parent, used for hover highlighting.
    mouse_position: Option<(u16, u16)>,
}

impl ChoiceSelectComponent {
    pub fn new() -> Self {
        Self {
            state: None,
            mouse_position: None,
        }
    }

    /// Open with the given choices, pre-selecting the current value.
    pub fn open(&mut self, choices: Vec<String>, current_value: &str, anchor: Rect) {
        let selected_index = choices.iter().position(|c| c == current_value);
        let mut edit_input = InputState::empty();
        edit_input.set_text(current_value.to_string());
        self.state = Some(ChoiceSelectInner {
            choices,
            descriptions: Vec::new(),
            selected_index,
            filter_active: false,
            edit_input,
            anchor,
        });
    }

    /// Open with choices and descriptions (for completions).
    pub fn open_with_descriptions(
        &mut self,
        choices: Vec<String>,
        descriptions: Vec<Option<String>>,
        current_value: &str,
        anchor: Rect,
    ) {
        self.open(choices, current_value, anchor);
        if let Some(ref mut inner) = self.state {
            inner.descriptions = descriptions;
        }
    }

    pub fn is_open(&self) -> bool {
        self.state.is_some()
    }

    pub fn close(&mut self) {
        self.state = None;
    }

    /// The current typed text (for syncing to the parent's edit state).
    pub fn typed_text(&self) -> &str {
        self.state
            .as_ref()
            .map(|s| s.edit_input.text())
            .unwrap_or("")
    }

    /// The text before the cursor (for inline edit rendering).
    pub fn text_before_cursor(&self) -> &str {
        self.state
            .as_ref()
            .map(|s| s.edit_input.text_before_cursor())
            .unwrap_or("")
    }

    /// The text after the cursor (for inline edit rendering).
    pub fn text_after_cursor(&self) -> &str {
        self.state
            .as_ref()
            .map(|s| s.edit_input.text_after_cursor())
            .unwrap_or("")
    }

    /// Compute the scroll offset for a given number of visible items.
    /// This derives the offset from the current `selected_index` and the
    /// viewport size, matching the formula used by `SelectList::render()`.
    pub fn compute_scroll_offset(&self, visible_items: usize) -> usize {
        compute_scroll_offset(
            self.state.as_ref().and_then(|s| s.selected_index),
            visible_items,
        )
    }

    /// Current selected index in the filtered list (None = text input mode).
    #[cfg(test)]
    pub fn selected_index(&self) -> Option<usize> {
        self.state.as_ref().and_then(|s| s.selected_index)
    }

    /// Filtered choices as (original_index, label) pairs.
    pub fn filtered_choices(&self) -> Vec<(usize, String)> {
        self.state
            .as_ref()
            .map(|s| s.filtered_choices())
            .unwrap_or_default()
    }

    /// Click-select: set selected index and confirm.
    #[allow(dead_code)] // used by mouse handling
    pub fn click_select(&mut self, index: usize) -> Option<ChoiceSelectAction> {
        let inner = self.state.as_mut()?;
        let filtered = inner.filtered_choices();
        if index < filtered.len() {
            inner.selected_index = Some(index);
            let value = inner.resolve_value();
            self.state = None;
            Some(ChoiceSelectAction::Selected(value))
        } else {
            None
        }
    }

    /// Update the anchor rect for overlay positioning.
    /// Called during rendering when the actual panel area is known.
    pub fn set_anchor(&mut self, anchor: Rect) {
        if let Some(ref mut inner) = self.state {
            inner.anchor = anchor;
        }
    }

    /// Update the mouse position for hover highlighting in the overlay.
    pub fn set_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.mouse_position = pos;
    }
}

impl Component for ChoiceSelectComponent {
    type Action = ChoiceSelectAction;

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<ChoiceSelectAction> {
        let Some(ref mut inner) = self.state else {
            return EventResult::NotHandled;
        };

        match key.code {
            KeyCode::Enter => {
                let value = inner.resolve_value();
                self.state = None;
                EventResult::Action(ChoiceSelectAction::Selected(value))
            }
            KeyCode::Esc => {
                let typed = inner.edit_input.text().to_string();
                self.state = None;
                EventResult::Action(ChoiceSelectAction::Cancelled(typed))
            }
            KeyCode::Up => {
                match inner.selected_index {
                    Some(0) | None => inner.selected_index = None,
                    Some(idx) => inner.selected_index = Some(idx - 1),
                }
                EventResult::Consumed
            }
            KeyCode::Down => {
                let filtered_len = inner.filtered_choices().len();
                if filtered_len > 0 {
                    match inner.selected_index {
                        None => inner.selected_index = Some(0),
                        Some(idx) if idx + 1 < filtered_len => {
                            inner.selected_index = Some(idx + 1);
                        }
                        _ => {}
                    }
                }
                EventResult::Consumed
            }
            KeyCode::Backspace => {
                inner.edit_input.delete_char_backward();
                inner.filter_active = true;
                inner.selected_index = None;
                EventResult::Consumed
            }
            KeyCode::Left => {
                inner.edit_input.move_left();
                EventResult::Consumed
            }
            KeyCode::Right => {
                inner.edit_input.move_right();
                EventResult::Consumed
            }
            KeyCode::Char(c) => {
                inner.edit_input.insert_char(c);
                inner.filter_active = true;
                inner.selected_index = None;
                EventResult::Consumed
            }
            _ => EventResult::NotHandled,
        }
    }

    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect) -> EventResult<ChoiceSelectAction> {
        EventResult::NotHandled
    }

    fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        let Some(ref inner) = self.state else {
            return Vec::new();
        };

        let filtered = inner.filtered_choices();
        let anchor = inner.anchor;

        // Compute preferred size
        let max_choice_len = filtered
            .iter()
            .map(|(_, c)| c.chars().count())
            .max()
            .unwrap_or(10) as u16;

        let descriptions: Vec<Option<String>> = filtered
            .iter()
            .map(|(orig_idx, _)| inner.description(*orig_idx).map(|s| s.to_string()))
            .collect();

        let max_desc_len = descriptions
            .iter()
            .map(|d| d.as_ref().map(|s| s.chars().count() + 2).unwrap_or(0))
            .max()
            .unwrap_or(0) as u16;

        let width = max_choice_len + max_desc_len + 4;
        let max_visible = 10u16;
        let height = if filtered.is_empty() {
            2 // "(no matches)" + bottom border
        } else {
            (filtered.len() as u16).min(max_visible) + 1 // items + bottom border
        };

        // Build the snapshot of data needed to render
        let labels: Vec<String> = filtered.iter().map(|(_, c)| c.clone()).collect();
        let selected_index = inner.selected_index;

        vec![OverlayRequest {
            anchor,
            size: (width, height),
            content: Box::new(ChoiceSelectOverlay {
                labels,
                descriptions,
                selected_index,
                mouse_position: self.mouse_position,
            }),
        }]
    }
}

// ── OverlayContent implementation ──────────────────────────────────

/// Snapshot of choice select data needed to render the overlay.
/// Created during collect_overlays(), rendered by the coordinator.
struct ChoiceSelectOverlay {
    labels: Vec<String>,
    descriptions: Vec<Option<String>>,
    selected_index: Option<usize>,
    mouse_position: Option<(u16, u16)>,
}

impl ChoiceSelectOverlay {
    /// Compute hovered item index from mouse position relative to the overlay area.
    fn hovered_index(&self, area: Rect) -> Option<usize> {
        let (col, row) = self.mouse_position?;
        let inner_top = area.y; // no top border
        let inner_bottom = area.y + area.height.saturating_sub(1);
        if col >= area.x
            && col < area.x + area.width
            && row >= inner_top
            && row < inner_bottom
        {
            let visible_items = area.height.saturating_sub(1) as usize; // minus bottom border
            let scroll = compute_scroll_offset(self.selected_index, visible_items);
            let idx = (row - inner_top) as usize + scroll;
            if idx < self.labels.len() {
                Some(idx)
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl OverlayContent for ChoiceSelectOverlay {
    fn render(&self, area: Rect, buf: &mut Buffer, colors: &UiColors) {
        let hovered = self.hovered_index(area);
        let widget = SelectList::new(
            String::new(),
            &self.labels,
            self.selected_index,
            colors.choice,
            colors.choice,
            colors,
        )
        .with_descriptions(&self.descriptions)
        .with_borders(Borders::LEFT | Borders::RIGHT | Borders::BOTTOM)
        .with_hovered(hovered);

        let mut scroll_state = SelectListScrollState::default();
        ratatui::widgets::StatefulWidget::render(widget, area, buf, &mut scroll_state);
    }
}

// ── Tests ──────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_choices() -> Vec<String> {
        vec![
            "alpha".to_string(),
            "beta".to_string(),
            "gamma".to_string(),
            "delta".to_string(),
        ]
    }

    #[test]
    fn test_open_close_lifecycle() {
        let mut cs = ChoiceSelectComponent::new();
        assert!(!cs.is_open());

        cs.open(make_choices(), "beta", Rect::new(0, 0, 20, 1));
        assert!(cs.is_open());

        cs.close();
        assert!(!cs.is_open());
    }

    #[test]
    fn test_open_preselects_matching_value() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "gamma", Rect::new(0, 0, 20, 1));

        // Should have pre-selected index 2 (gamma)
        let inner = cs.state.as_ref().unwrap();
        assert_eq!(inner.selected_index, Some(2));
    }

    #[test]
    fn test_open_no_match_preselects_none() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "nonexistent", Rect::new(0, 0, 20, 1));

        let inner = cs.state.as_ref().unwrap();
        assert_eq!(inner.selected_index, None);
    }

    #[test]
    fn test_enter_confirms_selected() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "beta", Rect::new(0, 0, 20, 1));

        let key = KeyEvent::from(KeyCode::Enter);
        let result = cs.handle_key(key);

        assert_eq!(
            result,
            EventResult::Action(ChoiceSelectAction::Selected("beta".to_string()))
        );
        assert!(!cs.is_open());
    }

    #[test]
    fn test_enter_with_no_selection_uses_typed_text() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "custom", Rect::new(0, 0, 20, 1));
        // No selection, typed text is "custom"

        let key = KeyEvent::from(KeyCode::Enter);
        let result = cs.handle_key(key);

        assert_eq!(
            result,
            EventResult::Action(ChoiceSelectAction::Selected("custom".to_string()))
        );
    }

    #[test]
    fn test_esc_cancels_with_typed_text() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "beta", Rect::new(0, 0, 20, 1));

        let key = KeyEvent::from(KeyCode::Esc);
        let result = cs.handle_key(key);

        assert_eq!(
            result,
            EventResult::Action(ChoiceSelectAction::Cancelled("beta".to_string()))
        );
        assert!(!cs.is_open());
    }

    #[test]
    fn test_down_navigates_selection() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "", Rect::new(0, 0, 20, 1));
        // No selection initially

        let down = KeyEvent::from(KeyCode::Down);
        assert_eq!(cs.handle_key(down), EventResult::Consumed);
        assert_eq!(cs.state.as_ref().unwrap().selected_index, Some(0));

        let down = KeyEvent::from(KeyCode::Down);
        assert_eq!(cs.handle_key(down), EventResult::Consumed);
        assert_eq!(cs.state.as_ref().unwrap().selected_index, Some(1));
    }

    #[test]
    fn test_up_from_first_deselects() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "alpha", Rect::new(0, 0, 20, 1));
        // Pre-selected at index 0

        let up = KeyEvent::from(KeyCode::Up);
        assert_eq!(cs.handle_key(up), EventResult::Consumed);
        assert_eq!(cs.state.as_ref().unwrap().selected_index, None);
    }

    #[test]
    fn test_typing_activates_filter_and_clears_selection() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "beta", Rect::new(0, 0, 20, 1));
        assert_eq!(cs.state.as_ref().unwrap().selected_index, Some(1));
        assert!(!cs.state.as_ref().unwrap().filter_active);

        let key = KeyEvent::from(KeyCode::Char('a'));
        assert_eq!(cs.handle_key(key), EventResult::Consumed);

        let inner = cs.state.as_ref().unwrap();
        assert!(inner.filter_active);
        assert_eq!(inner.selected_index, None);
        assert_eq!(inner.edit_input.text(), "betaa");
    }

    #[test]
    fn test_filtering_narrows_choices() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "", Rect::new(0, 0, 20, 1));

        // Type "al" to filter
        cs.handle_key(KeyEvent::from(KeyCode::Char('a')));
        cs.handle_key(KeyEvent::from(KeyCode::Char('l')));

        let filtered = cs.filtered_choices();
        // "alpha" matches "al"
        assert!(filtered.iter().any(|(_, c)| c == "alpha"));
        // "beta" and "gamma" should not match "al"
        assert!(!filtered.iter().any(|(_, c)| c == "beta"));
        assert!(!filtered.iter().any(|(_, c)| c == "gamma"));
        // Result set is narrowed from the original 4
        assert!(filtered.len() < 4);
    }

    #[test]
    fn test_backspace_activates_filter() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "beta", Rect::new(0, 0, 20, 1));

        let key = KeyEvent::from(KeyCode::Backspace);
        assert_eq!(cs.handle_key(key), EventResult::Consumed);

        let inner = cs.state.as_ref().unwrap();
        assert!(inner.filter_active);
        assert_eq!(inner.selected_index, None);
        assert_eq!(inner.edit_input.text(), "bet");
    }

    #[test]
    fn test_not_handled_when_closed() {
        let mut cs = ChoiceSelectComponent::new();
        let key = KeyEvent::from(KeyCode::Enter);
        assert_eq!(cs.handle_key(key), EventResult::NotHandled);
    }

    #[test]
    fn test_collect_overlays_when_closed() {
        let mut cs = ChoiceSelectComponent::new();
        assert!(cs.collect_overlays().is_empty());
    }

    #[test]
    fn test_collect_overlays_when_open() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "", Rect::new(5, 10, 20, 1));

        let overlays = cs.collect_overlays();
        assert_eq!(overlays.len(), 1);

        let req = &overlays[0];
        assert_eq!(req.anchor, Rect::new(5, 10, 20, 1));
        // Width should accommodate labels + padding
        assert!(req.size.0 > 0);
        // Height = 4 items (capped at max 10) + 1 bottom border
        assert_eq!(req.size.1, 5);
    }

    #[test]
    fn test_click_select() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "", Rect::new(0, 0, 20, 1));

        let result = cs.click_select(2);
        assert_eq!(
            result,
            Some(ChoiceSelectAction::Selected("gamma".to_string()))
        );
        assert!(!cs.is_open());
    }

    #[test]
    fn test_click_select_out_of_range() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "", Rect::new(0, 0, 20, 1));

        let result = cs.click_select(100);
        assert_eq!(result, None);
        assert!(cs.is_open()); // stays open
    }

    #[test]
    fn test_open_with_descriptions() {
        let mut cs = ChoiceSelectComponent::new();
        let descs = vec![
            Some("First letter".to_string()),
            None,
            Some("Third letter".to_string()),
            None,
        ];
        cs.open_with_descriptions(make_choices(), descs.clone(), "", Rect::new(0, 0, 20, 1));

        let inner = cs.state.as_ref().unwrap();
        assert_eq!(inner.descriptions, descs);
    }

    #[test]
    fn test_down_does_not_exceed_filtered_count() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(vec!["a".to_string(), "b".to_string()], "", Rect::new(0, 0, 20, 1));

        // Navigate to last item
        cs.handle_key(KeyEvent::from(KeyCode::Down)); // -> 0
        cs.handle_key(KeyEvent::from(KeyCode::Down)); // -> 1
        cs.handle_key(KeyEvent::from(KeyCode::Down)); // stays at 1

        assert_eq!(cs.state.as_ref().unwrap().selected_index, Some(1));
    }

    #[test]
    fn test_overlay_renders_without_panic() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_choices(), "beta", Rect::new(0, 0, 20, 1));

        let overlays = cs.collect_overlays();
        assert_eq!(overlays.len(), 1);

        let req = &overlays[0];
        let area = Rect::new(0, 0, req.size.0, req.size.1);
        let mut buf = Buffer::empty(area);
        let palette = ratatui_themes::Theme::default().palette();
        let colors = UiColors::from_palette(&palette);
        req.content.render(area, &mut buf, &colors);
    }

    /// Helper: create 15 choices (enough to scroll with max_visible=10).
    fn make_many_choices() -> Vec<String> {
        (0..15).map(|i| format!("item_{i:02}")).collect()
    }

    #[test]
    fn test_click_select_with_scroll_offset() {
        // When the list is scrolled (selected item past visible area),
        // click_select should select based on the filtered index, NOT visual row.
        // The caller (App) must add scroll_offset to the visual row before calling.
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_many_choices(), "", Rect::new(0, 0, 20, 1));

        // Navigate down to item 12 so the list scrolls
        let down = KeyEvent::from(KeyCode::Down);
        for _ in 0..13 {
            cs.handle_key(down);
        }
        assert_eq!(cs.selected_index(), Some(12));

        // The scroll_offset should account for the selected item.
        // With 10 visible items and selected=12: offset = 12 - (10-1) = 3
        // So clicking on visual row 0 should map to item 3.
        // The caller must compute: clicked_index = visual_row + scroll_offset(visible_items)
        let visible = 10usize;
        let scroll = cs.compute_scroll_offset(visible);
        assert_eq!(scroll, 3, "scroll_offset should be 3 when selected=12, visible=10");

        // Clicking visual row 5 → actual index 5 + 3 = 8
        let clicked_index = 5 + scroll;
        let result = cs.click_select(clicked_index);
        assert_eq!(
            result,
            Some(ChoiceSelectAction::Selected("item_08".to_string())),
            "Should select item_08 (visual row 5 + scroll 3)"
        );
    }

    #[test]
    fn test_scroll_offset_computation() {
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_many_choices(), "", Rect::new(0, 0, 20, 1));

        // No selection → offset 0
        assert_eq!(cs.compute_scroll_offset(10), 0);

        // Select item 5 (visible within 10 items) → offset 0
        let down = KeyEvent::from(KeyCode::Down);
        for _ in 0..6 {
            cs.handle_key(down);
        }
        assert_eq!(cs.selected_index(), Some(5));
        assert_eq!(cs.compute_scroll_offset(10), 0);

        // Select item 10 → offset = 10 - 9 = 1
        for _ in 0..5 {
            cs.handle_key(down);
        }
        assert_eq!(cs.selected_index(), Some(10));
        assert_eq!(cs.compute_scroll_offset(10), 1);

        // Select item 14 → offset = 14 - 9 = 5
        for _ in 0..4 {
            cs.handle_key(down);
        }
        assert_eq!(cs.selected_index(), Some(14));
        assert_eq!(cs.compute_scroll_offset(10), 5);
    }

    #[test]
    fn test_hover_accounts_for_scroll() {
        // When the choice list is scrolled, hovering over the 4th visible row
        // should highlight item at index (4 + scroll_offset), not item 4.
        let mut cs = ChoiceSelectComponent::new();
        cs.open(make_many_choices(), "", Rect::new(0, 0, 20, 1));

        // Navigate to item 12 to cause scroll
        let down = KeyEvent::from(KeyCode::Down);
        for _ in 0..13 {
            cs.handle_key(down);
        }
        assert_eq!(cs.selected_index(), Some(12));

        // Set mouse position at visual row 3 (within overlay area starting at y=1)
        // Overlay area: x=0, y=1, width=20, height=11 (10 items + 1 bottom border)
        cs.set_mouse_position(Some((5, 4))); // row 4, within overlay at y=1 → visual row 3

        let overlays = cs.collect_overlays();
        assert_eq!(overlays.len(), 1);

        let req = &overlays[0];
        // Simulate the actual overlay area
        let area = Rect::new(0, 1, req.size.0, req.size.1);
        let mut buf = Buffer::empty(Rect::new(0, 0, 30, 20));
        let palette = ratatui_themes::Theme::default().palette();
        let colors = UiColors::from_palette(&palette);
        req.content.render(area, &mut buf, &colors);

        // With scroll_offset=3, visual row 3 should highlight item 6 (3+3),
        // not item 3. Verify by checking the buffer for hover highlight on the correct row.
        // The hover highlight uses colors.choice.hover_bg.
        // Visual row 3 in the overlay (row=4 absolute) should have hover_bg.
        let hover_row = 4u16; // absolute row where we set the mouse
        let cell = buf.cell((1, hover_row)).unwrap(); // col 1 to skip border
        assert_ne!(
            cell.bg,
            ratatui::style::Color::Reset,
            "Hovered row should have a background highlight"
        );
    }
}
