use nucleo_matcher::pattern::{Atom, AtomKind, CaseMatching};
use nucleo_matcher::{Config, Matcher};
use ratatui::layout::Rect;
use ratatui_interact::components::{InputState, ListPickerState, TreeNode, TreeViewState};
use ratatui_interact::state::FocusManager;
use ratatui_interact::traits::ClickRegionRegistry;
use ratatui_themes::{ThemeName, ThemePalette};
use usage::{Spec, SpecCommand, SpecFlag};

/// Actions that the event loop should take after handling a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Accept,
}

/// Which panel currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Focus {
    Commands,
    Flags,
    Args,
    Preview,
}

/// Tracks the value set for a flag.
#[derive(Debug, Clone, PartialEq)]
pub enum FlagValue {
    /// Boolean flag toggled on/off.
    Bool(bool),
    /// Flag with a string value.
    String(String),
    /// Count flag (e.g., -vvv).
    Count(u32),
}

/// State for one positional argument's user-entered value.
#[derive(Debug, Clone)]
pub struct ArgValue {
    pub name: String,
    pub value: String,
    pub required: bool,
    pub choices: Vec<String>,
}

/// Data stored in each tree node for a command.
#[derive(Debug, Clone)]
pub struct CmdData {
    pub name: String,
    pub help: Option<String>,
    pub aliases: Vec<String>,
}

/// Main application state.
pub struct App {
    pub spec: Spec,

    /// Current color theme.
    pub theme_name: ThemeName,

    /// Breadcrumb path of subcommand names derived from the tree selection.
    /// Empty means we're at the root command.
    pub command_path: Vec<String>,

    /// Flag values keyed by flag name, per command path depth.
    /// The key is the full command path joined by space.
    pub flag_values: std::collections::HashMap<String, Vec<(String, FlagValue)>>,

    /// Arg values for the current command.
    pub arg_values: Vec<ArgValue>,

    /// Focus manager for Tab navigation between panels.
    pub focus_manager: FocusManager<Focus>,

    /// Whether we are currently editing a text field (flag value or arg value).
    pub editing: bool,

    /// Input state for the filter bar (fzf-style matching).
    pub filter_input: InputState,

    /// Whether the filter input is active.
    pub filtering: bool,

    /// Tree nodes representing the full command hierarchy.
    pub command_tree_nodes: Vec<TreeNode<CmdData>>,

    /// Tree view state (selection, collapsed set, scroll).
    pub command_tree_state: TreeViewState,

    /// ListPickerState for the flag list.
    pub flag_list_state: ListPickerState,

    /// ListPickerState for the arg list.
    pub arg_list_state: ListPickerState,

    /// InputState for editing flag/arg values.
    pub edit_input: InputState,

    /// Click region registry for mouse hit-testing.
    pub click_regions: ClickRegionRegistry<Focus>,
}

impl App {
    pub fn new(spec: Spec) -> Self {
        Self::with_theme(spec, ThemeName::default())
    }

    pub fn with_theme(spec: Spec, theme_name: ThemeName) -> Self {
        let tree_nodes = build_command_tree(&spec);
        let mut tree_state = TreeViewState::new();
        // Collapse all nodes that have children, so the initial view
        // shows only the top-level commands.
        collapse_parents_with_children(&tree_nodes, &mut tree_state);

        let mut app = Self {
            spec,
            theme_name,
            command_path: Vec::new(),
            flag_values: std::collections::HashMap::new(),
            arg_values: Vec::new(),
            focus_manager: FocusManager::new(),
            editing: false,
            filter_input: InputState::empty(),
            filtering: false,
            command_tree_nodes: tree_nodes,
            command_tree_state: tree_state,
            flag_list_state: ListPickerState::new(0),
            arg_list_state: ListPickerState::new(0),
            edit_input: InputState::empty(),
            click_regions: ClickRegionRegistry::new(),
        };
        app.sync_state();
        app
    }

    /// Rebuild focus manager based on what panels are available.
    fn rebuild_focus_manager(&mut self) {
        let had_focus = !self.focus_manager.is_empty();
        let current = self.focus();
        self.focus_manager.clear();
        if self.has_any_commands() {
            self.focus_manager.register(Focus::Commands);
        }
        if !self.visible_flags().is_empty() {
            self.focus_manager.register(Focus::Flags);
        }
        if !self.arg_values.is_empty() {
            self.focus_manager.register(Focus::Args);
        }
        self.focus_manager.register(Focus::Preview);
        // Only restore previous focus if we had one before;
        // otherwise let FocusManager default to the first registered element
        if had_focus {
            self.focus_manager.set(current);
        }
    }

    /// Get the current theme palette.
    pub fn palette(&self) -> ThemePalette {
        self.theme_name.palette()
    }

    /// Cycle to the next theme.
    pub fn next_theme(&mut self) {
        self.theme_name = self.theme_name.next();
    }

    /// Cycle to the previous theme.
    #[allow(dead_code)]
    pub fn prev_theme(&mut self) {
        self.theme_name = self.theme_name.prev();
    }

    /// Get the current focus panel.
    pub fn focus(&self) -> Focus {
        self.focus_manager
            .current()
            .copied()
            .unwrap_or(Focus::Preview)
    }

    /// Set the focus to a specific panel.
    pub fn set_focus(&mut self, panel: Focus) {
        self.focus_manager.set(panel);
    }

    // --- Convenience accessors for indices ---

    pub fn command_index(&self) -> usize {
        self.command_tree_state.selected_index
    }

    #[allow(dead_code)]
    pub fn set_command_index(&mut self, idx: usize) {
        self.command_tree_state.selected_index = idx;
        self.sync_command_path_from_tree();
    }

    pub fn flag_index(&self) -> usize {
        self.flag_list_state.selected_index
    }

    pub fn set_flag_index(&mut self, idx: usize) {
        self.flag_list_state.select(idx);
    }

    pub fn arg_index(&self) -> usize {
        self.arg_list_state.selected_index
    }

    pub fn set_arg_index(&mut self, idx: usize) {
        self.arg_list_state.select(idx);
    }

    pub fn command_scroll(&self) -> usize {
        self.command_tree_state.scroll as usize
    }

    pub fn flag_scroll(&self) -> usize {
        self.flag_list_state.scroll as usize
    }

    pub fn arg_scroll(&self) -> usize {
        self.arg_list_state.scroll as usize
    }

    /// Get the filter text.
    pub fn filter(&self) -> &str {
        self.filter_input.text()
    }

    /// Get the current SpecCommand based on the command_path.
    pub fn current_command(&self) -> &SpecCommand {
        let mut cmd = &self.spec.cmd;
        for name in &self.command_path {
            if let Some(sub) = cmd.find_subcommand(name) {
                cmd = sub;
            } else {
                break;
            }
        }
        cmd
    }

    /// Whether the spec has any subcommands at all (for focus manager).
    fn has_any_commands(&self) -> bool {
        !self.spec.cmd.subcommands.is_empty()
    }

    /// Returns the visible (non-hidden) subcommands of the current command,
    /// optionally filtered by the current filter string when focus is Commands.
    #[cfg(test)]
    pub fn visible_subcommands(&self) -> Vec<(&String, &SpecCommand)> {
        let cmd = self.current_command();
        let items: Vec<(&String, &SpecCommand)> =
            cmd.subcommands.iter().filter(|(_, c)| !c.hide).collect();

        if self.filtering && !self.filter().is_empty() && self.focus() == Focus::Commands {
            let filter_lower = self.filter().to_lowercase();
            items
                .into_iter()
                .filter(|(name, c)| {
                    fuzzy_match(&name.to_lowercase(), &filter_lower)
                        || c.aliases
                            .iter()
                            .any(|a| fuzzy_match(&a.to_lowercase(), &filter_lower))
                        || c.help
                            .as_ref()
                            .map(|h| fuzzy_match(&h.to_lowercase(), &filter_lower))
                            .unwrap_or(false)
                })
                .collect()
        } else {
            items
        }
    }

    /// Returns the visible (non-hidden) flags of the current command,
    /// including global flags from ancestors, optionally filtered when focus is Flags.
    pub fn visible_flags(&self) -> Vec<&SpecFlag> {
        let cmd = self.current_command();
        let mut flags: Vec<&SpecFlag> = cmd.flags.iter().filter(|f| !f.hide).collect();

        // Include global flags from the root spec
        for flag in &self.spec.cmd.flags {
            if flag.global && !flag.hide {
                // Don't duplicate if already present
                if !flags.iter().any(|f| f.name == flag.name) {
                    flags.push(flag);
                }
            }
        }

        // No filtering here - rendering will apply subdued styling to non-matches
        flags
    }

    /// Synchronize internal state (arg_values, flag_values) when navigating to a new command.
    pub fn sync_state(&mut self) {
        let cmd = self.current_command();

        // Initialize arg values for the current command
        self.arg_values = cmd
            .args
            .iter()
            .filter(|a| !a.hide)
            .map(|a| {
                let choices = a
                    .choices
                    .as_ref()
                    .map(|c| c.choices.clone())
                    .unwrap_or_default();
                let default = a.default.first().cloned().unwrap_or_default();
                ArgValue {
                    name: a.name.clone(),
                    value: default,
                    required: a.required,
                    choices,
                }
            })
            .collect();

        // Initialize flag values for the current command path if not already set
        let path_key = self.command_path_key();
        if !self.flag_values.contains_key(&path_key) {
            let flags = self.visible_flags_snapshot();
            let values: Vec<(String, FlagValue)> = flags
                .iter()
                .map(|f| {
                    let val = if f.count {
                        FlagValue::Count(0)
                    } else if f.arg.is_some() {
                        let default = f.default.first().cloned().unwrap_or_default();
                        FlagValue::String(default)
                    } else {
                        FlagValue::Bool(false)
                    };
                    (f.name.clone(), val)
                })
                .collect();
            self.flag_values.insert(path_key, values);
        }

        // Update list picker states with correct totals
        self.flag_list_state
            .set_total(self.current_flag_values().len());
        self.arg_list_state.set_total(self.arg_values.len());

        // Rebuild focus manager with available panels
        self.rebuild_focus_manager();
    }

    /// Snapshot of visible flags (owned) for initialization purposes.
    fn visible_flags_snapshot(&self) -> Vec<SpecFlag> {
        self.visible_flags().into_iter().cloned().collect()
    }

    /// Current command path as a key string.
    fn command_path_key(&self) -> String {
        self.command_path.join(" ")
    }

    /// Get the flag values for the current command.
    pub fn current_flag_values(&self) -> &[(String, FlagValue)] {
        let key = self.command_path_key();
        self.flag_values
            .get(&key)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Get a mutable reference to the flag values for the current command.
    pub fn current_flag_values_mut(&mut self) -> &mut Vec<(String, FlagValue)> {
        let key = self.command_path_key();
        self.flag_values.entry(key).or_default()
    }

    // --- Tree view helpers ---

    /// Get the total number of visible nodes in the command tree.
    pub fn total_visible_commands(&self) -> usize {
        count_visible(&self.command_tree_nodes, &self.command_tree_state)
    }

    /// Get the number of matching nodes when filtering is active.
    pub fn matching_commands_count(&self) -> usize {
        if self.filtering && !self.filter().is_empty() && self.focus() == Focus::Commands {
            let scores = self.compute_tree_match_scores();
            scores.values().filter(|&&score| score > 0).count()
        } else {
            self.total_visible_commands()
        }
    }

    /// Compute match scores for all tree nodes when filtering.
    /// Returns a map of node ID → score (0 for non-matches).
    pub fn compute_tree_match_scores(&self) -> std::collections::HashMap<String, u32> {
        let pattern = self.filter();
        if pattern.is_empty() {
            return std::collections::HashMap::new();
        }
        compute_tree_scores(&self.command_tree_nodes, pattern)
    }

    /// Compute match scores for all flags when filtering.
    /// Returns a map of flag name → score (0 for non-matches).
    pub fn compute_flag_match_scores(&self) -> std::collections::HashMap<String, u32> {
        let pattern = self.filter();
        if pattern.is_empty() {
            return std::collections::HashMap::new();
        }

        let flags = self.visible_flags();
        let mut scores = std::collections::HashMap::new();

        for flag in flags {
            let mut temp_matcher = Matcher::new(Config::DEFAULT);
            let name_score = fuzzy_match_score(&flag.name, pattern, &mut temp_matcher);
            let long_score = flag
                .long
                .iter()
                .map(|l| fuzzy_match_score(l, pattern, &mut temp_matcher))
                .max()
                .unwrap_or(0);
            let short_score = flag
                .short
                .iter()
                .map(|s| {
                    let s_str = s.to_string();
                    fuzzy_match_score(&s_str, pattern, &mut temp_matcher)
                })
                .max()
                .unwrap_or(0);
            let help_score = flag
                .help
                .as_ref()
                .map(|h| fuzzy_match_score(h, pattern, &mut temp_matcher))
                .unwrap_or(0);

            let best_score = name_score.max(long_score).max(short_score).max(help_score);
            scores.insert(flag.name.clone(), best_score);
        }

        scores
    }

    /// Get the ID of the currently selected tree node.
    pub fn selected_command_id(&self) -> Option<String> {
        let visible = flatten_visible_ids(&self.command_tree_nodes, &self.command_tree_state);
        visible.get(self.command_tree_state.selected_index).cloned()
    }

    /// Update command_path based on the currently selected tree node.
    pub fn sync_command_path_from_tree(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if id.is_empty() {
                self.command_path = vec![];
            } else {
                self.command_path = id.split(' ').map(|s| s.to_string()).collect();
            }
        }
        self.sync_state();
    }

    /// Find a tree node by ID and check if it has children.
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
        find_in(&self.command_tree_nodes, id).unwrap_or(false)
    }

    /// Find the parent node index in the flattened visible list.
    fn find_parent_index(&self) -> Option<usize> {
        let visible = flatten_visible_ids(&self.command_tree_nodes, &self.command_tree_state);
        let selected_id = visible.get(self.command_tree_state.selected_index)?;
        let parent = parent_id(selected_id)?;
        visible.iter().position(|id| *id == parent)
    }

    /// Toggle expand/collapse of the selected tree node (Enter key on Commands).
    pub fn tree_toggle_selected(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                self.command_tree_state.toggle_collapsed(&id);
            }
        }
    }

    /// Expand selected node, or move to first child if already expanded (Right/l key).
    pub fn tree_expand_or_enter(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                if self.command_tree_state.is_collapsed(&id) {
                    self.command_tree_state.expand(&id);
                } else {
                    // Already expanded, move to first child
                    let total = self.total_visible_commands();
                    self.command_tree_state.select_next(total);
                    self.sync_command_path_from_tree();
                }
            }
        }
    }

    /// Collapse selected node, or move to parent if already collapsed/leaf (Left/h key).
    pub fn tree_collapse_or_parent(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) && !self.command_tree_state.is_collapsed(&id) {
                self.command_tree_state.collapse(&id);
            } else {
                // Move to parent
                if let Some(parent_idx) = self.find_parent_index() {
                    self.command_tree_state.selected_index = parent_idx;
                    self.sync_command_path_from_tree();
                }
            }
        }
    }

    /// Expand the selected node (for right-click).
    pub fn tree_expand_selected(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                self.command_tree_state.expand(&id);
            }
        }
    }

    /// Navigate to a specific command path in the tree. Expands all ancestors
    /// and selects the target node. Used for tests and programmatic navigation.
    #[allow(dead_code)]
    pub fn navigate_to_command(&mut self, path: &[&str]) {
        // Expand all ancestors
        for i in 1..path.len() {
            let ancestor_id = path[..i].join(" ");
            self.command_tree_state.expand(&ancestor_id);
        }
        let target_id = path.join(" ");
        let visible = flatten_visible_ids(&self.command_tree_nodes, &self.command_tree_state);
        if let Some(idx) = visible.iter().position(|id| *id == target_id) {
            self.command_tree_state.selected_index = idx;
            self.sync_command_path_from_tree();
        }
    }

    /// Backward-compatible: expand selected node and select first child.
    /// Equivalent to the old "navigate into selected subcommand".
    #[allow(dead_code)]
    pub fn navigate_into_selected(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                self.command_tree_state.expand(&id);
                // Move selection to first child
                let total = self.total_visible_commands();
                self.command_tree_state.select_next(total);
                self.sync_command_path_from_tree();
            }
        }
    }

    /// Backward-compatible: move selection to parent node.
    /// Equivalent to the old "navigate up".
    pub fn navigate_up(&mut self) {
        if let Some(parent_idx) = self.find_parent_index() {
            self.command_tree_state.selected_index = parent_idx;
            self.sync_command_path_from_tree();
        }
    }

    /// Handle a mouse event and return the resulting Action.
    pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) -> Action {
        use crossterm::event::{MouseButton, MouseEventKind};

        let col = event.column;
        let row = event.row;

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // Use click region registry for hit-testing
                if let Some(&clicked_panel) = self.click_regions.handle_click(col, row) {
                    // Finish any in-progress editing before switching focus or item
                    if self.editing {
                        self.finish_editing();
                    }
                    let was_focused = self.focus() == clicked_panel;
                    self.set_focus(clicked_panel);

                    match clicked_panel {
                        Focus::Commands => {
                            // Calculate which item was clicked based on the region
                            if let Some(area) = self.command_area() {
                                let inner_top = area.y + 1; // border
                                if row >= inner_top {
                                    let clicked_offset = (row - inner_top) as usize;
                                    let item_index = self.command_scroll() + clicked_offset;
                                    let total = self.total_visible_commands();
                                    if item_index < total {
                                        if was_focused && self.command_index() == item_index {
                                            // Second click on same item: toggle expand/collapse
                                            self.tree_toggle_selected();
                                        } else {
                                            self.command_tree_state.selected_index = item_index;
                                            self.sync_command_path_from_tree();
                                        }
                                    }
                                }
                            }
                        }
                        Focus::Flags => {
                            if let Some(area) = self.flag_area() {
                                let inner_top = area.y + 1;
                                if row >= inner_top {
                                    let clicked_offset = (row - inner_top) as usize;
                                    let item_index = self.flag_scroll() + clicked_offset;
                                    let len = self.current_flag_values().len();
                                    if item_index < len {
                                        if was_focused && self.flag_index() == item_index {
                                            return self.handle_enter();
                                        } else {
                                            self.set_flag_index(item_index);
                                        }
                                    }
                                }
                            }
                        }
                        Focus::Args => {
                            if let Some(area) = self.arg_area() {
                                let inner_top = area.y + 1;
                                if row >= inner_top {
                                    let clicked_offset = (row - inner_top) as usize;
                                    let item_index = self.arg_scroll() + clicked_offset;
                                    let len = self.arg_values.len();
                                    if item_index < len {
                                        if was_focused && self.arg_index() == item_index {
                                            return self.handle_enter();
                                        } else {
                                            self.set_arg_index(item_index);
                                        }
                                    }
                                }
                            }
                        }
                        Focus::Preview => {
                            if was_focused {
                                return Action::Accept;
                            }
                        }
                    }
                    return Action::None;
                }
                Action::None
            }
            MouseEventKind::Down(MouseButton::Right) => {
                // Right-click: expand tree node in commands area
                if let Some(&Focus::Commands) = self.click_regions.handle_click(col, row) {
                    if self.focus() == Focus::Commands {
                        self.tree_expand_selected();
                    }
                }
                Action::None
            }
            MouseEventKind::ScrollUp => {
                self.scroll_up_in_focused();
                Action::None
            }
            MouseEventKind::ScrollDown => {
                self.scroll_down_in_focused();
                Action::None
            }
            MouseEventKind::Up(MouseButton::Left) => Action::None,
            _ => Action::None,
        }
    }

    /// Get the stored area for the command panel (from click regions).
    fn command_area(&self) -> Option<Rect> {
        self.click_regions
            .regions()
            .iter()
            .find(|r| r.data == Focus::Commands)
            .map(|r| r.area)
    }

    /// Get the stored area for the flag panel.
    fn flag_area(&self) -> Option<Rect> {
        self.click_regions
            .regions()
            .iter()
            .find(|r| r.data == Focus::Flags)
            .map(|r| r.area)
    }

    /// Get the stored area for the arg panel.
    fn arg_area(&self) -> Option<Rect> {
        self.click_regions
            .regions()
            .iter()
            .find(|r| r.data == Focus::Args)
            .map(|r| r.area)
    }

    /// Scroll up in the currently focused list.
    fn scroll_up_in_focused(&mut self) {
        self.move_up();
    }

    /// Scroll down in the currently focused list.
    fn scroll_down_in_focused(&mut self) {
        self.move_down();
    }

    /// Ensure the scroll offset keeps the selected index visible within the given viewport height.
    pub fn ensure_visible(&mut self, panel: Focus, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        match panel {
            Focus::Commands => {
                self.command_tree_state.ensure_visible(viewport_height);
            }
            Focus::Flags => {
                self.flag_list_state.ensure_visible(viewport_height);
            }
            Focus::Args => {
                self.arg_list_state.ensure_visible(viewport_height);
            }
            Focus::Preview => {}
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::KeyCode;

        // If we're editing a text field, handle that separately
        if self.editing {
            return self.handle_editing_key(key);
        }

        // If we're in filter mode, handle filter input
        if self.filtering {
            return self.handle_filter_key(key);
        }

        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Backspace => {
                // Decrement count flags
                if self.focus() == Focus::Flags {
                    self.handle_decrement();
                }
                Action::None
            }
            KeyCode::Char('T') => {
                self.next_theme();
                Action::None
            }
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter_input.clear();
                Action::None
            }
            KeyCode::Tab => {
                self.focus_manager.next();
                Action::None
            }
            KeyCode::BackTab => {
                self.focus_manager.prev();
                Action::None
            }
            KeyCode::Esc => {
                if self.focus() == Focus::Commands {
                    // In tree view: collapse or move to parent, quit if at top level
                    if let Some(id) = self.selected_command_id() {
                        // If expanded and has children, collapse
                        if self.node_has_children(&id) && !self.command_tree_state.is_collapsed(&id)
                        {
                            self.command_tree_state.collapse(&id);
                            return Action::None;
                        }
                        // Move to parent if one exists
                        if let Some(parent_idx) = self.find_parent_index() {
                            self.command_tree_state.selected_index = parent_idx;
                            self.sync_command_path_from_tree();
                            return Action::None;
                        }
                        // No parent means we're at a top-level command, quit
                        return Action::Quit;
                    }
                    Action::Quit
                } else if !self.command_path.is_empty() {
                    // Other panels: navigate up in the tree
                    self.navigate_up();
                    Action::None
                } else {
                    Action::Quit
                }
            }
            KeyCode::Enter => self.handle_enter(),
            KeyCode::Up | KeyCode::Char('k') => {
                self.move_up();
                Action::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.move_down();
                Action::None
            }
            KeyCode::Char(' ') => {
                self.handle_space();
                Action::None
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if self.focus() == Focus::Commands {
                    self.tree_collapse_or_parent();
                }
                Action::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.focus() == Focus::Commands {
                    self.tree_expand_or_enter();
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_editing_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc => {
                self.finish_editing();
                Action::None
            }
            KeyCode::Enter => {
                self.finish_editing();
                Action::None
            }
            KeyCode::Backspace => {
                self.edit_input.delete_char_backward();
                self.sync_edit_to_value();
                Action::None
            }
            KeyCode::Delete => {
                self.edit_input.delete_char_forward();
                self.sync_edit_to_value();
                Action::None
            }
            KeyCode::Left => {
                self.edit_input.move_left();
                Action::None
            }
            KeyCode::Right => {
                self.edit_input.move_right();
                Action::None
            }
            KeyCode::Home => {
                self.edit_input.move_home();
                Action::None
            }
            KeyCode::End => {
                self.edit_input.move_end();
                Action::None
            }
            KeyCode::Char(c) => {
                self.edit_input.insert_char(c);
                self.sync_edit_to_value();
                Action::None
            }
            _ => Action::None,
        }
    }

    /// Sync the edit_input text back to the underlying flag/arg value.
    fn sync_edit_to_value(&mut self) {
        let text = self.edit_input.text.clone();
        match self.focus() {
            Focus::Flags => {
                let flag_idx = self.flag_index();
                let values = self.current_flag_values_mut();
                if let Some((_, FlagValue::String(ref mut s))) = values.get_mut(flag_idx) {
                    *s = text;
                }
            }
            Focus::Args => {
                let arg_idx = self.arg_index();
                if let Some(arg) = self.arg_values.get_mut(arg_idx) {
                    arg.value = text;
                }
            }
            _ => {}
        }
    }

    /// Start editing: populate edit_input from current value.
    pub fn start_editing(&mut self) {
        self.editing = true;
        let current_text = match self.focus() {
            Focus::Flags => {
                let flag_idx = self.flag_index();
                self.current_flag_values()
                    .get(flag_idx)
                    .and_then(|(_, v)| match v {
                        FlagValue::String(s) => Some(s.clone()),
                        _ => None,
                    })
                    .unwrap_or_default()
            }
            Focus::Args => {
                let arg_idx = self.arg_index();
                self.arg_values
                    .get(arg_idx)
                    .map(|a| a.value.clone())
                    .unwrap_or_default()
            }
            _ => String::new(),
        };
        self.edit_input.set_text(current_text);
    }

    /// Stop editing and sync final value.
    pub fn finish_editing(&mut self) {
        self.sync_edit_to_value();
        self.editing = false;
    }

    fn handle_filter_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc => {
                self.filtering = false;
                self.filter_input.clear();
                Action::None
            }
            KeyCode::Enter => {
                self.filtering = false;
                // Keep the filter active to show filtered results
                Action::None
            }
            KeyCode::Tab => {
                // Allow switching focus while filtering — stop filtering first
                self.filtering = false;
                self.filter_input.clear();
                self.focus_manager.next();
                Action::None
            }
            KeyCode::BackTab => {
                self.filtering = false;
                self.filter_input.clear();
                self.focus_manager.prev();
                Action::None
            }
            KeyCode::Backspace => {
                self.filter_input.delete_char_backward();
                // Auto-select next matching item if current doesn't match
                self.auto_select_next_match();
                Action::None
            }
            KeyCode::Char(c) => {
                self.filter_input.insert_char(c);
                // Auto-select next matching item if current doesn't match
                self.auto_select_next_match();
                Action::None
            }
            KeyCode::Up => {
                self.move_up();
                Action::None
            }
            KeyCode::Down => {
                self.move_down();
                Action::None
            }
            _ => Action::None,
        }
    }

    fn move_up(&mut self) {
        match self.focus() {
            Focus::Commands => {
                self.command_tree_state.select_prev();
                self.sync_command_path_from_tree();
            }
            Focus::Flags => {
                self.flag_list_state.select_prev();
            }
            Focus::Args => {
                self.arg_list_state.select_prev();
            }
            Focus::Preview => {}
        }
    }

    fn move_down(&mut self) {
        match self.focus() {
            Focus::Commands => {
                let total = self.total_visible_commands();
                self.command_tree_state.select_next(total);
                self.sync_command_path_from_tree();
            }
            Focus::Flags => {
                self.flag_list_state.select_next();
            }
            Focus::Args => {
                self.arg_list_state.select_next();
            }
            Focus::Preview => {}
        }
    }

    fn handle_enter(&mut self) -> Action {
        match self.focus() {
            Focus::Commands => {
                // Toggle expand/collapse of the selected tree node
                self.tree_toggle_selected();
                Action::None
            }
            Focus::Flags => {
                let flag_idx = self.flag_index();

                // Check if the flag has choices before mutably borrowing
                let maybe_choices: Option<Vec<String>> = {
                    let flags = self.visible_flags();
                    flags.get(flag_idx).and_then(|flag| {
                        flag.arg
                            .as_ref()
                            .and_then(|a| a.choices.as_ref())
                            .map(|c| c.choices.clone())
                    })
                };

                // Toggle bool flags, start editing string flags
                let values = self.current_flag_values_mut();
                if let Some((_, value)) = values.get_mut(flag_idx) {
                    match value {
                        FlagValue::Bool(b) => *b = !*b,
                        FlagValue::Count(c) => *c += 1,
                        FlagValue::String(s) => {
                            if let Some(choices) = maybe_choices {
                                // Cycle through choices
                                let idx = choices
                                    .iter()
                                    .position(|c| c == s.as_str())
                                    .map(|i| (i + 1) % choices.len())
                                    .unwrap_or(0);
                                *s = choices[idx].clone();
                            } else {
                                self.start_editing();
                            }
                        }
                    }
                }
                Action::None
            }
            Focus::Args => {
                let arg_idx = self.arg_index();
                let arg = &self.arg_values[arg_idx];
                if !arg.choices.is_empty() {
                    // Cycle through choices
                    let current = arg.value.clone();
                    let choices = arg.choices.clone();
                    let idx = choices
                        .iter()
                        .position(|c| c == &current)
                        .map(|i| (i + 1) % choices.len())
                        .unwrap_or(0);
                    self.arg_values[arg_idx].value = choices[idx].clone();
                } else {
                    self.start_editing();
                }
                Action::None
            }
            Focus::Preview => Action::Accept,
        }
    }

    fn handle_space(&mut self) {
        if self.focus() == Focus::Flags {
            let flag_idx = self.flag_index();
            let values = self.current_flag_values_mut();
            if let Some((_, value)) = values.get_mut(flag_idx) {
                match value {
                    FlagValue::Bool(b) => *b = !*b,
                    FlagValue::Count(c) => *c += 1,
                    _ => {}
                }
            }
        }
    }

    /// Auto-select the next matching item if the current selection doesn't match the filter.
    fn auto_select_next_match(&mut self) {
        match self.focus() {
            Focus::Commands => {
                let scores = self.compute_tree_match_scores();
                let visible =
                    flatten_visible_ids(&self.command_tree_nodes, &self.command_tree_state);
                let current_idx = self.command_tree_state.selected_index;

                // Check if current selection matches
                if let Some(id) = visible.get(current_idx) {
                    if let Some(&score) = scores.get(id) {
                        if score > 0 {
                            // Current selection matches, keep it
                            return;
                        }
                    }
                }

                // Current doesn't match, find next matching item
                for (idx, id) in visible.iter().enumerate() {
                    if let Some(&score) = scores.get(id) {
                        if score > 0 {
                            self.command_tree_state.selected_index = idx;
                            self.sync_command_path_from_tree();
                            return;
                        }
                    }
                }

                // No matches found, stay at current position
            }
            Focus::Flags => {
                let scores = self.compute_flag_match_scores();
                let flags = self.visible_flags();
                let current_idx = self.flag_list_state.selected_index;

                // Check if current selection matches
                if let Some(flag) = flags.get(current_idx) {
                    if let Some(&score) = scores.get(&flag.name) {
                        if score > 0 {
                            // Current selection matches, keep it
                            return;
                        }
                    }
                }

                // Current doesn't match, find next matching item
                for (idx, flag) in flags.iter().enumerate() {
                    if let Some(&score) = scores.get(&flag.name) {
                        if score > 0 {
                            self.flag_list_state.select(idx);
                            return;
                        }
                    }
                }

                // No matches found, stay at current position
            }
            _ => {}
        }
    }

    /// Decrement a count flag (floor at 0).
    fn handle_decrement(&mut self) {
        let flag_idx = self.flag_index();
        let values = self.current_flag_values_mut();
        if let Some((_, FlagValue::Count(c))) = values.get_mut(flag_idx) {
            *c = c.saturating_sub(1);
        }
    }

    /// Build the full command string from the current state.
    pub fn build_command(&self) -> String {
        let mut parts: Vec<String> = Vec::new();

        // Binary name
        let bin = if self.spec.bin.is_empty() {
            &self.spec.name
        } else {
            &self.spec.bin
        };
        parts.push(bin.clone());

        // Gather global flag values (from root command path)
        let root_key = String::new();
        if let Some(root_flags) = self.flag_values.get(&root_key) {
            for (name, value) in root_flags {
                if let Some(flag_str) = self.format_flag_value(name, value, &self.spec.cmd.flags) {
                    parts.push(flag_str);
                }
            }
        }

        // Add subcommand path
        let mut cmd = &self.spec.cmd;
        for (i, name) in self.command_path.iter().enumerate() {
            parts.push(name.clone());

            if let Some(sub) = cmd.find_subcommand(name) {
                cmd = sub;

                // Add flag values for this level
                let path_key = self.command_path[..=i].join(" ");
                if let Some(level_flags) = self.flag_values.get(&path_key) {
                    for (fname, fvalue) in level_flags {
                        // Skip global flags, they were already added
                        let is_global = self
                            .spec
                            .cmd
                            .flags
                            .iter()
                            .any(|f| f.global && f.name == *fname);
                        if is_global {
                            continue;
                        }
                        if let Some(flag_str) = self.format_flag_value(fname, fvalue, &cmd.flags) {
                            parts.push(flag_str);
                        }
                    }
                }
            }
        }

        // Add positional arg values
        for arg in &self.arg_values {
            if !arg.value.is_empty() {
                // Quote the value if it contains spaces
                if arg.value.contains(' ') {
                    parts.push(format!("\"{}\"", arg.value));
                } else {
                    parts.push(arg.value.clone());
                }
            }
        }

        parts.join(" ")
    }

    fn format_flag_value(
        &self,
        name: &str,
        value: &FlagValue,
        flags: &[SpecFlag],
    ) -> Option<String> {
        let flag = flags.iter().find(|f| f.name == name);
        // Also check global flags
        let flag = flag.or_else(|| {
            self.spec
                .cmd
                .flags
                .iter()
                .find(|f| f.name == name && f.global)
        });

        let flag = flag?;

        match value {
            FlagValue::Bool(true) => {
                let prefix = if let Some(long) = flag.long.first() {
                    format!("--{long}")
                } else if let Some(short) = flag.short.first() {
                    format!("-{short}")
                } else {
                    return None;
                };
                Some(prefix)
            }
            FlagValue::Bool(false) => None,
            FlagValue::Count(0) => None,
            FlagValue::Count(n) => {
                if let Some(short) = flag.short.first() {
                    Some(format!("-{}", short.to_string().repeat(*n as usize)))
                } else if let Some(long) = flag.long.first() {
                    Some(
                        std::iter::repeat_n(format!("--{long}"), *n as usize)
                            .collect::<Vec<_>>()
                            .join(" "),
                    )
                } else {
                    None
                }
            }
            FlagValue::String(s) if s.is_empty() => None,
            FlagValue::String(s) => {
                let prefix = if let Some(long) = flag.long.first() {
                    format!("--{long}")
                } else if let Some(short) = flag.short.first() {
                    format!("-{short}")
                } else {
                    return None;
                };
                if s.contains(' ') {
                    Some(format!("{prefix} \"{s}\""))
                } else {
                    Some(format!("{prefix} {s}"))
                }
            }
        }
    }

    /// Get the help text for the currently highlighted item.
    pub fn current_help(&self) -> Option<String> {
        match self.focus() {
            Focus::Commands => {
                // Get help from the selected tree node
                let visible =
                    flatten_visible_nodes(&self.command_tree_nodes, &self.command_tree_state);
                visible
                    .get(self.command_tree_state.selected_index)
                    .and_then(|node| node.data.help.clone())
            }
            Focus::Flags => {
                let flags = self.visible_flags();
                flags.get(self.flag_index()).and_then(|f| f.help.clone())
            }
            Focus::Args => self.arg_values.get(self.arg_index()).and_then(|_| {
                let cmd = self.current_command();
                cmd.args
                    .iter()
                    .filter(|a| !a.hide)
                    .nth(self.arg_index())
                    .and_then(|a| a.help.clone())
            }),
            Focus::Preview => Some("Press Enter to accept the command, Esc to go back".to_string()),
        }
    }
}

// --- Tree building functions ---

/// Build tree nodes from a usage spec.
pub fn build_command_tree(spec: &Spec) -> Vec<TreeNode<CmdData>> {
    // Build top-level commands directly (no root wrapper node)
    build_cmd_nodes(&spec.cmd, &[])
}

fn build_cmd_nodes(cmd: &SpecCommand, parent_path: &[String]) -> Vec<TreeNode<CmdData>> {
    cmd.subcommands
        .iter()
        .filter(|(_, c)| !c.hide)
        .map(|(name, c)| {
            let mut path = parent_path.to_vec();
            path.push(name.clone());
            let id = path.join(" ");
            TreeNode::new(
                &id,
                CmdData {
                    name: name.clone(),
                    help: c.help.clone(),
                    aliases: c.aliases.clone(),
                },
            )
            .with_children(build_cmd_nodes(c, &path))
        })
        .collect()
}

/// Collapse all nodes that have children, so the initial view
/// shows just the top-level commands.
fn collapse_parents_with_children(nodes: &[TreeNode<CmdData>], state: &mut TreeViewState) {
    for node in nodes {
        if node.has_children() {
            state.collapse(&node.id);
        }
        collapse_descendants(&node.children, state);
    }
}

fn collapse_descendants(nodes: &[TreeNode<CmdData>], state: &mut TreeViewState) {
    for node in nodes {
        if node.has_children() {
            state.collapse(&node.id);
        }
        collapse_descendants(&node.children, state);
    }
}

/// Compute match scores for all tree nodes recursively.
/// Returns a map of node ID → score (0 for non-matches).
fn compute_tree_scores(
    nodes: &[TreeNode<CmdData>],
    pattern: &str,
) -> std::collections::HashMap<String, u32> {
    let mut scores = std::collections::HashMap::new();

    for node in nodes {
        let mut temp_matcher = Matcher::new(Config::DEFAULT);

        // Score this node
        let name_score = fuzzy_match_score(&node.data.name, pattern, &mut temp_matcher);

        let alias_score = node
            .data
            .aliases
            .iter()
            .map(|a| fuzzy_match_score(a, pattern, &mut temp_matcher))
            .max()
            .unwrap_or(0);

        let help_score = node
            .data
            .help
            .as_ref()
            .map(|h| fuzzy_match_score(h, pattern, &mut temp_matcher))
            .unwrap_or(0);

        let node_score = name_score.max(alias_score).max(help_score);
        scores.insert(node.id.clone(), node_score);

        // Recursively score children
        let child_scores = compute_tree_scores(&node.children, pattern);
        scores.extend(child_scores);
    }

    scores
}

/// Count visible nodes in the tree (respecting collapsed state).
fn count_visible<T>(nodes: &[TreeNode<T>], state: &TreeViewState) -> usize {
    let mut total = 0;
    for node in nodes {
        total += 1;
        if node.has_children() && !state.is_collapsed(&node.id) {
            total += count_visible(&node.children, state);
        }
    }
    total
}

/// Flatten visible node IDs in order.
fn flatten_visible_ids<T>(nodes: &[TreeNode<T>], state: &TreeViewState) -> Vec<String> {
    let mut result = Vec::new();
    flatten_ids_recursive(nodes, state, &mut result);
    result
}

fn flatten_ids_recursive<T>(
    nodes: &[TreeNode<T>],
    state: &TreeViewState,
    result: &mut Vec<String>,
) {
    for node in nodes {
        result.push(node.id.clone());
        if node.has_children() && !state.is_collapsed(&node.id) {
            flatten_ids_recursive(&node.children, state, result);
        }
    }
}

/// Flatten visible tree nodes (references) in order.
fn flatten_visible_nodes<'a, T>(
    nodes: &'a [TreeNode<T>],
    state: &TreeViewState,
) -> Vec<&'a TreeNode<T>> {
    let mut result = Vec::new();
    flatten_nodes_recursive(nodes, state, &mut result);
    result
}

fn flatten_nodes_recursive<'a, T>(
    nodes: &'a [TreeNode<T>],
    state: &TreeViewState,
    result: &mut Vec<&'a TreeNode<T>>,
) {
    for node in nodes {
        result.push(node);
        if node.has_children() && !state.is_collapsed(&node.id) {
            flatten_nodes_recursive(&node.children, state, result);
        }
    }
}

/// Get the parent ID from a node ID.
fn parent_id(id: &str) -> Option<String> {
    if id.is_empty() {
        None // root has no parent
    } else if let Some(pos) = id.rfind(' ') {
        Some(id[..pos].to_string())
    } else {
        Some(String::new()) // parent is root
    }
}

/// Simple fuzzy matching: checks if all characters in the pattern appear in order in the text.
/// Fuzzy match using nucleo-matcher, returns score (0 if no match).
pub fn fuzzy_match_score(text: &str, pattern: &str, matcher: &mut Matcher) -> u32 {
    use nucleo_matcher::pattern::Normalization;
    use nucleo_matcher::Utf32Str;

    let atom = Atom::new(
        pattern,
        CaseMatching::Smart,
        Normalization::Smart,
        AtomKind::Fuzzy,
        false, // append_dollar
    );

    // Convert text to UTF-32 for matching
    let mut haystack_buf = Vec::new();
    let haystack = Utf32Str::new(text, &mut haystack_buf);

    atom.score(haystack, matcher)
        .map(|score| score as u32)
        .unwrap_or(0)
}

/// Simple boolean fuzzy match for backward compatibility (used in tests).
#[cfg(test)]
pub fn fuzzy_match(text: &str, pattern: &str) -> bool {
    let mut text_chars = text.chars();
    for pc in pattern.chars() {
        loop {
            match text_chars.next() {
                Some(tc) if tc == pc => break,
                Some(_) => continue,
                None => return false,
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_spec() -> Spec {
        let input = include_str!("../fixtures/sample.usage.kdl");
        input.parse::<Spec>().expect("Failed to parse sample spec")
    }

    #[test]
    fn test_app_creation() {
        let app = App::new(sample_spec());
        assert_eq!(app.spec.bin, "mycli");
        assert_eq!(app.spec.name, "My CLI");
        assert_eq!(app.command_path, Vec::<String>::new());
        assert_eq!(app.focus(), Focus::Commands);
    }

    #[test]
    fn test_tree_built_from_spec() {
        let app = App::new(sample_spec());
        // The tree should have top-level command nodes (no root wrapper)
        assert!(app.command_tree_nodes.len() > 1);
        // Check for some expected top-level commands
        let names: Vec<&str> = app
            .command_tree_nodes
            .iter()
            .map(|n| n.data.name.as_str())
            .collect();
        assert!(names.contains(&"init"));
        assert!(names.contains(&"config"));
        assert!(names.contains(&"run"));
    }

    #[test]
    fn test_tree_root_expanded_by_default() {
        let app = App::new(sample_spec());
        // Root should be expanded
        assert!(!app.command_tree_state.is_collapsed(""));
        // Non-root parents should be collapsed
        assert!(app.command_tree_state.is_collapsed("config"));
        assert!(app.command_tree_state.is_collapsed("plugin"));
    }

    #[test]
    fn test_visible_subcommands_at_root() {
        let app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let names: Vec<&str> = subs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"init"));
        assert!(names.contains(&"config"));
        assert!(names.contains(&"run"));
        assert!(names.contains(&"deploy"));
        assert!(names.contains(&"plugin"));
        assert!(names.contains(&"version"));
        assert!(names.contains(&"help"));
    }

    #[test]
    fn test_navigate_to_command() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        assert_eq!(app.command_path, vec!["config"]);

        let subs = app.visible_subcommands();
        let names: Vec<&str> = subs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"set"));
        assert!(names.contains(&"get"));
        assert!(names.contains(&"list"));
        assert!(names.contains(&"remove"));
    }

    #[test]
    fn test_navigate_to_deep_command() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config", "set"]);
        assert_eq!(app.command_path, vec!["config", "set"]);
    }

    #[test]
    fn test_navigate_into_subcommand() {
        let mut app = App::new(sample_spec());
        // Select "config" in the tree (index 0 = root, so find config)
        app.navigate_to_command(&["config"]);
        assert_eq!(app.command_path, vec!["config"]);

        // Now navigate into (expand + first child)
        app.navigate_into_selected();
        // config's first child should now be selected
        assert!(!app.command_path.is_empty());
        // We should be at one of config's subcommands
        assert!(
            app.command_path.len() == 2 && app.command_path[0] == "config",
            "Should be in config's subtree: {:?}",
            app.command_path
        );
    }

    #[test]
    fn test_navigate_up() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config", "set"]);
        assert_eq!(app.command_path, vec!["config", "set"]);

        app.navigate_up();
        assert_eq!(app.command_path, vec!["config"]);

        // At top-level command, navigate_up does nothing (no root node)
        app.navigate_up();
        assert_eq!(app.command_path, vec!["config"]);
    }

    #[test]
    fn test_build_command_basic() {
        let app = App::new(sample_spec());
        let cmd = app.build_command();
        assert_eq!(cmd, "mycli");
    }

    #[test]
    fn test_build_command_with_subcommand() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        let cmd = app.build_command();
        assert!(cmd.starts_with("mycli init"));
    }

    #[test]
    fn test_build_command_with_flags_and_args() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        // Set the "name" arg
        if let Some(arg) = app.arg_values.get_mut(0) {
            arg.value = "myproject".to_string();
        }

        // Toggle force flag
        let values = app.current_flag_values_mut();
        for (name, value) in values.iter_mut() {
            if name == "force" {
                *value = FlagValue::Bool(true);
            }
        }

        let cmd = app.build_command();
        assert!(cmd.contains("mycli"));
        assert!(cmd.contains("init"));
        assert!(cmd.contains("--force"));
        assert!(cmd.contains("myproject"));
    }

    #[test]
    fn test_build_command_with_count_flag() {
        let mut app = App::new(sample_spec());

        // Set verbose count to 3
        let key = app.command_path_key();
        if let Some(flags) = app.flag_values.get_mut(&key) {
            for (name, value) in flags.iter_mut() {
                if name == "verbose" {
                    *value = FlagValue::Count(3);
                }
            }
        }

        let cmd = app.build_command();
        assert!(cmd.contains("-vvv"));
    }

    #[test]
    fn test_fuzzy_match() {
        assert!(fuzzy_match("config", "cfg"));
        assert!(fuzzy_match("config", "con"));
        assert!(fuzzy_match("config", "config"));
        assert!(!fuzzy_match("config", "xyz"));
        assert!(fuzzy_match("deploy", "dpl"));
        assert!(!fuzzy_match("deploy", "dpx"));
        assert!(fuzzy_match("hello world", "hwd"));
    }

    #[test]
    fn test_command_path_navigation() {
        let mut app = App::new(sample_spec());
        assert!(app.command_path.is_empty());

        app.navigate_to_command(&["config"]);
        assert_eq!(app.command_path, vec!["config"]);

        app.navigate_to_command(&["config", "set"]);
        assert_eq!(app.command_path, vec!["config", "set"]);

        app.navigate_up();
        assert_eq!(app.command_path, vec!["config"]);
    }

    #[test]
    fn test_current_help() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        // Select a command that has help text
        app.navigate_to_command(&["init"]);

        // Should return help for the selected command
        let help = app.current_help();
        assert!(help.is_some());
    }

    #[test]
    fn test_visible_flags_includes_global() {
        let app = App::new(sample_spec());
        let flags = app.visible_flags();
        let names: Vec<&str> = flags.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"verbose"));
        assert!(names.contains(&"quiet"));
    }

    #[test]
    fn test_arg_values_initialized() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        assert!(!app.arg_values.is_empty());
        assert_eq!(app.arg_values[0].name, "name");
        assert!(app.arg_values[0].required);
    }

    #[test]
    fn test_deploy_has_choices() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // The <environment> arg should have choices
        assert!(!app.arg_values.is_empty());
        assert_eq!(app.arg_values[0].name, "environment");
        assert!(app.arg_values[0].choices.contains(&"dev".to_string()));
        assert!(app.arg_values[0].choices.contains(&"staging".to_string()));
        assert!(app.arg_values[0].choices.contains(&"prod".to_string()));
    }

    #[test]
    fn test_flag_with_default_value() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        let flag_values = app.current_flag_values();
        let jobs = flag_values.iter().find(|(n, _)| n == "jobs");
        assert!(jobs.is_some());
        if let Some((_, FlagValue::String(s))) = jobs {
            assert_eq!(s, "4");
        } else {
            panic!("Expected string flag value for jobs");
        }
    }

    #[test]
    fn test_key_handling_quit() {
        let mut app = App::new(sample_spec());
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );
        assert_eq!(app.handle_key(key), Action::Quit);
    }

    #[test]
    fn test_key_handling_navigation() {
        let mut app = App::new(sample_spec());

        // Move down in tree
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.command_index(), 1);

        // Move up in tree
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(app.command_index(), 0);
    }

    #[test]
    fn test_tab_cycles_focus() {
        let mut app = App::new(sample_spec());
        assert_eq!(app.focus(), Focus::Commands);

        let tab = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(tab);
        assert_eq!(app.focus(), Focus::Flags);

        let tab = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(tab);
        // Root has no args, so should skip to Preview
        assert_eq!(app.focus(), Focus::Preview);
    }

    #[test]
    fn test_filter_mode() {
        let mut app = App::new(sample_spec());

        // Set focus to Commands panel
        app.set_focus(Focus::Commands);

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        // Type "cfg" to filter top-level commands
        // Note: typing in filter mode resets selection to index 0
        for c in "cfg".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }
        assert_eq!(app.filter(), "cfg");

        // After filtering, selection is at index 0 which is "init"
        // visible_subcommands returns subcommands of current selection
        // Since filtering resets to first node, we're now at "init" which has no subcommands
        // This is the current behavior - filtering in tree mode resets selection

        // For now, just verify filtering is active and filter value is correct
        assert!(app.filtering);
        assert_eq!(app.filter(), "cfg");
    }

    #[test]
    fn test_filter_flags() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Focus on flags
        app.set_focus(Focus::Flags);

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        // Type "roll" to filter for --rollback
        for c in "roll".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }
        assert_eq!(app.filter(), "roll");

        // visible_flags returns all flags, but match scores show which ones match
        let flags = app.visible_flags();
        let scores = app.compute_flag_match_scores();

        // rollback should match (score > 0)
        let rollback_score = scores.get("rollback").copied().unwrap_or(0);
        assert!(rollback_score > 0, "rollback should match 'roll'");

        // tag and yes should not match (score = 0)
        let tag_score = scores.get("tag").copied().unwrap_or(0);
        let yes_score = scores.get("yes").copied().unwrap_or(0);
        assert_eq!(tag_score, 0, "tag should not match 'roll'");
        assert_eq!(yes_score, 0, "yes should not match 'roll'");

        // All flags should still be in visible_flags (subdued filtering)
        let names: Vec<&str> = flags.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"rollback"));
        assert!(names.contains(&"tag"));
        assert!(names.contains(&"yes"));
    }

    #[test]
    fn test_filter_tab_switches_focus_and_clears() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        // Type something
        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(key);
        assert_eq!(app.filter(), "x");

        // Tab should stop filtering and switch focus
        let tab = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(tab);
        assert!(!app.filtering);
        assert!(app.filter().is_empty());
        assert_eq!(app.focus(), Focus::Flags);
    }

    #[test]
    fn test_scroll_offset_ensure_visible() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Manually set selected index beyond a small viewport
        app.command_tree_state.selected_index = 5;
        app.command_tree_state.scroll = 0;
        app.ensure_visible(Focus::Commands, 3);
        // Index 5 should push scroll to at least 3 (5 - 3 + 1)
        assert_eq!(app.command_scroll(), 3);

        // Now move index back into view
        app.command_tree_state.selected_index = 2;
        app.ensure_visible(Focus::Commands, 3);
        // Index 2 < scroll 3, so scroll should snap to 2
        assert_eq!(app.command_scroll(), 2);

        // Already visible: no change
        app.command_tree_state.selected_index = 3;
        app.ensure_visible(Focus::Commands, 3);
        assert_eq!(app.command_scroll(), 2);
    }

    #[test]
    fn test_scroll_offset_on_move_up() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Set scroll and index so move_up triggers scroll adjustment
        app.command_tree_state.scroll = 2;
        app.command_tree_state.selected_index = 2;
        app.move_up();
        assert_eq!(app.command_index(), 1);
        // ListPickerState/TreeViewState.select_prev() doesn't auto-adjust scroll,
        // but our ensure_visible will handle it during render
    }

    #[test]
    fn test_navigate_resets_to_parent() {
        let mut app = App::new(sample_spec());

        // Navigate deep
        app.navigate_to_command(&["config", "set"]);
        assert_eq!(app.command_path, vec!["config", "set"]);

        // Navigate up
        app.navigate_up();
        assert_eq!(app.command_path, vec!["config"]);

        // At top-level command, navigate_up does nothing (no root node)
        app.navigate_up();
        assert_eq!(app.command_path, vec!["config"]);
    }

    #[test]
    fn test_mouse_click_focuses_panel() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Set up click regions (simulating what render would produce)
        app.click_regions.clear();
        app.click_regions
            .register(ratatui::layout::Rect::new(0, 1, 40, 18), Focus::Commands);
        app.click_regions
            .register(ratatui::layout::Rect::new(40, 1, 60, 18), Focus::Flags);
        app.click_regions
            .register(ratatui::layout::Rect::new(0, 21, 100, 3), Focus::Preview);

        // Click in the flag area
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 50,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert_eq!(app.focus(), Focus::Flags);

        // Click in the preview area
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 22,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert_eq!(app.focus(), Focus::Preview);

        // Click in the command area
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert_eq!(app.focus(), Focus::Commands);
    }

    #[test]
    fn test_mouse_click_selects_item() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Set up click region — border at y=1, first item at y=2
        app.click_regions.clear();
        app.click_regions
            .register(ratatui::layout::Rect::new(0, 1, 40, 18), Focus::Commands);

        // Click on 3rd item (row = border_top + 1 + 2 = 4)
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 4,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert_eq!(app.command_index(), 2);
    }

    #[test]
    fn test_mouse_scroll_moves_selection() {
        use crossterm::event::{MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Scroll down
        let mouse = MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 10,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert_eq!(app.command_index(), 1);

        // Scroll down again
        app.handle_mouse(mouse);
        assert_eq!(app.command_index(), 2);

        // Scroll up
        let mouse_up = MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 10,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse_up);
        assert_eq!(app.command_index(), 1);
    }

    #[test]
    fn test_focus_manager_integration() {
        let mut app = App::new(sample_spec());
        // Root has commands and flags, no args → [Commands, Flags, Preview]
        assert_eq!(app.focus(), Focus::Commands);

        app.focus_manager.next();
        assert_eq!(app.focus(), Focus::Flags);

        app.focus_manager.next();
        assert_eq!(app.focus(), Focus::Preview);

        app.focus_manager.next();
        assert_eq!(app.focus(), Focus::Commands); // wraps around

        app.focus_manager.prev();
        assert_eq!(app.focus(), Focus::Preview);
    }

    #[test]
    fn test_tree_view_state_integration() {
        let mut app = App::new(sample_spec());
        let total = app.total_visible_commands();
        // 7 top-level commands (no root, config and plugin collapsed)
        assert_eq!(total, 7);
        assert_eq!(app.command_index(), 0);

        let total = app.total_visible_commands();
        app.command_tree_state.select_next(total);
        assert_eq!(app.command_index(), 1);

        app.command_tree_state.select_prev();
        assert_eq!(app.command_index(), 0);

        // Can't go below 0
        app.command_tree_state.select_prev();
        assert_eq!(app.command_index(), 0);
    }

    #[test]
    fn test_input_state_editing() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        // Focus on args and start editing
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Start editing
        app.start_editing();
        assert!(app.editing);
        assert!(app.edit_input.text().is_empty()); // default is empty for name arg

        // Type "hello"
        app.edit_input.insert_char('h');
        app.edit_input.insert_char('e');
        app.edit_input.insert_char('l');
        app.edit_input.insert_char('l');
        app.edit_input.insert_char('o');
        app.sync_edit_to_value();

        assert_eq!(app.edit_input.text(), "hello");
        assert_eq!(app.arg_values[0].value, "hello");

        // Use cursor movement
        app.edit_input.move_home();
        assert_eq!(app.edit_input.cursor_pos, 0);
        app.edit_input.move_end();
        assert_eq!(app.edit_input.cursor_pos, 5);

        // Delete backward
        app.edit_input.delete_char_backward();
        app.sync_edit_to_value();
        assert_eq!(app.arg_values[0].value, "hell");

        app.finish_editing();
        assert!(!app.editing);
    }

    #[test]
    fn test_click_region_registry() {
        let mut app = App::new(sample_spec());
        app.click_regions.clear();
        app.click_regions
            .register(Rect::new(0, 0, 40, 20), Focus::Commands);
        app.click_regions
            .register(Rect::new(40, 0, 60, 20), Focus::Flags);
        app.click_regions
            .register(Rect::new(0, 20, 100, 3), Focus::Preview);

        assert_eq!(
            app.click_regions.handle_click(10, 5),
            Some(&Focus::Commands)
        );
        assert_eq!(app.click_regions.handle_click(50, 5), Some(&Focus::Flags));
        assert_eq!(
            app.click_regions.handle_click(50, 21),
            Some(&Focus::Preview)
        );
    }

    #[test]
    fn test_filter_input_state() {
        let mut app = App::new(sample_spec());

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);
        assert!(app.filter().is_empty());

        // Type using InputState
        let key_d = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(key_d);
        assert_eq!(app.filter(), "d");

        // Backspace
        let bs = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(bs);
        assert!(app.filter().is_empty());

        // Escape clears filter
        let key_x = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('x'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(key_x);
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(esc);
        assert!(!app.filtering);
        assert!(app.filter().is_empty());
    }

    // ── Tree-specific tests ─────────────────────────────────────────────

    #[test]
    fn test_tree_expand_collapse() {
        let mut app = App::new(sample_spec());

        // Initially config is collapsed
        assert!(app.command_tree_state.is_collapsed("config"));
        let initial_visible = app.total_visible_commands();

        // Expand config
        app.command_tree_state.expand("config");
        let expanded_visible = app.total_visible_commands();
        assert!(expanded_visible > initial_visible);

        // Collapse config
        app.command_tree_state.collapse("config");
        assert_eq!(app.total_visible_commands(), initial_visible);
    }

    #[test]
    fn test_tree_toggle_via_enter() {
        let mut app = App::new(sample_spec());
        // Select config (which has children)
        app.navigate_to_command(&["config"]);
        assert!(app.command_tree_state.is_collapsed("config"));

        // Press Enter to toggle
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.set_focus(Focus::Commands);
        app.handle_key(enter);
        assert!(!app.command_tree_state.is_collapsed("config"));

        // Press Enter again to collapse
        app.handle_key(enter);
        assert!(app.command_tree_state.is_collapsed("config"));
    }

    #[test]
    fn test_tree_right_expands() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        assert!(app.command_tree_state.is_collapsed("config"));

        // Press Right to expand
        app.set_focus(Focus::Commands);
        let right = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(right);
        assert!(!app.command_tree_state.is_collapsed("config"));
    }

    #[test]
    fn test_tree_left_collapses() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        // Expand config first
        app.command_tree_state.expand("config");
        assert!(!app.command_tree_state.is_collapsed("config"));

        // Press Left to collapse
        app.set_focus(Focus::Commands);
        let left = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Left,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(left);
        assert!(app.command_tree_state.is_collapsed("config"));
    }

    #[test]
    fn test_tree_left_on_leaf_moves_to_parent() {
        let mut app = App::new(sample_spec());
        // Select "config set" which is a leaf (no children)
        app.navigate_to_command(&["config", "set"]);
        assert_eq!(app.command_path, vec!["config", "set"]);

        // Press Left should move to parent (config)
        app.set_focus(Focus::Commands);
        let left = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Left,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(left);
        assert_eq!(
            app.command_path,
            vec!["config"],
            "Should have moved to parent of set"
        );
    }

    #[test]
    fn test_tree_selected_command_determines_flags() {
        let mut app = App::new(sample_spec());

        // At root, should show root's flags
        let root_flags = app.visible_flags();
        let root_flag_names: Vec<&str> = root_flags.iter().map(|f| f.name.as_str()).collect();
        assert!(root_flag_names.contains(&"verbose"));

        // Navigate to deploy, should show deploy's flags
        app.navigate_to_command(&["deploy"]);
        let deploy_flags = app.visible_flags();
        let deploy_flag_names: Vec<&str> = deploy_flags.iter().map(|f| f.name.as_str()).collect();
        assert!(deploy_flag_names.contains(&"tag"));
        assert!(deploy_flag_names.contains(&"rollback"));
    }

    #[test]
    fn test_parent_id() {
        assert_eq!(parent_id(""), None);
        assert_eq!(parent_id("init"), Some("".to_string()));
        assert_eq!(parent_id("config"), Some("".to_string()));
        assert_eq!(parent_id("config set"), Some("config".to_string()));
        assert_eq!(parent_id("plugin install"), Some("plugin".to_string()));
    }

    // ── Count flag / editing tests ──────────────────────────────────────

    #[test]
    fn test_count_flag_decrement_via_backspace() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());

        // verbose is a count flag at the root level
        app.set_focus(Focus::Flags);
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "verbose")
            .unwrap();
        app.set_flag_index(fidx);

        // Increment via space three times
        for _ in 0..3 {
            app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        }
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::Count(3),
            "Space should increment count to 3"
        );

        // Decrement via backspace
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::Count(2),
            "Backspace should decrement count to 2"
        );

        // Decrement all the way to zero
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::Count(0),
            "Count should be 0 after decrementing fully"
        );

        // Decrement again — should stay at 0 (floor)
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::Count(0),
            "Count should not go below 0"
        );
    }

    #[test]
    fn test_backspace_only_affects_count_flags() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());

        // quiet is a boolean flag — backspace should not change it
        app.set_focus(Focus::Flags);
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "quiet")
            .unwrap();
        app.set_flag_index(fidx);

        // Toggle it on first
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.current_flag_values()[fidx].1, FlagValue::Bool(true));

        // Backspace should not toggle it off (only affects count flags)
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::Bool(true),
            "Backspace should not affect boolean flags"
        );
    }

    #[test]
    fn test_editing_finished_on_mouse_click_different_arg() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        // Focus on args panel
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <task>

        // Start editing <task> and type some text
        app.start_editing();
        assert!(app.editing);
        app.edit_input.insert_char('h');
        app.edit_input.insert_char('i');
        app.sync_edit_to_value();
        assert_eq!(app.arg_values[0].value, "hi");

        // Now simulate clicking on the args area (which triggers finish_editing)
        // Set up a fake click region for Args
        let args_area = ratatui::layout::Rect::new(40, 5, 60, 10);
        app.click_regions.clear();
        app.click_regions.register(args_area, Focus::Args);

        // Click on the second arg item (row offset 1 from inner top)
        let mouse_event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 50,
            row: 7, // args_area.y + 1 (border) + 1 (second item)
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse_event);

        // Editing should have been finished before switching
        assert!(
            !app.editing,
            "Editing should be finished after clicking a different item"
        );

        // The <task> arg should still have the value we typed
        assert_eq!(
            app.arg_values[0].value, "hi",
            "First arg value should be preserved"
        );

        // The [args...] arg should NOT have the old edit_input text
        assert_eq!(
            app.arg_values[1].value, "",
            "Second arg value should be empty, not copied from first"
        );
    }

    #[test]
    fn test_editing_finished_on_mouse_click_different_panel() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        // Focus on args panel and start editing
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();
        app.edit_input.insert_char('t');
        app.edit_input.insert_char('e');
        app.edit_input.insert_char('s');
        app.edit_input.insert_char('t');
        app.sync_edit_to_value();
        assert_eq!(app.arg_values[0].value, "test");
        assert!(app.editing);

        // Set up click regions and click on the Flags panel
        app.click_regions.clear();
        app.click_regions
            .register(ratatui::layout::Rect::new(0, 0, 40, 15), Focus::Flags);
        app.click_regions
            .register(ratatui::layout::Rect::new(40, 0, 60, 15), Focus::Args);

        let mouse_event = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 10,
            row: 3,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse_event);

        // Editing should be finished
        assert!(
            !app.editing,
            "Editing should be finished when clicking another panel"
        );
        // The arg value should be preserved
        assert_eq!(app.arg_values[0].value, "test");
        // Focus should have moved to Flags
        assert_eq!(app.focus(), Focus::Flags);
    }

    #[test]
    fn test_edit_input_not_shared_between_args() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        // Focus on args, edit <task>
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();
        app.edit_input.insert_char('b');
        app.edit_input.insert_char('u');
        app.edit_input.insert_char('i');
        app.edit_input.insert_char('l');
        app.edit_input.insert_char('d');
        app.sync_edit_to_value();
        assert_eq!(app.arg_values[0].value, "build");

        // Finish editing via Enter key
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.editing);

        // Move to next arg and start editing
        app.set_arg_index(1);
        app.start_editing();

        // edit_input should now contain the value of the second arg (empty),
        // NOT the value from the first arg
        assert_eq!(
            app.edit_input.text(),
            "",
            "edit_input should be initialized from the new arg's value, not the old one"
        );

        // The first arg should still have its value
        assert_eq!(app.arg_values[0].value, "build");
        // The second arg should still be empty
        assert_eq!(app.arg_values[1].value, "");
    }

    #[test]
    fn test_handle_decrement_only_on_flags_focus() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());

        // Focus on commands — backspace should do nothing harmful
        app.set_focus(Focus::Commands);
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        // Should not panic or change state
        assert_eq!(app.focus(), Focus::Commands);
    }

    #[test]
    fn test_total_visible_commands_changes_with_expand() {
        let mut app = App::new(sample_spec());

        // Initially: 7 top-level commands (no root, config and plugin collapsed)
        let initial = app.total_visible_commands();
        assert_eq!(initial, 7);

        // Expand config (has 4 children)
        app.command_tree_state.expand("config");
        assert_eq!(app.total_visible_commands(), 11);

        // Expand plugin (has 4 children)
        app.command_tree_state.expand("plugin");
        assert_eq!(app.total_visible_commands(), 15);

        // Collapse config
        app.command_tree_state.collapse("config");
        assert_eq!(app.total_visible_commands(), 11);
    }

    #[test]
    fn test_flatten_visible_ids() {
        let app = App::new(sample_spec());
        let ids = flatten_visible_ids(&app.command_tree_nodes, &app.command_tree_state);

        // 7 top-level commands (no root node)
        assert_eq!(ids.len(), 7);
        assert_eq!(ids[0], "init");
        assert_eq!(ids[1], "config");
        // config is collapsed, so no config children
        assert_eq!(ids[2], "run");
        assert_eq!(ids[3], "deploy");
        assert_eq!(ids[4], "plugin");
        assert_eq!(ids[5], "version");
        assert_eq!(ids[6], "help");
    }

    #[test]
    fn test_flatten_visible_ids_with_expanded() {
        let mut app = App::new(sample_spec());
        app.command_tree_state.expand("config");

        let ids = flatten_visible_ids(&app.command_tree_nodes, &app.command_tree_state);
        assert_eq!(ids[0], "init");
        assert_eq!(ids[1], "config");
        assert_eq!(ids[2], "config set");
        assert_eq!(ids[3], "config get");
        assert_eq!(ids[4], "config list");
        assert_eq!(ids[5], "config remove");
        assert_eq!(ids[6], "run");
    }

    #[test]
    fn test_esc_at_root_quits() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        // Select root node (index 0)
        app.command_tree_state.selected_index = 0;
        app.sync_command_path_from_tree();

        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        assert_eq!(app.handle_key(esc), Action::Quit);
    }

    #[test]
    fn test_esc_on_expanded_node_collapses() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        app.command_tree_state.expand("config");
        app.set_focus(Focus::Commands);

        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(esc);
        assert_eq!(result, Action::None);
        assert!(app.command_tree_state.is_collapsed("config"));
    }

    #[test]
    fn test_esc_on_leaf_moves_to_parent() {
        let mut app = App::new(sample_spec());
        // Navigate to a nested command
        app.navigate_to_command(&["config", "set"]);
        app.set_focus(Focus::Commands);

        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(esc);
        assert_eq!(result, Action::None);
        assert_eq!(
            app.command_path,
            vec!["config"],
            "Should have moved to parent"
        );
    }
}
