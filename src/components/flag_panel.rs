//! Flag panel component.
//!
//! Owns flag navigation state and handles keyboard events for moving
//! through the flag list. Value mutations (toggle, clear, edit) are
//! emitted as actions for the parent to apply.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, StatefulWidget, Widget},
};

use crate::app::FlagValue;
use crate::theme::UiColors;

use super::list_panel_base::{ChoiceEvent, EditEvent, FocusLostEvent, ListPanelBase, MouseResult};
use super::filterable::{FilterableItem, Filterable};
use super::{
    build_help_line, panel_block, panel_title, push_edit_cursor, push_highlighted_name,
    push_selection_cursor, render_help_overlays, render_panel_scrollbar, selection_bg, Component,
    EventResult, ItemContext, OverlayRequest, PanelState,
};

// ── Actions ─────────────────────────────────────────────────────────

/// Actions that the flag panel can emit to its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum FlagPanelAction {
    /// Toggle a Bool/NegBool/Count flag at the given index (Space or Enter on non-string).
    ToggleFlag(usize),
    /// Enter pressed on the selected flag row.
    EnterRequest(FlagPanelEnterRequest),
    /// Backspace pressed: clear/decrement flag at the given index.
    ClearFlag(usize),
    /// Mouse click on negate region: target state for the NegBool flag at the given index.
    NegBoolClick(usize, Option<bool>),
    /// Choice select confirmed: apply the selected value to the flag at the given index.
    ChoiceSelected { index: usize, value: String },
    /// Choice select cancelled: keep the typed text as the flag value.
    ChoiceCancelled { index: usize, value: String },
    /// Inline edit value changed (live sync to flag_values).
    ValueChanged { index: usize, value: String },
    /// Inline edit finished (Enter/Esc committed the value).
    EditFinished { index: usize, value: String },
}

/// Panel-owned interpretation of Enter on a flag row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlagPanelEnterRequest {
    Toggle,
    ChoiceSelect {
        index: usize,
        choices: Vec<String>,
        current_value: String,
        value_column: u16,
    },
    EditOrComplete {
        index: usize,
        arg_name: Option<String>,
        current_value: String,
        value_column: u16,
    },
}

// ── FlagPanelComponent ──────────────────────────────────────────────

/// A flag list panel that owns its navigation state and handles events.
///
/// Flag *values* are owned by App (they live in a per-command-path HashMap).
/// The component receives flag specs and values via setters before rendering,
/// and emits actions for the parent to apply mutations.
pub struct FlagPanelComponent {
    base: ListPanelBase,
    /// Column start positions of negate strings (populated during render, used for mouse clicks).
    negate_cols: HashMap<usize, u16>,
    enter_requests: Vec<FlagPanelEnterRequest>,
}

impl FlagPanelComponent {
    pub fn new() -> Self {
        Self {
            base: ListPanelBase::new(),
            negate_cols: HashMap::new(),
            enter_requests: Vec::new(),
        }
    }

    // ── Delegated public API ────────────────────────────────────────

    pub fn selected_index(&self) -> usize {
        self.base.selected_index()
    }

    pub fn set_focused(&mut self, focused: bool) {
        self.base.set_focused(focused);
    }

    #[allow(dead_code)]
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.base.set_hovered_index(idx);
    }

    pub fn set_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.base.set_mouse_position(pos);
    }

    pub fn set_total(&mut self, total: usize) {
        self.base.set_total(total);
    }

    #[cfg(test)]
    pub fn select(&mut self, index: usize) {
        self.base.select(index);
    }

    #[cfg(test)]
    pub fn ensure_visible(&mut self, viewport_height: usize) {
        self.base.ensure_visible(viewport_height);
    }

    /// Cache filterable item data from flag specs for standalone filtering.
    pub fn set_filterable_items_from_flags(&mut self, flags: &[&usage::SpecFlag]) {
        let items: Vec<FilterableItem> = flags
            .iter()
            .map(|f| {
                let mut name_texts = vec![f.name.clone()];
                for l in &f.long {
                    name_texts.push(format!("--{l}"));
                }
                for s in &f.short {
                    name_texts.push(format!("-{s}"));
                }
                FilterableItem {
                    key: f.name.clone(),
                    name_texts,
                    help: f.help.clone(),
                }
            })
            .collect();
        self.base.set_filterable_items(items);
    }

    pub fn set_enter_requests_from_flags(
        &mut self,
        flags: &[&usage::SpecFlag],
        flag_values: &[(String, FlagValue)],
    ) {
        self.enter_requests = flags
            .iter()
            .enumerate()
            .filter_map(|(index, _)| Self::build_enter_request(index, flags, flag_values))
            .collect();
    }

    #[cfg(test)]
    pub fn move_up(&mut self) {
        self.base.move_up();
    }

    #[allow(dead_code)]
    pub fn set_scroll(&mut self, scroll: usize) {
        self.base.set_scroll(scroll);
    }

    // ── Choice select (delegated) ───────────────────────────────────

    pub fn is_choosing(&self) -> bool {
        self.base.is_choosing()
    }

    #[cfg(test)]
    pub fn choice_select_index(&self) -> Option<usize> {
        self.base.choice_select_index()
    }

    pub fn set_choice_select_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.base.set_choice_select_mouse_position(pos);
    }

    pub fn handle_choice_click(
        &mut self,
        col: u16,
        row: u16,
        overlay_rect: Option<Rect>,
    ) -> EventResult<FlagPanelAction> {
        match self.base.handle_choice_click(col, row, overlay_rect) {
            MouseResult::ChoiceClicked(ChoiceEvent::Selected { index, value }) => {
                EventResult::Action(FlagPanelAction::ChoiceSelected { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Cancelled { index, value }) => {
                EventResult::Action(FlagPanelAction::ChoiceCancelled { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Consumed) | MouseResult::Consumed => {
                EventResult::Consumed
            }
            MouseResult::ClickActivate(_) | MouseResult::ItemSelected(_) => EventResult::Consumed,
            MouseResult::NotHandled => EventResult::NotHandled,
        }
    }

    pub fn open_choice_select(
        &mut self,
        index: usize,
        choices: Vec<String>,
        current_value: &str,
        value_column: u16,
    ) {
        self.base
            .open_choice_select(index, choices, current_value, value_column);
    }

    pub fn open_completion_select(
        &mut self,
        index: usize,
        choices: Vec<String>,
        descriptions: Vec<Option<String>>,
        current_value: &str,
        value_column: u16,
    ) {
        self.base
            .open_completion_select(index, choices, descriptions, current_value, value_column);
    }

    // ── Inline editing (delegated) ──────────────────────────────────

    pub fn is_editing(&self) -> bool {
        self.base.is_editing()
    }

    pub fn start_editing(&mut self, text: &str) {
        self.base.start_editing(text);
    }

    pub fn finish_editing(&mut self) -> String {
        self.base.finish_editing()
    }

    #[allow(dead_code)]
    pub fn editing_text(&self) -> &str {
        self.base.editing_text()
    }

    fn build_enter_request(
        index: usize,
        flags: &[&usage::SpecFlag],
        flag_values: &[(String, FlagValue)],
    ) -> Option<FlagPanelEnterRequest> {
        let flag = flags.get(index)?;
        let value = flag_values
            .iter()
            .find(|(name, _)| name == &flag.name)
            .map(|(_, value)| value);

        let FlagValue::String(current_value) = value?.clone() else {
            return Some(FlagPanelEnterRequest::Toggle);
        };

        let value_column = Self::value_column_for_flag(flag, value);
        if let Some(choices) = flag
            .arg
            .as_ref()
            .and_then(|arg| arg.choices.as_ref())
            .map(|choices| choices.choices.clone())
        {
            Some(FlagPanelEnterRequest::ChoiceSelect {
                index,
                choices,
                current_value,
                value_column,
            })
        } else {
            Some(FlagPanelEnterRequest::EditOrComplete {
                index,
                arg_name: flag.arg.as_ref().map(|arg| arg.name.clone()),
                current_value,
                value_column,
            })
        }
    }

    fn enter_request_for_index(&self, index: usize) -> Option<FlagPanelEnterRequest> {
        self.enter_requests.get(index).cloned()
    }

    fn enter_action_for_index(&self, index: usize) -> EventResult<FlagPanelAction> {
        self.enter_request_for_index(index)
            .map(FlagPanelAction::EnterRequest)
            .map(EventResult::Action)
            .unwrap_or(EventResult::Consumed)
    }

    fn value_column_for_flag(flag: &usage::SpecFlag, value: Option<&FlagValue>) -> u16 {
        // indicator width: "✓ "=2, "○ "=2, "[n] "=varies, "[·] "=4, "[•] "=4
        let indicator_width = match value {
            Some(FlagValue::Count(n)) => format!("[{}] ", n).chars().count(),
            Some(FlagValue::String(_)) => 4,
            _ => 2,
        };

        let flag_display = flag_display_string(flag);
        let flag_display_len = flag_display.chars().count();

        let mut extra = 0usize;
        if flag.global {
            extra += 4; // " [G]"
        }
        if flag.required {
            extra += 2; // " *"
        }

        (1 + 2 + indicator_width + flag_display_len + extra + 3) as u16
    }

    // ── Event mapping ───────────────────────────────────────────────

    fn map_edit_event(event: EditEvent) -> EventResult<FlagPanelAction> {
        match event {
            EditEvent::ValueChanged { index, value } => {
                EventResult::Action(FlagPanelAction::ValueChanged { index, value })
            }
            EditEvent::EditFinished { index, value } => {
                EventResult::Action(FlagPanelAction::EditFinished { index, value })
            }
            EditEvent::Consumed => EventResult::Consumed,
        }
    }

    fn map_choice_event(event: ChoiceEvent) -> EventResult<FlagPanelAction> {
        match event {
            ChoiceEvent::Selected { index, value } => {
                EventResult::Action(FlagPanelAction::ChoiceSelected { index, value })
            }
            ChoiceEvent::Cancelled { index, value } => {
                EventResult::Action(FlagPanelAction::ChoiceCancelled { index, value })
            }
            ChoiceEvent::Consumed => EventResult::Consumed,
        }
    }

    // ── Rendering ───────────────────────────────────────────────────

    pub fn render_with_data(
        &mut self,
        area: Rect,
        buf: &mut Buffer,
        colors: &UiColors,
        data: FlagRenderData<'_>,
    ) {
        let inner_height = area.height.saturating_sub(2) as usize;
        self.base.ensure_visible(inner_height);
        self.base.compute_hovered_index(area);
        let ps = self.panel_state(colors);

        // Compute negate column positions for mouse hit-testing
        let inner_left = area.x + 1;
        self.negate_cols = data
            .flags
            .iter()
            .enumerate()
            .filter(|(_, flag)| flag.negate.is_some())
            .map(|(i, flag)| {
                let display_len = flag_display_string(flag).len() as u16;
                let mut col = inner_left + 2 + 2 + display_len;
                if flag.global {
                    col += 4;
                }
                if flag.required {
                    col += 2;
                }
                col += 3; // " / " separator
                (i, col)
            })
            .collect();

        // Derive edit state: choice_select takes priority, then inline editing
        let (editing, edit_before, edit_after, cs_flag_idx) =
            if self.base.choice_select.is_open() {
                (
                    true,
                    self.base.choice_select.text_before_cursor().to_string(),
                    self.base.choice_select.text_after_cursor().to_string(),
                    self.base.choice_select_index,
                )
            } else if self.base.editing {
                (
                    true,
                    self.base.edit_input.text_before_cursor().to_string(),
                    self.base.edit_input.text_after_cursor().to_string(),
                    None,
                )
            } else {
                (false, String::new(), String::new(), None)
            };

        let panel = FlagPanel {
            flags: data.flags,
            flag_values: data.flag_values,
            flag_defaults: data.flag_defaults,
            flag_index: self.base.list_state.selected_index,
            scroll_offset: self.base.list_state.scroll as usize,
            hovered_index: self.base.hovered_index,
            editing,
            edit_before_cursor: edit_before,
            edit_after_cursor: edit_after,
            choice_select_flag_index: cs_flag_idx,
            panel_state: &ps,
            colors,
        };
        panel.render(area, buf);

        // Update choice_select anchor now that we know the panel area
        if self.base.choice_select.is_open() {
            if let Some(cs_idx) = self.base.choice_select_index {
                let scroll = self.base.list_state.scroll as usize;
                let row_in_viewport = cs_idx.saturating_sub(scroll);
                let anchor_y = area.y + 1 + row_in_viewport as u16;
                let anchor_x = area.x + self.base.value_column.saturating_sub(1);
                let anchor = Rect::new(
                    anchor_x,
                    anchor_y,
                    area.width
                        .saturating_sub(self.base.value_column.saturating_sub(1)),
                    1,
                );
                self.base.choice_select.set_anchor(anchor);
            }
        }
    }

    fn panel_state(&self, colors: &UiColors) -> PanelState {
        let border_color = if self.base.focused || self.base.filter_active {
            colors.active_border
        } else {
            colors.inactive_border
        };
        PanelState {
            is_focused: self.base.focused,
            is_filtering: self.base.filter_active,
            has_filter: !self.base.filter_text.is_empty(),
            border_color,
            filter_text: self.base.filter_text.clone(),
            match_scores: self.base.match_scores.clone(),
        }
    }
}

impl Component for FlagPanelComponent {
    type Action = FlagPanelAction;

    fn handle_focus_gained(&mut self) -> EventResult<FlagPanelAction> {
        self.base.handle_focus_gained();
        EventResult::NotHandled
    }

    fn handle_focus_lost(&mut self) -> EventResult<FlagPanelAction> {
        match self.base.handle_focus_lost() {
            FocusLostEvent::EditFinished { index, value } => {
                EventResult::Action(FlagPanelAction::EditFinished { index, value })
            }
            FocusLostEvent::Consumed => EventResult::Consumed,
            FocusLostEvent::NotHandled => EventResult::NotHandled,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<FlagPanelAction> {
        // Delegate to choice select when open
        if self.base.is_choosing() {
            match self.base.handle_choice_key(key) {
                Some(event) => return Self::map_choice_event(event),
                None => return EventResult::Consumed,
            }
        }

        // Delegate to inline editing when active
        if self.base.editing {
            return Self::map_edit_event(self.base.handle_editing_key(key));
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.base.move_up();
                EventResult::Consumed
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.base.move_down();
                EventResult::Consumed
            }
            KeyCode::Char(' ') => {
                EventResult::Action(FlagPanelAction::ToggleFlag(self.base.selected_index()))
            }
            KeyCode::Backspace => {
                EventResult::Action(FlagPanelAction::ClearFlag(self.base.selected_index()))
            }
            KeyCode::Enter => self.enter_action_for_index(self.base.selected_index()),
            _ => EventResult::NotHandled,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> EventResult<FlagPanelAction> {
        use crossterm::event::MouseEventKind;

        let col = event.column;

        match self.base.handle_mouse(event, area) {
            MouseResult::ChoiceClicked(ChoiceEvent::Selected { index, value }) => {
                EventResult::Action(FlagPanelAction::ChoiceSelected { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Cancelled { index, value }) => {
                EventResult::Action(FlagPanelAction::ChoiceCancelled { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Consumed) | MouseResult::Consumed => {
                EventResult::Consumed
            }
            MouseResult::ClickActivate(index) => {
                // Check if this is a negatable flag with column regions
                if let Some(&nc) = self.negate_cols.get(&index) {
                    let inner_left = area.x + 1;
                    if col < inner_left + 4 {
                        // Icon region → cycle through all states
                        self.enter_action_for_index(index)
                    } else if col >= nc {
                        // Negate string → set off
                        EventResult::Action(FlagPanelAction::NegBoolClick(index, Some(false)))
                    } else {
                        // Positive flag name → set on
                        EventResult::Action(FlagPanelAction::NegBoolClick(index, Some(true)))
                    }
                } else {
                    self.enter_action_for_index(index)
                }
            }
            MouseResult::ItemSelected(index) => {
                // For negate flags, also dispatch region-specific action on first click
                if let Some(&nc) = self.negate_cols.get(&index) {
                    let inner_left = area.x + 1;
                    if col < inner_left + 4 {
                        self.enter_action_for_index(index)
                    } else if col >= nc {
                        EventResult::Action(FlagPanelAction::NegBoolClick(index, Some(false)))
                    } else {
                        EventResult::Action(FlagPanelAction::NegBoolClick(index, Some(true)))
                    }
                } else {
                    EventResult::Consumed
                }
            }
            MouseResult::NotHandled => {
                // Check for scroll events that weren't in panel area
                if matches!(event.kind, MouseEventKind::ScrollUp | MouseEventKind::ScrollDown) {
                    EventResult::Consumed
                } else {
                    EventResult::NotHandled
                }
            }
        }
    }

    fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        self.base.collect_overlays()
    }
}

impl Filterable for FlagPanelComponent {
    fn apply_filter(&mut self, text: &str) -> Option<FlagPanelAction> {
        self.base.apply_filter_from_items(text);
        self.base.auto_select_next_match();
        None
    }

    fn clear_filter(&mut self) -> Option<FlagPanelAction> {
        self.base.clear_filter_state();
        None
    }

    fn set_filter_active(&mut self, active: bool) {
        self.base.filter_active = active;
    }

    fn has_active_filter(&self) -> bool {
        self.base.has_active_filter()
    }

    fn filter_text(&self) -> &str {
        self.base.filter_text()
    }
}

// ── Rendering data ──────────────────────────────────────────────────

/// External data needed by the flag panel for rendering.
/// Provided by the parent since flag values live in App.
pub struct FlagRenderData<'a> {
    pub flags: &'a [&'a usage::SpecFlag],
    pub flag_values: &'a [(String, FlagValue)],
    pub flag_defaults: &'a [Option<String>],
}

// ── FlagPanel Widget (rendering) ────────────────────────────────────

/// Props for the flag panel rendering.
/// Used internally by FlagPanelComponent::render_with_data().
struct FlagPanel<'a> {
    flags: &'a [&'a usage::SpecFlag],
    flag_values: &'a [(String, FlagValue)],
    flag_defaults: &'a [Option<String>],
    flag_index: usize,
    scroll_offset: usize,
    hovered_index: Option<usize>,
    editing: bool,
    edit_before_cursor: String,
    edit_after_cursor: String,
    choice_select_flag_index: Option<usize>,
    panel_state: &'a PanelState,
    colors: &'a UiColors,
}

impl Widget for FlagPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let ps = self.panel_state;
        let colors = self.colors;

        let title = panel_title("Flags", ps);
        let block = panel_block(title, ps);

        let mut help_entries: Vec<(usize, Line<'static>)> = Vec::new();
        let items: Vec<ListItem> = self
            .flags
            .iter()
            .enumerate()
            .map(|(i, flag)| {
                let is_selected = ps.is_focused && i == self.flag_index;
                let is_hovered = self.hovered_index == Some(i) && !is_selected;
                let is_editing = is_selected && self.editing;
                let value = self.flag_values.iter().find(|(n, _)| n == &flag.name);
                let default_val = self.flag_defaults.get(i).and_then(|d| d.as_ref());

                let ctx = ItemContext::new(&flag.name, is_selected, ps);

                let mut spans = Vec::new();

                push_selection_cursor(&mut spans, is_selected, colors);

                // Checkbox / toggle indicator
                let indicator = render_flag_indicator(value.map(|(_, v)| v), colors);
                spans.push(indicator);

                // Flag display (short + long) with highlighting
                let flag_display = flag_display_string(flag);

                let is_negated =
                    matches!(value.map(|(_, v)| v), Some(FlagValue::NegBool(Some(false))));
                let flag_name_color = if is_negated { colors.help } else { colors.flag };

                push_highlighted_name(&mut spans, &flag_display, flag_name_color, &ctx, ps, colors);

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

                // Negate indicator
                if let Some(ref negate) = flag.negate {
                    render_negate_indicator(&mut spans, negate, is_negated, &ctx, ps, colors);
                }

                // Value display for string flags
                if let Some((_, FlagValue::String(s))) = value {
                    self.render_string_value(&mut spans, s, flag, default_val, is_editing, i);
                }

                // Collect help text for overlay
                if let Some(help) = &flag.help {
                    help_entries.push((i, build_help_line(help, &ctx, ps, colors)));
                }

                let mut item = ListItem::new(Line::from(spans));
                if is_selected {
                    item = item.style(selection_bg(is_editing, colors));
                } else if is_hovered {
                    item = item.style(Style::default().bg(colors.hover_bg));
                }
                item
            })
            .collect();

        let total_flags = self.flags.len();
        let mut state = ListState::default()
            .with_selected(if ps.is_focused {
                Some(self.flag_index)
            } else {
                None
            })
            .with_offset(self.scroll_offset);

        let list = List::new(items).block(block);
        StatefulWidget::render(list, area, buf, &mut state);

        render_panel_scrollbar(buf, area, total_flags, self.scroll_offset, colors);

        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        render_help_overlays(buf, &help_entries, self.scroll_offset, inner);
    }
}

impl FlagPanel<'_> {
    fn render_string_value(
        &self,
        spans: &mut Vec<Span<'static>>,
        s: &str,
        flag: &usage::SpecFlag,
        default_val: Option<&String>,
        is_editing: bool,
        flag_idx: usize,
    ) {
        let colors = self.colors;
        spans.push(Span::styled(" = ", Style::default().fg(colors.help)));

        let is_choice_selecting = self.choice_select_flag_index == Some(flag_idx);

        if is_choice_selecting || is_editing {
            push_edit_cursor(
                spans,
                &self.edit_before_cursor,
                &self.edit_after_cursor,
                colors,
            );
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
}

/// Build the flag display string (e.g. "-v, --verbose").
pub fn flag_display_string(flag: &usage::SpecFlag) -> String {
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

/// Render the checkbox/toggle indicator for a flag value.
fn render_flag_indicator<'a>(value: Option<&FlagValue>, colors: &UiColors) -> Span<'a> {
    match value {
        Some(FlagValue::Bool(true)) => Span::styled(
            "✓ ",
            Style::default().fg(colors.arg).add_modifier(Modifier::BOLD),
        ),
        Some(FlagValue::Bool(false)) => Span::styled("○ ", Style::default().fg(colors.help)),
        Some(FlagValue::NegBool(None)) => Span::styled("○ ", Style::default().fg(colors.help)),
        Some(FlagValue::NegBool(Some(true))) => Span::styled(
            "✓ ",
            Style::default().fg(colors.arg).add_modifier(Modifier::BOLD),
        ),
        Some(FlagValue::NegBool(Some(false))) => Span::styled(
            "✗ ",
            Style::default()
                .fg(colors.required)
                .add_modifier(Modifier::BOLD),
        ),
        Some(FlagValue::Count(n)) => {
            if *n > 0 {
                Span::styled(format!("[{}] ", n), Style::default().fg(colors.count))
            } else {
                Span::styled("[0] ".to_string(), Style::default().fg(colors.help))
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
    }
}

/// Render the negate indicator spans for a negatable flag.
fn render_negate_indicator(
    spans: &mut Vec<Span<'static>>,
    negate: &str,
    is_negated: bool,
    ctx: &ItemContext,
    ps: &PanelState,
    colors: &UiColors,
) {
    let dim = !ctx.is_match && !ps.match_scores.is_empty();
    let sep_style = if dim {
        Style::default().fg(colors.help).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(colors.help)
    };
    spans.push(Span::styled(" / ", sep_style));

    let negate_style = if is_negated {
        if dim {
            Style::default().fg(colors.flag).add_modifier(Modifier::DIM)
        } else {
            Style::default().fg(colors.flag)
        }
    } else if dim {
        Style::default().fg(colors.help).add_modifier(Modifier::DIM)
    } else {
        Style::default().fg(colors.help)
    };
    spans.push(Span::styled(negate.to_string(), negate_style));
}
