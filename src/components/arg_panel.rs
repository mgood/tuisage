//! Argument panel component.
//!
//! Owns arg navigation state and handles keyboard events for moving
//! through the arg list. Value mutations (edit, choose) are emitted
//! as actions for the parent to apply.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, StatefulWidget, Widget},
};

use crate::app::ArgValue;
use crate::theme::UiColors;

use super::list_panel_base::{ChoiceEvent, EditEvent, FocusLostEvent, ListPanelBase, MouseResult};
use super::filterable::{FilterableItem, Filterable};
use super::{
    build_help_line, panel_block, panel_title, push_edit_cursor, push_highlighted_name,
    push_selection_cursor, render_help_overlays, render_panel_scrollbar, selection_bg, Component,
    EventResult, ItemContext, OverlayRequest, PanelState,
};

// ── Actions ─────────────────────────────────────────────────────────

/// Actions that the arg panel can emit to its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ArgPanelAction {
    /// Enter pressed on an arg.
    EnterRequest(ArgPanelEnterRequest),
    /// Backspace pressed: clear the arg value at the given index.
    ClearArg(usize),
    /// Choice select confirmed: apply the selected value to the arg at the given index.
    ChoiceSelected { index: usize, value: String },
    /// Choice select cancelled: keep the typed text as the arg value.
    ChoiceCancelled { index: usize, value: String },
    /// Inline edit value changed (live sync to arg_values).
    ValueChanged { index: usize, value: String },
    /// Inline edit finished (Enter/Esc committed the value).
    EditFinished { index: usize, value: String },
}

/// Panel-owned interpretation of Enter on an argument row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgPanelEnterRequest {
    ChoiceSelect {
        index: usize,
        choices: Vec<String>,
        current_value: String,
        value_column: u16,
    },
    EditOrComplete {
        index: usize,
        arg_name: String,
        current_value: String,
        value_column: u16,
    },
}

// ── ArgPanelComponent ───────────────────────────────────────────────

/// An argument list panel that owns its navigation state and handles events.
///
/// Arg *values* are owned by App (they are rebuilt on every sync_state).
/// The component receives arg data via `ArgRenderData` for rendering,
/// and emits actions for the parent to apply mutations.
pub struct ArgPanelComponent {
    base: ListPanelBase,
    enter_requests: Vec<ArgPanelEnterRequest>,
}

impl ArgPanelComponent {
    pub fn new() -> Self {
        Self {
            base: ListPanelBase::new(),
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

    /// Update filter text and recompute match scores from arg values.
    /// Cache filterable item data from arg values for standalone filtering.
    pub fn set_filterable_items_from_args(&mut self, args: &[ArgValue]) {
        let items: Vec<FilterableItem> = args
            .iter()
            .map(|a| FilterableItem {
                key: a.name.clone(),
                name_texts: vec![a.name.clone()],
                help: a.help.clone(),
            })
            .collect();
        self.base.set_filterable_items(items);
    }

    pub fn set_enter_requests_from_args(&mut self, args: &[ArgValue]) {
        self.enter_requests = args
            .iter()
            .enumerate()
            .filter_map(|(index, _)| Self::build_enter_request(index, args))
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

    pub fn choice_select_index(&self) -> Option<usize> {
        self.base.choice_select_index()
    }

    pub fn choice_select_text(&self) -> &str {
        self.base.choice_select_text()
    }

    #[cfg(test)]
    pub fn selected_choice_index(&self) -> Option<usize> {
        self.base.selected_choice_index()
    }

    pub fn set_choice_select_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.base.set_choice_select_mouse_position(pos);
    }

    pub fn handle_choice_click(
        &mut self,
        col: u16,
        row: u16,
        overlay_rect: Option<Rect>,
    ) -> EventResult<ArgPanelAction> {
        match self.base.handle_choice_click(col, row, overlay_rect) {
            MouseResult::ChoiceClicked(ChoiceEvent::Selected { index, value }) => {
                EventResult::Action(ArgPanelAction::ChoiceSelected { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Cancelled { index, value }) => {
                EventResult::Action(ArgPanelAction::ChoiceCancelled { index, value })
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

    #[cfg(test)]
    pub fn filtered_choices(&self) -> Vec<(usize, String)> {
        self.base.filtered_choices()
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

    pub fn editing_text(&self) -> &str {
        self.base.editing_text()
    }

    fn build_enter_request(index: usize, args: &[ArgValue]) -> Option<ArgPanelEnterRequest> {
        let arg = args.get(index)?;
        let current_value = arg.value.clone();
        let value_column = Self::value_column_for_arg(arg);

        if !arg.choices.is_empty() {
            Some(ArgPanelEnterRequest::ChoiceSelect {
                index,
                choices: arg.choices.clone(),
                current_value,
                value_column,
            })
        } else {
            Some(ArgPanelEnterRequest::EditOrComplete {
                index,
                arg_name: arg.name.clone(),
                current_value,
                value_column,
            })
        }
    }

    fn enter_request_for_index(&self, index: usize) -> Option<ArgPanelEnterRequest> {
        self.enter_requests.get(index).cloned()
    }

    fn value_column_for_arg(arg: &ArgValue) -> u16 {
        // Layout: border(1) + cursor "▶ "(2) + indicator "● "(2) + "<name>"(name+2) + " = "(3)
        let indicator_width = 2usize; // "● " or "○ "
        let arg_display_len = arg.name.chars().count() + 2; // includes <> or []
        (1 + 2 + indicator_width + arg_display_len + 3) as u16
    }

    // ── Event mapping ───────────────────────────────────────────────

    fn map_edit_event(event: EditEvent) -> EventResult<ArgPanelAction> {
        match event {
            EditEvent::ValueChanged { index, value } => {
                EventResult::Action(ArgPanelAction::ValueChanged { index, value })
            }
            EditEvent::EditFinished { index, value } => {
                EventResult::Action(ArgPanelAction::EditFinished { index, value })
            }
            EditEvent::Consumed => EventResult::Consumed,
        }
    }

    fn map_choice_event(event: ChoiceEvent) -> EventResult<ArgPanelAction> {
        match event {
            ChoiceEvent::Selected { index, value } => {
                EventResult::Action(ArgPanelAction::ChoiceSelected { index, value })
            }
            ChoiceEvent::Cancelled { index, value } => {
                EventResult::Action(ArgPanelAction::ChoiceCancelled { index, value })
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
        data: ArgRenderData<'_>,
    ) {
        let inner_height = area.height.saturating_sub(2) as usize;
        self.base.ensure_visible(inner_height);
        self.base.compute_hovered_index(area);
        let ps = self.panel_state(colors);

        // Derive edit state: choice_select takes priority, then inline editing
        let (editing, edit_before, edit_after, cs_arg_idx) =
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

        let panel = ArgPanel {
            arg_values: data.arg_values,
            arg_index: self.base.list_state.selected_index,
            scroll_offset: self.base.list_state.scroll as usize,
            hovered_index: self.base.hovered_index,
            editing,
            edit_before_cursor: edit_before,
            edit_after_cursor: edit_after,
            choice_select_arg_index: cs_arg_idx,
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

impl Component for ArgPanelComponent {
    type Action = ArgPanelAction;

    fn handle_focus_gained(&mut self) -> EventResult<ArgPanelAction> {
        self.base.handle_focus_gained();
        EventResult::NotHandled
    }

    fn handle_focus_lost(&mut self) -> EventResult<ArgPanelAction> {
        match self.base.handle_focus_lost() {
            FocusLostEvent::EditFinished { index, value } => {
                EventResult::Action(ArgPanelAction::EditFinished { index, value })
            }
            FocusLostEvent::Consumed => EventResult::Consumed,
            FocusLostEvent::NotHandled => EventResult::NotHandled,
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<ArgPanelAction> {
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
            KeyCode::Backspace => {
                EventResult::Action(ArgPanelAction::ClearArg(self.base.selected_index()))
            }
            KeyCode::Enter => self
                .enter_request_for_index(self.base.selected_index())
                .map(ArgPanelAction::EnterRequest)
                .map(EventResult::Action)
                .unwrap_or(EventResult::Consumed),
            _ => EventResult::NotHandled,
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> EventResult<ArgPanelAction> {
        match self.base.handle_mouse(event, area) {
            MouseResult::ChoiceClicked(ChoiceEvent::Selected { index, value }) => {
                EventResult::Action(ArgPanelAction::ChoiceSelected { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Cancelled { index, value }) => {
                EventResult::Action(ArgPanelAction::ChoiceCancelled { index, value })
            }
            MouseResult::ChoiceClicked(ChoiceEvent::Consumed) | MouseResult::Consumed => {
                EventResult::Consumed
            }
            MouseResult::ClickActivate(index) => self
                .enter_request_for_index(index)
                .map(ArgPanelAction::EnterRequest)
                .map(EventResult::Action)
                .unwrap_or(EventResult::Consumed),
            MouseResult::ItemSelected(_) => EventResult::Consumed,
            MouseResult::NotHandled => EventResult::NotHandled,
        }
    }

    fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        self.base.collect_overlays()
    }
}

impl Filterable for ArgPanelComponent {
    fn apply_filter(&mut self, text: &str) -> Option<ArgPanelAction> {
        self.base.apply_filter_from_items(text);
        self.base.auto_select_next_match();
        None
    }

    fn clear_filter(&mut self) -> Option<ArgPanelAction> {
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

/// External data needed by the arg panel for rendering.
/// Provided by the parent since arg values live in App.
pub struct ArgRenderData<'a> {
    pub arg_values: &'a [ArgValue],
}

// ── Score computation ───────────────────────────────────────────────

// ── Internal rendering widget (private) ─────────────────────────────

/// Props for the argument panel rendering.
struct ArgPanel<'a> {
    arg_values: &'a [ArgValue],
    arg_index: usize,
    scroll_offset: usize,
    hovered_index: Option<usize>,
    editing: bool,
    edit_before_cursor: String,
    edit_after_cursor: String,
    choice_select_arg_index: Option<usize>,
    panel_state: &'a PanelState,
    colors: &'a UiColors,
}

impl Widget for ArgPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let ps = self.panel_state;
        let colors = self.colors;

        let title = panel_title("Arguments", ps);
        let block = panel_block(title, ps);

        let mut help_entries: Vec<(usize, Line<'static>)> = Vec::new();
        let items: Vec<ListItem> = self
            .arg_values
            .iter()
            .enumerate()
            .map(|(i, arg_val)| {
                let is_selected = ps.is_focused && i == self.arg_index;
                let is_hovered = self.hovered_index == Some(i) && !is_selected;
                let is_editing = is_selected && self.editing;

                let ctx = ItemContext::new(&arg_val.name, is_selected, ps);

                let mut spans = Vec::new();

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

                push_highlighted_name(&mut spans, &arg_display, colors.arg, &ctx, ps, colors);

                // Value display
                spans.push(Span::styled(" = ", Style::default().fg(colors.help)));

                let is_choice_selecting = self.choice_select_arg_index == Some(i);

                if is_choice_selecting || is_editing {
                    push_edit_cursor(
                        &mut spans,
                        &self.edit_before_cursor,
                        &self.edit_after_cursor,
                        colors,
                    );
                } else if arg_val.value.is_empty() {
                    if !arg_val.choices.is_empty() {
                        let hint = arg_val.choices.join("|");
                        spans.push(Span::styled(
                            format!("<{hint}>"),
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

                // Show choices if arg has them and we're not editing
                if !arg_val.choices.is_empty()
                    && !arg_val.value.is_empty()
                    && !is_editing
                    && !is_choice_selecting
                {
                    spans.push(Span::styled(
                        format!(" [{}]", arg_val.choices.join("|")),
                        Style::default().fg(colors.choice),
                    ));
                }

                // Collect help text for overlay
                if let Some(ref help) = arg_val.help {
                    if !help.is_empty() {
                        help_entries.push((i, build_help_line(help, &ctx, ps, colors)));
                    }
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

        let total_args = self.arg_values.len();
        let mut state = ListState::default()
            .with_selected(if ps.is_focused {
                Some(self.arg_index)
            } else {
                None
            })
            .with_offset(self.scroll_offset);

        let list = List::new(items).block(block);
        StatefulWidget::render(list, area, buf, &mut state);

        render_panel_scrollbar(buf, area, total_args, self.scroll_offset, colors);

        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        render_help_overlays(buf, &help_entries, self.scroll_offset, inner);
    }
}
