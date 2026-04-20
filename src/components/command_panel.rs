//! Command tree panel component.
//!
//! Renders the command/subcommand tree as a selectable list with
//! indentation, aliases, fuzzy-match highlighting, and help overlays.
//! As a Component, owns tree state and handles navigation events.

use std::collections::HashMap;

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{List, ListItem, ListState, StatefulWidget, Widget},
};

use ratatui_interact::components::{TreeNode, TreeViewState};

use crate::app::{
    compute_tree_scores, flatten_command_tree, CmdData, FlatCommand,
    MatchScores,
};
use crate::theme::UiColors;

use super::filterable::Filterable;
use super::{
    build_help_line, find_adjacent_match, find_first_match, panel_block, panel_title,
    push_highlighted_name, push_selection_cursor, render_help_overlays, render_panel_scrollbar,
    selection_bg, Component, EventResult, ItemContext, PanelState, RenderableComponent,
};

// ── Actions ─────────────────────────────────────────────────────────

/// Actions that the command panel can emit to its parent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandPanelAction {
    /// The selected command path changed — parent should sync flags/args.
    PathChanged(Vec<String>),
}

// ── CommandPanelComponent ───────────────────────────────────────────

/// A self-contained command tree panel that owns its navigation state.
pub struct CommandPanelComponent {
    /// Tree nodes representing the full command hierarchy.
    tree_nodes: Vec<TreeNode<CmdData>>,
    /// Tree view state (selection, scroll).
    tree_state: TreeViewState,
    /// Current command path derived from the tree selection.
    path: Vec<String>,

    // Display state (set by parent)
    focused: bool,
    /// Whether filter input is actively being typed (set by wrapper, used for border color).
    filter_active: bool,
    filter_text: String,
    match_scores: HashMap<String, MatchScores>,
    hovered_index: Option<usize>,
    /// Current mouse position (set by parent, used to compute hovered_index during render).
    mouse_position: Option<(u16, u16)>,
}

impl CommandPanelComponent {
    pub fn new(tree_nodes: Vec<TreeNode<CmdData>>) -> Self {
        let tree_state = TreeViewState::new();
        let mut component = Self {
            tree_nodes,
            tree_state,
            path: Vec::new(),
            focused: false,
            filter_active: false,
            filter_text: String::new(),
            match_scores: HashMap::new(),
            hovered_index: None,
            mouse_position: None,
        };
        component.sync_path();
        component
    }

    // ── Public API for parent ───────────────────────────────────────

    /// Current command path (e.g. ["config", "set"]).
    pub fn path(&self) -> &[String] {
        &self.path
    }

    #[allow(dead_code)]
    pub fn selected_index(&self) -> usize {
        self.tree_state.selected_index
    }

    pub fn scroll_offset(&self) -> usize {
        self.tree_state.scroll as usize
    }

    pub fn total_visible(&self) -> usize {
        flatten_command_tree(&self.tree_nodes).len()
    }

    #[allow(dead_code)]
    pub fn flat_commands(&self) -> Vec<FlatCommand> {
        flatten_command_tree(&self.tree_nodes)
    }

    #[allow(dead_code)]
    pub fn tree_nodes(&self) -> &[TreeNode<CmdData>] {
        &self.tree_nodes
    }

    /// Update the focused state from the parent.
    pub fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    /// Update filter text and recompute match scores.
    pub fn set_filter(&mut self, text: &str) {
        self.filter_text = text.to_string();
        if text.is_empty() {
            self.match_scores.clear();
        } else {
            self.match_scores = compute_tree_scores(&self.tree_nodes, text);
        }
    }

    /// Update the hovered item index (from mouse position).
    #[allow(dead_code)]
    pub fn set_hovered_index(&mut self, idx: Option<usize>) {
        self.hovered_index = idx;
    }

    pub fn set_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.mouse_position = pos;
    }

    /// Compute hovered_index from mouse_position and the given panel area.
    fn compute_hovered_index(&mut self, area: Rect) {
        self.hovered_index = if self.focused {
            self.hovered_index_for_area(area)
        } else {
            None
        };
    }

    fn hovered_index_for_area(&self, area: Rect) -> Option<usize> {
        let (col, row) = self.mouse_position?;
        let inner_top = area.y + 1;
        let inner_bottom = area.y + area.height.saturating_sub(1);
        if col < area.x || col >= area.x + area.width || row < inner_top || row >= inner_bottom {
            return None;
        }
        Some((row - inner_top) as usize + self.scroll_offset())
    }

    /// Ensure the selection is visible within the given viewport height.
    pub fn ensure_visible(&mut self, viewport_height: usize) {
        self.tree_state.ensure_visible(viewport_height);
    }

    /// Select a specific item by index (e.g. from mouse click).
    /// Returns true if the path changed.
    pub fn select_item(&mut self, index: usize) -> bool {
        let total = self.total_visible();
        if index < total {
            let old_path = self.path.clone();
            self.tree_state.selected_index = index;
            self.sync_path();
            self.path != old_path
        } else {
            false
        }
    }

    /// Navigate to a specific command path. Used for tests and programmatic navigation.
    #[allow(dead_code)]
    pub fn navigate_to(&mut self, target_path: &[&str]) {
        let target_id = target_path.join(" ");
        let flat = flatten_command_tree(&self.tree_nodes);
        if let Some(idx) = flat.iter().position(|cmd| cmd.id == target_id) {
            self.tree_state.selected_index = idx;
            self.sync_path();
        }
    }

    /// Auto-select the first matching item if current selection doesn't match.
    /// Returns true if the path changed.
    pub fn auto_select_next_match(&mut self) -> bool {
        if self.match_scores.is_empty() {
            return false;
        }
        let flat = flatten_command_tree(&self.tree_nodes);
        let keys: Vec<String> = flat.iter().map(|c| c.id.clone()).collect();
        let current = self.tree_state.selected_index;
        if let Some(idx) = find_first_match(&keys, &self.match_scores, current) {
            if idx != current {
                let old_path = self.path.clone();
                self.tree_state.selected_index = idx;
                self.sync_path();
                return self.path != old_path;
            }
        }
        false
    }

    /// Set the scroll offset directly (used by tests).
    #[allow(dead_code)]
    pub fn set_scroll(&mut self, scroll: usize) {
        self.tree_state.scroll = scroll as u16;
    }

    /// Expand a tree node by ID (used by tests to set up collapsed/expanded state).
    #[allow(dead_code)]
    pub fn expand(&mut self, id: &str) {
        self.tree_state.expand(id);
    }

    // ── Internal navigation ─────────────────────────────────────────

    /// Update path from the currently selected tree node (without calling App's sync_state).
    fn sync_path(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if id.is_empty() {
                self.path = vec![];
            } else {
                self.path = id.split(' ').map(|s| s.to_string()).collect();
            }
        }
    }

    fn selected_command_id(&self) -> Option<String> {
        let flat = flatten_command_tree(&self.tree_nodes);
        flat.get(self.tree_state.selected_index)
            .map(|cmd| cmd.id.clone())
    }

    fn node_has_children(&self, id: &str) -> bool {
        fn find_in<T>(nodes: &[TreeNode<T>], id: &str) -> Option<bool> {
            for node in nodes {
                if node.id == id {
                    return Some(node.has_children());
                }
                if let Some(result) = find_in(&node.children, id) {
                    return Some(result);
                }
            }
            None
        }
        find_in(&self.tree_nodes, id).unwrap_or(false)
    }

    fn find_parent_index(&self) -> Option<usize> {
        let flat = flatten_command_tree(&self.tree_nodes);
        let selected_id = flat
            .get(self.tree_state.selected_index)
            .map(|cmd| cmd.id.clone())?;
        let parent = parent_id(&selected_id)?;
        flat.iter().position(|cmd| cmd.id == parent)
    }

    /// Move to first child of selected node (Right/l/Enter key).
    /// Returns true if the path changed.
    fn tree_expand_or_enter(&mut self) -> bool {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                let old_path = self.path.clone();
                let total = self.total_visible();
                self.tree_state.select_next(total);
                self.sync_path();
                return self.path != old_path;
            }
        }
        false
    }

    /// Move to parent node (Left/h key).
    /// Returns true if the path changed.
    fn tree_collapse_or_parent(&mut self) -> bool {
        if let Some(parent_idx) = self.find_parent_index() {
            let old_path = self.path.clone();
            self.tree_state.selected_index = parent_idx;
            self.sync_path();
            return self.path != old_path;
        }
        false
    }

    /// Move selection up. Filter-aware when scores are present.
    /// Returns true if the path changed.
    pub fn move_up(&mut self) -> bool {
        let old_path = self.path.clone();
        if !self.filter_text.is_empty() && !self.match_scores.is_empty() {
            let flat = flatten_command_tree(&self.tree_nodes);
            let keys: Vec<String> = flat.iter().map(|c| c.id.clone()).collect();
            let current = self.tree_state.selected_index;
            if let Some(idx) = find_adjacent_match(&keys, &self.match_scores, current, false) {
                self.tree_state.selected_index = idx;
                self.sync_path();
            }
        } else {
            self.tree_state.select_prev();
            self.sync_path();
        }
        self.path != old_path
    }

    /// Move selection down. Filter-aware when scores are present.
    /// Returns true if the path changed.
    pub fn move_down(&mut self) -> bool {
        let old_path = self.path.clone();
        if !self.filter_text.is_empty() && !self.match_scores.is_empty() {
            let flat = flatten_command_tree(&self.tree_nodes);
            let keys: Vec<String> = flat.iter().map(|c| c.id.clone()).collect();
            let current = self.tree_state.selected_index;
            if let Some(idx) = find_adjacent_match(&keys, &self.match_scores, current, true) {
                self.tree_state.selected_index = idx;
                self.sync_path();
            }
        } else {
            let total = self.total_visible();
            self.tree_state.select_next(total);
            self.sync_path();
        }
        self.path != old_path
    }

    /// Build a PanelState for rendering.
    fn panel_state(&self, colors: &UiColors) -> PanelState {
        let border_color = if self.focused || self.filter_active {
            colors.active_border
        } else {
            colors.inactive_border
        };
        PanelState {
            is_focused: self.focused,
            is_filtering: self.filter_active,
            has_filter: !self.filter_text.is_empty(),
            border_color,
            filter_text: self.filter_text.clone(),
            match_scores: self.match_scores.clone(),
        }
    }
}

impl Component for CommandPanelComponent {
    type Action = CommandPanelAction;

    fn handle_focus_gained(&mut self) -> EventResult<CommandPanelAction> {
        self.focused = true;
        EventResult::NotHandled
    }

    fn handle_focus_lost(&mut self) -> EventResult<CommandPanelAction> {
        self.focused = false;
        EventResult::NotHandled
    }

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<CommandPanelAction> {
        let path_changed = match key.code {
            KeyCode::Up | KeyCode::Char('k') => self.move_up(),
            KeyCode::Down | KeyCode::Char('j') => self.move_down(),
            KeyCode::Left | KeyCode::Char('h') => self.tree_collapse_or_parent(),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Enter => self.tree_expand_or_enter(),
            _ => return EventResult::NotHandled,
        };
        if path_changed {
            EventResult::Action(CommandPanelAction::PathChanged(self.path.clone()))
        } else {
            EventResult::Consumed
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> EventResult<CommandPanelAction> {
        use crossterm::event::{MouseButton, MouseEventKind};

        let row = event.row;

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let inner_top = area.y + 1; // skip border
                if row >= inner_top
                    && row < area.y + area.height.saturating_sub(1)
                    && event.column >= area.x
                    && event.column < area.x + area.width
                {
                    let clicked_offset = (row - inner_top) as usize;
                    let item_index = self.scroll_offset() + clicked_offset;
                    if self.select_item(item_index) {
                        return EventResult::Action(CommandPanelAction::PathChanged(
                            self.path.clone(),
                        ));
                    }
                    return EventResult::Consumed;
                }
                EventResult::NotHandled
            }
            MouseEventKind::ScrollUp => {
                if self.move_up() {
                    EventResult::Action(CommandPanelAction::PathChanged(self.path.clone()))
                } else {
                    EventResult::Consumed
                }
            }
            MouseEventKind::ScrollDown => {
                if self.move_down() {
                    EventResult::Action(CommandPanelAction::PathChanged(self.path.clone()))
                } else {
                    EventResult::Consumed
                }
            }
            _ => EventResult::NotHandled,
        }
    }
}

impl RenderableComponent for CommandPanelComponent {
    fn render(&mut self, area: Rect, buf: &mut Buffer, colors: &UiColors) {
        let inner_height = area.height.saturating_sub(2) as usize;
        self.ensure_visible(inner_height);
        self.compute_hovered_index(area);
        let ps = self.panel_state(colors);
        let flat_commands = flatten_command_tree(&self.tree_nodes);

        let panel = CommandPanel {
            flat_commands: &flat_commands,
            selected_index: self.tree_state.selected_index,
            hovered_index: self.hovered_index,
            scroll_offset: self.tree_state.scroll as usize,
            panel_state: &ps,
            colors,
        };
        panel.render(area, buf);
    }
}

impl Filterable for CommandPanelComponent {
    fn apply_filter(&mut self, text: &str) -> Option<CommandPanelAction> {
        self.set_filter(text);
        if self.auto_select_next_match() {
            Some(CommandPanelAction::PathChanged(self.path.clone()))
        } else {
            None
        }
    }

    fn clear_filter(&mut self) -> Option<CommandPanelAction> {
        self.filter_text.clear();
        self.match_scores.clear();
        self.filter_active = false;
        None
    }

    fn set_filter_active(&mut self, active: bool) {
        self.filter_active = active;
    }

    fn has_active_filter(&self) -> bool {
        !self.filter_text.is_empty()
    }

    fn filter_text(&self) -> &str {
        &self.filter_text
    }
}

// ── CommandPanel Widget (rendering) ─────────────────────────────────

/// Props for the command tree panel rendering.
///
/// Used internally by CommandPanelComponent::render().
struct CommandPanel<'a> {
    flat_commands: &'a [FlatCommand],
    selected_index: usize,
    hovered_index: Option<usize>,
    scroll_offset: usize,
    panel_state: &'a PanelState,
    colors: &'a UiColors,
}

impl Widget for CommandPanel<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let ps = self.panel_state;
        let colors = self.colors;

        let title = panel_title("Commands", ps);
        let block = panel_block(title, ps);

        // Build list items with indentation markers
        let mut help_entries: Vec<(usize, Line<'static>)> = Vec::new();
        let items: Vec<ListItem> = self
            .flat_commands
            .iter()
            .enumerate()
            .map(|(i, cmd)| {
                let is_selected = i == self.selected_index;
                let is_hovered = self.hovered_index == Some(i) && !is_selected;
                let ctx = ItemContext::new(&cmd.id, is_selected, ps);

                let mut spans = Vec::new();

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

                push_highlighted_name(&mut spans, &name_text, colors.command, &ctx, ps, colors);

                // Collect help text for overlay rendering
                if let Some(help) = &cmd.help {
                    help_entries.push((i, build_help_line(help, &ctx, ps, colors)));
                }

                let mut item = ListItem::new(Line::from(spans));
                if is_selected {
                    item = item.style(selection_bg(false, colors));
                } else if is_hovered {
                    item = item.style(Style::default().bg(colors.hover_bg));
                }
                item
            })
            .collect();

        let mut state = ListState::default()
            .with_selected(if ps.is_focused {
                Some(self.selected_index)
            } else {
                None
            })
            .with_offset(self.scroll_offset);

        let list = List::new(items).block(block);
        StatefulWidget::render(list, area, buf, &mut state);

        // Scrollbar
        render_panel_scrollbar(buf, area, self.flat_commands.len(), self.scroll_offset, colors);

        // Render right-aligned help text overlays
        let inner = area.inner(ratatui::layout::Margin::new(1, 1));
        render_help_overlays(buf, &help_entries, self.scroll_offset, inner);
    }
}

// ── Helper functions ────────────────────────────────────────────────

fn parent_id(id: &str) -> Option<String> {
    if id.is_empty() {
        None
    } else if let Some(pos) = id.rfind(' ') {
        Some(id[..pos].to_string())
    } else {
        Some(String::new())
    }
}
