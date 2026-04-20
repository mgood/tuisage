//! Shared state and logic for list-based panels (flags, args).
//!
//! `ListPanelBase` extracts the common fields and methods from
//! `FlagPanelComponent` and `ArgPanelComponent` to eliminate duplication.
//! Each panel embeds a `ListPanelBase` and delegates shared operations.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::Rect;
use ratatui_interact::components::{InputState, ListPickerState};

use crate::app::MatchScores;

use super::choice_select::{ChoiceSelectAction, ChoiceSelectComponent};
use super::filterable::{compute_match_scores, FilterableItem};
use super::{find_adjacent_match, find_first_match, Component, EventResult, OverlayRequest};

// ── Internal event types ────────────────────────────────────────────

/// Result from inline editing key handling.
pub enum EditEvent {
    /// Value changed during editing (live sync).
    ValueChanged { index: usize, value: String },
    /// Editing finished (Enter/Esc committed).
    EditFinished { index: usize, value: String },
    /// Key consumed internally (cursor move, etc.).
    Consumed,
}

/// Result from choice select key handling.
pub enum ChoiceEvent {
    /// User selected a value.
    Selected { index: usize, value: String },
    /// User cancelled, keeping typed text.
    Cancelled { index: usize, value: String },
    /// Key consumed by the choice select.
    Consumed,
}

/// Result from mouse event handling in a list panel.
pub enum MouseResult {
    /// A choice select item was clicked — yields a ChoiceEvent.
    ChoiceClicked(ChoiceEvent),
    /// An item was clicked and it was already selected (activate/enter behavior).
    ClickActivate(usize),
    /// An item was clicked and selected (no activation).
    ItemSelected(usize),
    /// The mouse event was consumed internally (scroll, overlay dismiss, etc.).
    Consumed,
    /// The mouse event was not handled by this panel.
    NotHandled,
}

/// Result from a focus-lost transition in a list panel.
pub enum FocusLostEvent {
    /// Inline editing finished and should be committed by the parent.
    EditFinished { index: usize, value: String },
    /// Focus loss closed local UI state without any parent action.
    Consumed,
    /// The panel had no focus-local state to clean up.
    NotHandled,
}

// ── ListPanelBase ───────────────────────────────────────────────────

/// Shared state for list-based panel components.
pub struct ListPanelBase {
    /// List selection/scroll state.
    pub list_state: ListPickerState,

    /// Whether this panel has keyboard focus.
    pub focused: bool,
    /// Whether filter input is actively being typed (set by wrapper, used for border color).
    pub filter_active: bool,
    /// Current filter text (may persist after typing mode ends).
    pub filter_text: String,
    /// Match scores keyed by item name.
    pub match_scores: HashMap<String, MatchScores>,
    /// Ordered item keys matching the visible list (set during set_filter).
    pub item_keys: Vec<String>,
    /// Cached filterable items for standalone score computation.
    pub filterable_items: Vec<FilterableItem>,
    /// Mouse hover index (computed during render from mouse_position + area).
    pub hovered_index: Option<usize>,
    /// Current mouse position (set by parent, used to compute hovered_index during render).
    pub mouse_position: Option<(u16, u16)>,

    /// Inline edit state.
    pub editing: bool,
    pub edit_input: InputState,

    /// Embedded choice select component.
    pub choice_select: ChoiceSelectComponent,
    /// Which item index has the choice select open.
    pub choice_select_index: Option<usize>,
    /// X-offset from panel left to the value column.
    pub value_column: u16,
}

impl ListPanelBase {
    pub fn new() -> Self {
        Self {
            list_state: ListPickerState::new(0),
            focused: false,
            filter_active: false,
            filter_text: String::new(),
            match_scores: HashMap::new(),
            item_keys: Vec::new(),
            filterable_items: Vec::new(),
            hovered_index: None,
            mouse_position: None,
            editing: false,
            edit_input: InputState::empty(),
            choice_select: ChoiceSelectComponent::new(),
            choice_select_index: None,
            value_column: 0,
        }
    }

    // ── Selection / navigation ──────────────────────────────────────

    pub fn selected_index(&self) -> usize {
        self.list_state.selected_index
    }

    pub fn set_total(&mut self, total: usize) {
        self.list_state.set_total(total);
    }

    #[cfg(test)]
    pub fn select(&mut self, index: usize) {
        self.list_state.select(index);
    }

    pub fn ensure_visible(&mut self, viewport_height: usize) {
        self.list_state.ensure_visible(viewport_height);
    }

    #[allow(dead_code)]
    pub fn set_scroll(&mut self, scroll: usize) {
        self.list_state.scroll = scroll as u16;
    }

    /// Move selection up. Filter-aware when scores are present.
    pub fn move_up(&mut self) {
        if !self.filter_text.is_empty() && !self.match_scores.is_empty() {
            let current = self.list_state.selected_index;
            if let Some(idx) =
                find_adjacent_match(&self.item_keys, &self.match_scores, current, false)
            {
                self.list_state.select(idx);
            }
        } else {
            self.list_state.select_prev();
        }
    }

    /// Move selection down. Filter-aware when scores are present.
    pub fn move_down(&mut self) {
        if !self.filter_text.is_empty() && !self.match_scores.is_empty() {
            let current = self.list_state.selected_index;
            if let Some(idx) =
                find_adjacent_match(&self.item_keys, &self.match_scores, current, true)
            {
                self.list_state.select(idx);
            }
        } else {
            self.list_state.select_next();
        }
    }

    /// Auto-select the first matching item if current selection doesn't match.
    pub fn auto_select_next_match(&mut self) -> bool {
        if self.match_scores.is_empty() {
            return false;
        }
        let current = self.list_state.selected_index;
        if let Some(idx) = find_first_match(&self.item_keys, &self.match_scores, current) {
            if idx != current {
                self.list_state.select(idx);
                return true;
            }
        }
        false
    }

    // ── Focus / hover ───────────────────────────────────────────────

    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    pub fn handle_focus_gained(&mut self) {
        self.focused = true;
    }

    pub fn handle_focus_lost(&mut self) -> FocusLostEvent {
        self.focused = false;

        if self.choice_select.is_open() {
            if self.editing {
                let index = self
                    .choice_select_index
                    .unwrap_or(self.list_state.selected_index);
                let value = self.choice_select.typed_text().to_string();
                self.close_choice_select();
                self.editing = false;
                return FocusLostEvent::EditFinished { index, value };
            }

            self.close_choice_select();
            return FocusLostEvent::Consumed;
        }

        if self.editing {
            let index = self.list_state.selected_index;
            let value = self.finish_editing();
            return FocusLostEvent::EditFinished { index, value };
        }

        FocusLostEvent::NotHandled
    }

    #[allow(dead_code)]
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.hovered_index = idx;
    }

    pub fn set_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.mouse_position = pos;
    }

    /// Compute hovered_index from mouse_position and the given panel area.
    /// Call this at the start of render to update hover state.
    pub fn compute_hovered_index(&mut self, area: Rect) {
        self.hovered_index = if self.focused {
            self.hovered_index_for_area(area)
        } else {
            None
        };
    }

    /// Compute hover index from mouse position relative to a bordered panel area.
    fn hovered_index_for_area(&self, area: Rect) -> Option<usize> {
        let (col, row) = self.mouse_position?;
        let inner_top = area.y + 1; // skip top border
        let inner_bottom = area.y + area.height.saturating_sub(1);
        if col < area.x || col >= area.x + area.width || row < inner_top || row >= inner_bottom {
            return None;
        }
        Some((row - inner_top) as usize + self.list_state.scroll as usize)
    }

    // ── Filter state ────────────────────────────────────────────────

    pub fn has_active_filter(&self) -> bool {
        !self.filter_text.is_empty()
    }

    pub fn filter_text(&self) -> &str {
        &self.filter_text
    }

    /// Cache filterable items for standalone score computation.
    /// Called during sync_state so that apply_filter can work without external data.
    pub fn set_filterable_items(&mut self, items: Vec<FilterableItem>) {
        self.item_keys = items.iter().map(|i| i.key.clone()).collect();
        self.filterable_items = items;
    }

    /// Apply filter from cached filterable items. Computes scores standalone.
    pub fn apply_filter_from_items(&mut self, text: &str) {
        self.filter_text = text.to_string();
        if text.is_empty() {
            self.match_scores.clear();
        } else {
            self.match_scores = compute_match_scores(&self.filterable_items, text);
        }
    }

    /// Clear all filter state (text, scores). Preserves item_keys and filterable_items.
    pub fn clear_filter_state(&mut self) {
        self.filter_text.clear();
        self.match_scores.clear();
        self.filter_active = false;
    }

    // ── Inline editing ──────────────────────────────────────────────

    pub fn is_editing(&self) -> bool {
        self.editing
    }

    pub fn start_editing(&mut self, text: &str) {
        self.editing = true;
        self.edit_input.set_text(text.to_string());
    }

    pub fn finish_editing(&mut self) -> String {
        self.editing = false;
        self.edit_input.text().to_string()
    }

    pub fn editing_text(&self) -> &str {
        if self.editing {
            self.edit_input.text()
        } else {
            ""
        }
    }

    /// Handle a key event during inline editing. Returns an EditEvent
    /// that the panel maps to its specific action type.
    pub fn handle_editing_key(&mut self, key: KeyEvent) -> EditEvent {
        let idx = self.list_state.selected_index;
        match key.code {
            KeyCode::Esc | KeyCode::Enter => {
                let value = self.finish_editing();
                EditEvent::EditFinished { index: idx, value }
            }
            KeyCode::Backspace => {
                self.edit_input.delete_char_backward();
                let value = self.edit_input.text().to_string();
                EditEvent::ValueChanged { index: idx, value }
            }
            KeyCode::Delete => {
                self.edit_input.delete_char_forward();
                let value = self.edit_input.text().to_string();
                EditEvent::ValueChanged { index: idx, value }
            }
            KeyCode::Left => {
                self.edit_input.move_left();
                EditEvent::Consumed
            }
            KeyCode::Right => {
                self.edit_input.move_right();
                EditEvent::Consumed
            }
            KeyCode::Home => {
                self.edit_input.move_home();
                EditEvent::Consumed
            }
            KeyCode::End => {
                self.edit_input.move_end();
                EditEvent::Consumed
            }
            KeyCode::Char(c) => {
                self.edit_input.insert_char(c);
                let value = self.edit_input.text().to_string();
                EditEvent::ValueChanged { index: idx, value }
            }
            _ => EditEvent::Consumed,
        }
    }

    // ── Choice select ───────────────────────────────────────────────

    pub fn is_choosing(&self) -> bool {
        self.choice_select.is_open()
    }

    pub fn choice_select_index(&self) -> Option<usize> {
        self.choice_select_index
    }

    pub fn choice_select_text(&self) -> &str {
        self.choice_select.typed_text()
    }

    #[cfg(test)]
    pub fn selected_choice_index(&self) -> Option<usize> {
        self.choice_select.selected_index()
    }

    pub fn set_choice_select_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.choice_select.set_mouse_position(pos);
    }

    pub fn open_choice_select(
        &mut self,
        index: usize,
        choices: Vec<String>,
        current_value: &str,
        value_column: u16,
    ) {
        self.choice_select_index = Some(index);
        self.value_column = value_column;
        self.choice_select
            .open(choices, current_value, Rect::ZERO);
    }

    pub fn open_completion_select(
        &mut self,
        index: usize,
        choices: Vec<String>,
        descriptions: Vec<Option<String>>,
        current_value: &str,
        value_column: u16,
    ) {
        self.choice_select_index = Some(index);
        self.value_column = value_column;
        self.start_editing(current_value);
        self.choice_select
            .open_with_descriptions(choices, descriptions, current_value, Rect::ZERO);
    }

    pub fn close_choice_select(&mut self) {
        self.choice_select.close();
        self.choice_select_index = None;
    }

    /// Handle a key event when the choice select is open.
    /// Returns Some(ChoiceEvent) if handled, None if the choice select isn't open.
    pub fn handle_choice_key(&mut self, key: KeyEvent) -> Option<ChoiceEvent> {
        if !self.choice_select.is_open() {
            return None;
        }
        let item_idx = self.choice_select_index.unwrap_or(0);
        match self.choice_select.handle_key(key) {
            EventResult::Action(ChoiceSelectAction::Selected(value)) => {
                self.choice_select_index = None;
                self.editing = false;
                Some(ChoiceEvent::Selected {
                    index: item_idx,
                    value,
                })
            }
            EventResult::Action(ChoiceSelectAction::Cancelled(value)) => {
                self.choice_select_index = None;
                self.editing = false;
                Some(ChoiceEvent::Cancelled {
                    index: item_idx,
                    value,
                })
            }
            _ => Some(ChoiceEvent::Consumed),
        }
    }

    /// Click-select an item in the choice overlay by visible index.
    #[allow(dead_code)]
    pub fn click_choice_select(&mut self, index: usize) -> Option<ChoiceEvent> {
        let item_idx = self.choice_select_index?;
        if let Some(action) = self.choice_select.click_select(index) {
            self.choice_select_index = None;
            self.editing = false;
            match action {
                ChoiceSelectAction::Selected(value) => Some(ChoiceEvent::Selected {
                    index: item_idx,
                    value,
                }),
                ChoiceSelectAction::Cancelled(value) => Some(ChoiceEvent::Cancelled {
                    index: item_idx,
                    value,
                }),
            }
        } else {
            Some(ChoiceEvent::Consumed)
        }
    }

    #[allow(dead_code)]
    pub fn filtered_choices(&self) -> Vec<(usize, String)> {
        self.choice_select.filtered_choices()
    }

    pub fn choice_select_scroll_offset(&self, visible_items: usize) -> usize {
        self.choice_select.compute_scroll_offset(visible_items)
    }

    pub fn scroll_choice_select_up(&mut self) {
        let key =
            crossterm::event::KeyEvent::new(KeyCode::Up, crossterm::event::KeyModifiers::NONE);
        let _ = self.choice_select.handle_key(key);
    }

    pub fn scroll_choice_select_down(&mut self) {
        let key =
            crossterm::event::KeyEvent::new(KeyCode::Down, crossterm::event::KeyModifiers::NONE);
        let _ = self.choice_select.handle_key(key);
    }

    pub fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        self.choice_select.collect_overlays()
    }

    pub fn handle_choice_click(
        &mut self,
        col: u16,
        row: u16,
        overlay_rect: Option<Rect>,
    ) -> MouseResult {
        if let Some(rect) = overlay_rect {
            if col >= rect.x
                && col < rect.x + rect.width
                && row >= rect.y
                && row < rect.y + rect.height.saturating_sub(1)
            {
                let visible_items = rect.height.saturating_sub(1) as usize;
                let scroll = self.choice_select_scroll_offset(visible_items);
                let clicked_index = (row - rect.y) as usize + scroll;
                return match self.click_choice_select(clicked_index) {
                    Some(event) => MouseResult::ChoiceClicked(event),
                    None => MouseResult::Consumed,
                };
            }
        }

        self.close_choice_select();
        MouseResult::Consumed
    }

    // ── Mouse handling ──────────────────────────────────────────────

    /// Handle a mouse event for this list panel.
    ///
    /// Handles overlay clicks, scroll, and item selection/activation.
    pub fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> MouseResult {
        let col = event.column;
        let row = event.row;
        let total_items = self.list_state.total_items;

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                if self.is_choosing() {
                    return MouseResult::Consumed;
                }

                // Item click within the panel area
                let inner_top = area.y + 1; // skip border
                if row >= inner_top
                    && row < area.y + area.height.saturating_sub(1)
                    && col >= area.x
                    && col < area.x + area.width
                {
                    let clicked_offset = (row - inner_top) as usize;
                    let item_index = self.list_state.scroll as usize + clicked_offset;
                    if item_index < total_items {
                        let was_selected =
                            self.focused && self.list_state.selected_index == item_index;
                        self.list_state.select(item_index);
                        if was_selected {
                            return MouseResult::ClickActivate(item_index);
                        }
                        return MouseResult::ItemSelected(item_index);
                    }
                }
                MouseResult::NotHandled
            }
            MouseEventKind::ScrollUp => {
                if self.is_choosing() {
                    self.scroll_choice_select_up();
                } else {
                    self.move_up();
                }
                MouseResult::Consumed
            }
            MouseEventKind::ScrollDown => {
                if self.is_choosing() {
                    self.scroll_choice_select_down();
                } else {
                    self.move_down();
                }
                MouseResult::Consumed
            }
            _ => MouseResult::NotHandled,
        }
    }
}
