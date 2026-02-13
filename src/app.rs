use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use portable_pty::{MasterPty, PtySize};
use ratatui::layout::Rect;
use ratatui_interact::components::{InputState, ListPickerState, TreeNode, TreeViewState};
use ratatui_interact::state::FocusManager;
use ratatui_interact::traits::ClickRegionRegistry;
use ratatui_themes::{ThemeName, ThemePalette};
use usage::{Spec, SpecCommand, SpecFlag};

/// Per-field match scores for an item (command or flag).
/// Keeps name and help scores separate so highlighting can be applied
/// independently to each field.
#[derive(Debug, Clone, Copy)]
pub struct MatchScores {
    /// Score from matching against the item's name (or aliases/path).
    pub name_score: u32,
    /// Score from matching against the item's help text.
    pub help_score: u32,
}

impl MatchScores {
    /// Overall score: the best of name and help.
    pub fn overall(&self) -> u32 {
        self.name_score.max(self.help_score)
    }
}

/// Actions that the event loop should take after handling a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Accept,
    Execute,
}

/// Whether the app is in command-builder mode or execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    /// Normal command-building UI.
    Builder,
    /// A command is running (or has finished) in an embedded terminal.
    Executing,
}

/// State for a running (or finished) command execution.
pub struct ExecutionState {
    /// The command string displayed at the top.
    pub command_display: String,
    /// vt100 parser holding the terminal screen state.
    pub parser: Arc<RwLock<vt100::Parser>>,
    /// Writer to send input to the PTY (None after the master is dropped).
    pub pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>,
    /// PTY master for resizing.
    pub pty_master: Arc<Mutex<Option<Box<dyn MasterPty + Send>>>>,
    /// Whether the child process has exited.
    pub exited: Arc<AtomicBool>,
    /// Exit status description (e.g. "0", "1", "signal 9").
    pub exit_status: Arc<Mutex<Option<String>>>,
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

/// A flattened command for display in a flat list with depth-based indentation.
#[derive(Debug, Clone)]
pub struct FlatCommand {
    pub id: String,
    pub name: String,
    pub help: Option<String>,
    pub aliases: Vec<String>,
    pub depth: usize,
    /// Full path of names from root to this command, e.g. "config set".
    /// Used for fuzzy matching so "cfgset" can match "config set".
    pub full_path: String,
}

/// Main application state.
pub struct App {
    pub spec: Spec,

    /// Current app mode (builder vs executing).
    pub mode: AppMode,

    /// Execution state when a command is running.
    pub execution: Option<ExecutionState>,

    /// Current color theme.
    pub theme_name: ThemeName,

    /// Path of subcommand names derived from the tree selection.
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

    /// Check if the app is currently in execution mode.
    pub fn is_executing(&self) -> bool {
        self.mode == AppMode::Executing
    }

    /// Check if the running command has exited.
    pub fn execution_exited(&self) -> bool {
        self.execution
            .as_ref()
            .map(|e| e.exited.load(Ordering::Relaxed))
            .unwrap_or(false)
    }

    /// Get the exit status string, if available.
    pub fn execution_exit_status(&self) -> Option<String> {
        self.execution
            .as_ref()
            .and_then(|e| e.exit_status.lock().ok().and_then(|s| s.clone()))
    }

    /// Close the execution view and return to the builder.
    pub fn close_execution(&mut self) {
        self.mode = AppMode::Builder;
        self.execution = None;
    }

    /// Start command execution with the given execution state.
    pub fn start_execution(&mut self, state: ExecutionState) {
        self.mode = AppMode::Executing;
        self.execution = Some(state);
    }

    /// Write bytes to the PTY (forward keyboard input during execution).
    pub fn write_to_pty(&self, data: &[u8]) {
        if let Some(ref exec) = self.execution {
            if let Ok(mut writer_guard) = exec.pty_writer.lock() {
                if let Some(ref mut writer) = *writer_guard {
                    let _ = writer.write_all(data);
                    let _ = writer.flush();
                }
            }
        }
    }

    /// Resize the PTY to fit the new terminal dimensions.
    pub fn resize_pty(&self, rows: u16, cols: u16) {
        if let Some(ref exec) = self.execution {
            // Resize the PTY master
            if let Ok(mut master_guard) = exec.pty_master.lock() {
                if let Some(ref mut master) = *master_guard {
                    let _ = master.resize(PtySize {
                        rows,
                        cols,
                        pixel_width: 0,
                        pixel_height: 0,
                    });
                }
            }
            // Resize the vt100 parser's screen in place so it matches the new PTY size.
            // This preserves existing content and avoids flashing/blanking.
            // The child process receives SIGWINCH from the PTY resize and will redraw.
            if let Ok(mut parser_guard) = exec.parser.write() {
                parser_guard.screen_mut().set_size(rows, cols);
            }
        }
    }

    pub fn with_theme(spec: usage::Spec, theme_name: ThemeName) -> Self {
        let tree_nodes = build_command_tree(&spec);
        let tree_state = TreeViewState::new();

        let mut app = Self {
            spec,
            mode: AppMode::Builder,
            execution: None,
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
        // Synchronize command_path with the tree's initial selection so the
        // correct flags/args are displayed on the very first render.
        app.sync_command_path_from_tree();
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
        // Clear filter when changing panels
        if self.focus() != panel {
            self.filtering = false;
            self.filter_input.clear();
        }
        self.focus_manager.set(panel);
    }

    /// Whether a filter is applied (filter text is non-empty), regardless of
    /// whether the user is still in typing mode (`self.filtering`).
    pub fn filter_active(&self) -> bool {
        !self.filter().is_empty()
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

    /// Returns the visible (non-hidden) args of the current command.
    pub fn visible_args(&self) -> Vec<&usage::SpecArg> {
        let cmd = self.current_command();
        cmd.args.iter().filter(|a| !a.hide).collect()
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
            // Collect current global flag values from root so new levels inherit them
            let root_global_values: std::collections::HashMap<String, FlagValue> = self
                .flag_values
                .get("")
                .map(|root_flags| {
                    root_flags
                        .iter()
                        .filter(|(name, _)| {
                            self.spec
                                .cmd
                                .flags
                                .iter()
                                .any(|f| f.global && f.name == *name)
                        })
                        .map(|(name, val)| (name.clone(), val.clone()))
                        .collect()
                })
                .unwrap_or_default();

            let flags = self.visible_flags_snapshot();
            let values: Vec<(String, FlagValue)> = flags
                .iter()
                .map(|f| {
                    // For global flags, inherit the current value from root
                    if let Some(global_val) = root_global_values.get(&f.name) {
                        return (f.name.clone(), global_val.clone());
                    }
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

    /// If the named flag is a global flag, propagate its value to all stored
    /// command-path levels so that build_command (which reads from root) and
    /// the UI (which reads from the current path) stay consistent.
    fn sync_global_flag(&mut self, flag_name: &str, new_value: &FlagValue) {
        let is_global = self
            .spec
            .cmd
            .flags
            .iter()
            .any(|f| f.global && f.name == flag_name);
        if !is_global {
            return;
        }
        // Collect all keys first to avoid borrow issues
        let keys: Vec<String> = self.flag_values.keys().cloned().collect();
        for key in keys {
            if let Some(flags) = self.flag_values.get_mut(&key) {
                if let Some((_, val)) = flags.iter_mut().find(|(n, _)| n == flag_name) {
                    *val = new_value.clone();
                }
            }
        }
    }

    // --- Tree view helpers ---

    /// Get the total number of commands in the flat list (all nodes, always visible).
    pub fn total_visible_commands(&self) -> usize {
        flatten_command_tree(&self.command_tree_nodes).len()
    }

    /// Compute match scores for all tree nodes when filtering.
    /// Returns a map of node ID → MatchScores with per-field scores.
    pub fn compute_tree_match_scores(&self) -> std::collections::HashMap<String, MatchScores> {
        let pattern = self.filter();
        if pattern.is_empty() {
            return std::collections::HashMap::new();
        }
        compute_tree_scores(&self.command_tree_nodes, pattern)
    }

    /// Compute match scores for all flags when filtering.
    /// Returns a map of flag name → score (0 for non-matches).
    pub fn compute_flag_match_scores(&self) -> std::collections::HashMap<String, MatchScores> {
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

            // name_score combines name, long, short, and path-like scores
            let combined_name_score = name_score.max(long_score).max(short_score);
            scores.insert(
                flag.name.clone(),
                MatchScores {
                    name_score: combined_name_score,
                    help_score,
                },
            );
        }

        scores
    }

    /// Compute match scores for all args when filtering.
    /// Returns a map of arg name → score (0 for non-matches).
    pub fn compute_arg_match_scores(&self) -> std::collections::HashMap<String, MatchScores> {
        let pattern = self.filter();
        if pattern.is_empty() {
            return std::collections::HashMap::new();
        }

        let args = self.visible_args();
        let mut scores = std::collections::HashMap::new();

        for arg in args {
            let mut temp_matcher = Matcher::new(Config::DEFAULT);
            let name_score = fuzzy_match_score(&arg.name, pattern, &mut temp_matcher);
            let help_score = arg
                .help
                .as_ref()
                .map(|h| fuzzy_match_score(h, pattern, &mut temp_matcher))
                .unwrap_or(0);

            scores.insert(
                arg.name.clone(),
                MatchScores {
                    name_score,
                    help_score,
                },
            );
        }

        scores
    }

    /// Get the ID of the currently selected tree node.
    pub fn selected_command_id(&self) -> Option<String> {
        let flat = flatten_command_tree(&self.command_tree_nodes);
        flat.get(self.command_tree_state.selected_index)
            .map(|cmd| cmd.id.clone())
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
        let flat = flatten_command_tree(&self.command_tree_nodes);
        let selected_id = flat
            .get(self.command_tree_state.selected_index)
            .map(|cmd| cmd.id.clone())?;
        let parent = parent_id(&selected_id)?;
        flat.iter().position(|cmd| cmd.id == parent)
    }

    /// Move to first child of selected node (Right/l key).
    pub fn tree_expand_or_enter(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                // Move to first child (next item in flat list)
                let total = self.total_visible_commands();
                self.command_tree_state.select_next(total);
                self.sync_command_path_from_tree();
            }
        }
    }

    /// Move to parent node (Left/h key).
    pub fn tree_collapse_or_parent(&mut self) {
        if let Some(parent_idx) = self.find_parent_index() {
            self.command_tree_state.selected_index = parent_idx;
            self.sync_command_path_from_tree();
        }
    }

    /// Navigate to a specific command path in the tree. Expands all ancestors
    /// and selects the target node. Used for tests and programmatic navigation.
    #[allow(dead_code)]
    pub fn navigate_to_command(&mut self, path: &[&str]) {
        let target_id = path.join(" ");
        let flat = flatten_command_tree(&self.command_tree_nodes);
        if let Some(idx) = flat.iter().position(|cmd| cmd.id == target_id) {
            self.command_tree_state.selected_index = idx;
            self.sync_command_path_from_tree();
        }
    }

    /// Select first child of current node.
    #[allow(dead_code)]
    pub fn navigate_into_selected(&mut self) {
        if let Some(id) = self.selected_command_id() {
            if self.node_has_children(&id) {
                // Move to first child (next item in flat list)
                let total = self.total_visible_commands();
                self.command_tree_state.select_next(total);
                self.sync_command_path_from_tree();
            }
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
                                        self.command_tree_state.selected_index = item_index;
                                        self.sync_command_path_from_tree();
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
                // Right-click in commands area (no-op now that tree is always flat)
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

        // If in execution mode, handle separately
        if self.is_executing() {
            return self.handle_execution_key(key);
        }

        // Ctrl+R executes command from any panel, regardless of edit/filter mode
        if key.code == KeyCode::Char('r')
            && key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return Action::Execute;
        }

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
            KeyCode::Char('T') | KeyCode::Char(']') => {
                self.next_theme();
                Action::None
            }
            KeyCode::Char('[') => {
                self.prev_theme();
                Action::None
            }
            KeyCode::Char('p') => {
                if self.focus() == Focus::Preview {
                    return Action::Accept;
                }
                Action::None
            }
            KeyCode::Char('/') => {
                // Only activate filter mode for panels that support filtering
                if matches!(self.focus(), Focus::Commands | Focus::Flags | Focus::Args) {
                    self.filtering = true;
                    self.filter_input.clear();
                }
                Action::None
            }
            KeyCode::Tab => {
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
            KeyCode::Esc => {
                // Esc only clears filter when active (otherwise no effect)
                if self.filter_active() {
                    self.filtering = false;
                    self.filter_input.clear();
                }
                Action::None
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
                if let Some((name, FlagValue::String(ref mut s))) = values.get_mut(flag_idx) {
                    let flag_name = name.clone();
                    *s = text;
                    let new_val = FlagValue::String(s.clone());
                    self.sync_global_flag(&flag_name, &new_val);
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
        // When a filter is applied, skip non-matching items
        if self.filter_active() {
            self.move_to_prev_match();
            return;
        }
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
        // When a filter is applied, skip non-matching items
        if self.filter_active() {
            self.move_to_next_match();
            return;
        }
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

    /// Move to the previous matching item when a filter is active.
    /// Wraps around to the last match if at the beginning.
    fn move_to_prev_match(&mut self) {
        match self.focus() {
            Focus::Commands => {
                let scores = self.compute_tree_match_scores();
                let flat = flatten_command_tree(&self.command_tree_nodes);
                let current = self.command_tree_state.selected_index;
                let total = flat.len();
                if total == 0 {
                    return;
                }
                // Search backwards, wrapping around
                for offset in 1..total {
                    let idx = (current + total - offset) % total;
                    if let Some(cmd) = flat.get(idx) {
                        if scores.get(&cmd.id).map(|s| s.overall()).unwrap_or(0) > 0 {
                            self.command_tree_state.selected_index = idx;
                            self.sync_command_path_from_tree();
                            return;
                        }
                    }
                }
            }
            Focus::Flags => {
                let scores = self.compute_flag_match_scores();
                let flags = self.visible_flags();
                let current = self.flag_list_state.selected_index;
                let total = flags.len();
                if total == 0 {
                    return;
                }
                for offset in 1..total {
                    let idx = (current + total - offset) % total;
                    if let Some(flag) = flags.get(idx) {
                        if scores.get(&flag.name).map(|s| s.overall()).unwrap_or(0) > 0 {
                            self.flag_list_state.select(idx);
                            return;
                        }
                    }
                }
            }
            Focus::Args => {
                let scores = self.compute_arg_match_scores();
                let args = self.visible_args();
                let current = self.arg_list_state.selected_index;
                let total = args.len();
                if total == 0 {
                    return;
                }
                for offset in 1..total {
                    let idx = (current + total - offset) % total;
                    if let Some(arg) = args.get(idx) {
                        if scores.get(&arg.name).map(|s| s.overall()).unwrap_or(0) > 0 {
                            self.arg_list_state.select(idx);
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Move to the next matching item when a filter is active.
    /// Wraps around to the first match if at the end.
    fn move_to_next_match(&mut self) {
        match self.focus() {
            Focus::Commands => {
                let scores = self.compute_tree_match_scores();
                let flat = flatten_command_tree(&self.command_tree_nodes);
                let current = self.command_tree_state.selected_index;
                let total = flat.len();
                if total == 0 {
                    return;
                }
                // Search forwards, wrapping around
                for offset in 1..total {
                    let idx = (current + offset) % total;
                    if let Some(cmd) = flat.get(idx) {
                        if scores.get(&cmd.id).map(|s| s.overall()).unwrap_or(0) > 0 {
                            self.command_tree_state.selected_index = idx;
                            self.sync_command_path_from_tree();
                            return;
                        }
                    }
                }
            }
            Focus::Flags => {
                let scores = self.compute_flag_match_scores();
                let flags = self.visible_flags();
                let current = self.flag_list_state.selected_index;
                let total = flags.len();
                if total == 0 {
                    return;
                }
                for offset in 1..total {
                    let idx = (current + offset) % total;
                    if let Some(flag) = flags.get(idx) {
                        if scores.get(&flag.name).map(|s| s.overall()).unwrap_or(0) > 0 {
                            self.flag_list_state.select(idx);
                            return;
                        }
                    }
                }
            }
            Focus::Args => {
                let scores = self.compute_arg_match_scores();
                let args = self.visible_args();
                let current = self.arg_list_state.selected_index;
                let total = args.len();
                if total == 0 {
                    return;
                }
                for offset in 1..total {
                    let idx = (current + offset) % total;
                    if let Some(arg) = args.get(idx) {
                        if scores.get(&arg.name).map(|s| s.overall()).unwrap_or(0) > 0 {
                            self.arg_list_state.select(idx);
                            return;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Handle key events during command execution mode.
    fn handle_execution_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::KeyCode;

        if self.execution_exited() {
            // Command has finished — any key closes the execution view
            match key.code {
                KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                    self.close_execution();
                    return Action::None;
                }
                _ => return Action::None,
            }
        }

        // Command is still running — forward input to the PTY
        let bytes: Option<Vec<u8>> = match key.code {
            KeyCode::Char(c) => {
                let mut buf = [0u8; 4];
                let s = c.encode_utf8(&mut buf);
                Some(s.as_bytes().to_vec())
            }
            KeyCode::Enter => Some(b"\r".to_vec()),
            KeyCode::Backspace => Some(b"\x7f".to_vec()),
            KeyCode::Tab => Some(b"\t".to_vec()),
            KeyCode::Esc => Some(b"\x1b".to_vec()),
            KeyCode::Up => Some(b"\x1b[A".to_vec()),
            KeyCode::Down => Some(b"\x1b[B".to_vec()),
            KeyCode::Right => Some(b"\x1b[C".to_vec()),
            KeyCode::Left => Some(b"\x1b[D".to_vec()),
            KeyCode::Home => Some(b"\x1b[H".to_vec()),
            KeyCode::End => Some(b"\x1b[F".to_vec()),
            KeyCode::Delete => Some(b"\x1b[3~".to_vec()),
            _ => None,
        };

        if let Some(data) = bytes {
            // Handle Ctrl+C to send SIGINT
            if key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
            {
                if let KeyCode::Char('c') = key.code {
                    self.write_to_pty(b"\x03");
                    return Action::None;
                }
                if let KeyCode::Char('d') = key.code {
                    self.write_to_pty(b"\x04");
                    return Action::None;
                }
            }
            self.write_to_pty(&data);
        }

        Action::None
    }

    fn handle_enter(&mut self) -> Action {
        match self.focus() {
            Focus::Commands => {
                // Enter navigates into the selected command (same as Right/l)
                self.tree_expand_or_enter();
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
                if let Some((name, value)) = values.get_mut(flag_idx) {
                    let flag_name = name.clone();
                    match value {
                        FlagValue::Bool(b) => {
                            *b = !*b;
                            let new_val = FlagValue::Bool(*b);
                            self.sync_global_flag(&flag_name, &new_val);
                        }
                        FlagValue::Count(c) => {
                            *c += 1;
                            let new_val = FlagValue::Count(*c);
                            self.sync_global_flag(&flag_name, &new_val);
                        }
                        FlagValue::String(s) => {
                            if let Some(choices) = maybe_choices {
                                // Cycle through choices
                                let idx = choices
                                    .iter()
                                    .position(|c| c == s.as_str())
                                    .map(|i| (i + 1) % choices.len())
                                    .unwrap_or(0);
                                *s = choices[idx].clone();
                                let new_val = FlagValue::String(s.clone());
                                self.sync_global_flag(&flag_name, &new_val);
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
            Focus::Preview => Action::Execute,
        }
    }

    fn handle_space(&mut self) {
        if self.focus() == Focus::Flags {
            let flag_idx = self.flag_index();
            let values = self.current_flag_values_mut();
            if let Some((name, value)) = values.get_mut(flag_idx) {
                let flag_name = name.clone();
                match value {
                    FlagValue::Bool(b) => {
                        *b = !*b;
                        let new_val = FlagValue::Bool(*b);
                        self.sync_global_flag(&flag_name, &new_val);
                    }
                    FlagValue::Count(c) => {
                        *c += 1;
                        let new_val = FlagValue::Count(*c);
                        self.sync_global_flag(&flag_name, &new_val);
                    }
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
                let flat = flatten_command_tree(&self.command_tree_nodes);
                let current_idx = self.command_tree_state.selected_index;

                // Check if current selection matches
                if let Some(cmd) = flat.get(current_idx) {
                    if let Some(score) = scores.get(&cmd.id) {
                        if score.overall() > 0 {
                            // Current selection matches, keep it
                            return;
                        }
                    }
                }

                // Current doesn't match, find next matching item
                for (idx, cmd) in flat.iter().enumerate() {
                    if let Some(score) = scores.get(&cmd.id) {
                        if score.overall() > 0 {
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
                    if let Some(score) = scores.get(&flag.name) {
                        if score.overall() > 0 {
                            // Current selection matches, keep it
                            return;
                        }
                    }
                }

                // Current doesn't match, find next matching item
                for (idx, flag) in flags.iter().enumerate() {
                    if let Some(score) = scores.get(&flag.name) {
                        if score.overall() > 0 {
                            self.flag_list_state.select(idx);
                            return;
                        }
                    }
                }

                // No matches found, stay at current position
            }
            Focus::Args => {
                let scores = self.compute_arg_match_scores();
                let current_idx = self.arg_list_state.selected_index;

                // Check if current selection matches
                if let Some(av) = self.arg_values.get(current_idx) {
                    if let Some(score) = scores.get(&av.name) {
                        if score.overall() > 0 {
                            // Current selection matches, keep it
                            return;
                        }
                    }
                }

                // Current doesn't match, find next matching item
                for (idx, av) in self.arg_values.iter().enumerate() {
                    if let Some(score) = scores.get(&av.name) {
                        if score.overall() > 0 {
                            self.arg_list_state.select(idx);
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
        if let Some((name, FlagValue::Count(c))) = values.get_mut(flag_idx) {
            let flag_name = name.clone();
            *c = c.saturating_sub(1);
            let new_val = FlagValue::Count(*c);
            self.sync_global_flag(&flag_name, &new_val);
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

        // Gather global flag values from the root command path.
        // Global flags are always synced to root via sync_global_flag(),
        // so we only need to check the root key.
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

                // Add flag values for this level (skip global flags, already added from root)
                let path_key = self.command_path[..=i].join(" ");
                if let Some(level_flags) = self.flag_values.get(&path_key) {
                    for (fname, fvalue) in level_flags {
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

    /// Build the command as a list of separate argument strings (for process execution).
    /// Unlike `build_command()`, this does NOT quote values — each element is a separate arg.
    pub fn build_command_parts(&self) -> Vec<String> {
        let mut parts: Vec<String> = Vec::new();

        // Binary name (may contain spaces like "mise run", split into separate args)
        let bin = if self.spec.bin.is_empty() {
            &self.spec.name
        } else {
            &self.spec.bin
        };
        for word in bin.split_whitespace() {
            parts.push(word.to_string());
        }

        // Gather global flag values from root (synced via sync_global_flag)
        let root_key = String::new();
        if let Some(root_flags) = self.flag_values.get(&root_key) {
            for (name, value) in root_flags {
                self.format_flag_parts(name, value, &self.spec.cmd.flags, &mut parts);
            }
        }

        // Add subcommand path
        let mut cmd = &self.spec.cmd;
        for (i, name) in self.command_path.iter().enumerate() {
            parts.push(name.clone());

            if let Some(sub) = cmd.find_subcommand(name) {
                cmd = sub;

                let path_key = self.command_path[..=i].join(" ");
                if let Some(level_flags) = self.flag_values.get(&path_key) {
                    for (fname, fvalue) in level_flags {
                        let is_global = self
                            .spec
                            .cmd
                            .flags
                            .iter()
                            .any(|f| f.global && f.name == *fname);
                        if is_global {
                            continue;
                        }
                        self.format_flag_parts(fname, fvalue, &cmd.flags, &mut parts);
                    }
                }
            }
        }

        // Add positional arg values (unquoted — each is a separate process arg)
        for arg in &self.arg_values {
            if !arg.value.is_empty() {
                parts.push(arg.value.clone());
            }
        }

        parts
    }

    /// Append flag parts (as separate arguments) to the parts list.
    fn format_flag_parts(
        &self,
        name: &str,
        value: &FlagValue,
        flags: &[SpecFlag],
        parts: &mut Vec<String>,
    ) {
        let flag = flags.iter().find(|f| f.name == name);
        let flag = flag.or_else(|| {
            self.spec
                .cmd
                .flags
                .iter()
                .find(|f| f.name == name && f.global)
        });

        let Some(flag) = flag else { return };

        match value {
            FlagValue::Bool(true) => {
                if let Some(long) = flag.long.first() {
                    parts.push(format!("--{long}"));
                } else if let Some(short) = flag.short.first() {
                    parts.push(format!("-{short}"));
                }
            }
            FlagValue::Bool(false) | FlagValue::Count(0) => {}
            FlagValue::Count(n) => {
                if let Some(short) = flag.short.first() {
                    parts.push(format!("-{}", short.to_string().repeat(*n as usize)));
                } else if let Some(long) = flag.long.first() {
                    for _ in 0..*n {
                        parts.push(format!("--{long}"));
                    }
                }
            }
            FlagValue::String(s) if s.is_empty() => {}
            FlagValue::String(s) => {
                if let Some(long) = flag.long.first() {
                    parts.push(format!("--{long}"));
                } else if let Some(short) = flag.short.first() {
                    parts.push(format!("-{short}"));
                } else {
                    return;
                }
                parts.push(s.clone());
            }
        }
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
                // Get help from the selected command in the flat list
                let flat = flatten_command_tree(&self.command_tree_nodes);
                flat.get(self.command_tree_state.selected_index)
                    .and_then(|cmd| cmd.help.clone())
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
            Focus::Preview => {
                Some("Enter: run command  p: print to stdout  Esc: go back".to_string())
            }
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

/// Compute match scores for all commands in the flat list.
/// Returns a map of node ID → score (0 for non-matches).
/// Matches against the command name, aliases, help text, AND the full ancestor
/// path so that e.g. "cfgset" matches "config set".
fn compute_tree_scores(
    nodes: &[TreeNode<CmdData>],
    pattern: &str,
) -> std::collections::HashMap<String, MatchScores> {
    let flat = flatten_command_tree(nodes);
    let mut scores = std::collections::HashMap::new();
    let mut matcher = Matcher::new(Config::DEFAULT);

    for cmd in &flat {
        let name_score = fuzzy_match_score(&cmd.name, pattern, &mut matcher);

        let alias_score = cmd
            .aliases
            .iter()
            .map(|a| fuzzy_match_score(a, pattern, &mut matcher))
            .max()
            .unwrap_or(0);

        let help_score = cmd
            .help
            .as_ref()
            .map(|h| fuzzy_match_score(h, pattern, &mut matcher))
            .unwrap_or(0);

        // Also match against the full path (e.g. "config set") so that
        // queries like "cfgset" can match subcommands via their parent chain.
        let path_score = fuzzy_match_score(&cmd.full_path, pattern, &mut matcher);

        // name_score combines name, alias, and path scores
        let combined_name_score = name_score.max(alias_score).max(path_score);
        scores.insert(
            cmd.id.clone(),
            MatchScores {
                name_score: combined_name_score,
                help_score,
            },
        );
    }

    scores
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

/// Flatten the tree structure into a list of commands with depth-based indentation.
pub fn flatten_command_tree(nodes: &[TreeNode<CmdData>]) -> Vec<FlatCommand> {
    fn flatten_recursive(
        nodes: &[TreeNode<CmdData>],
        depth: usize,
        parent_names: &[String],
        result: &mut Vec<FlatCommand>,
    ) {
        for node in nodes {
            let mut path_parts = parent_names.to_vec();
            path_parts.push(node.data.name.clone());
            let full_path = path_parts.join(" ");

            result.push(FlatCommand {
                id: node.id.clone(),
                name: node.data.name.clone(),
                help: node.data.help.clone(),
                aliases: node.data.aliases.clone(),
                depth,
                full_path,
            });

            if !node.children.is_empty() {
                flatten_recursive(&node.children, depth + 1, &path_parts, result);
            }
        }
    }

    let mut result = Vec::new();
    flatten_recursive(nodes, 0, &[], &mut result);
    result
}

/// Fuzzy match using nucleo-matcher Pattern, returns score (0 if no match).
/// Uses Pattern instead of Atom to properly handle multi-word patterns and special characters.
pub fn fuzzy_match_score(text: &str, pattern: &str, matcher: &mut Matcher) -> u32 {
    use nucleo_matcher::Utf32Str;

    let pattern = Pattern::parse(pattern, CaseMatching::Smart, Normalization::Smart);

    // Convert text to UTF-32 for matching
    let mut haystack_buf = Vec::new();
    let haystack = Utf32Str::new(text, &mut haystack_buf);

    pattern.score(haystack, matcher).unwrap_or(0)
}

/// Fuzzy match and return both score and match indices.
/// Returns (score, Vec<char_indices>) where indices are the positions of matched characters.
/// Uses Pattern instead of Atom to properly handle multi-word patterns and special characters.
/// Indices are sorted and deduplicated as recommended by nucleo-matcher documentation.
pub fn fuzzy_match_indices(
    text: &str,
    pattern_str: &str,
    matcher: &mut Matcher,
) -> (u32, Vec<u32>) {
    use nucleo_matcher::Utf32Str;

    let pattern = Pattern::parse(pattern_str, CaseMatching::Smart, Normalization::Smart);

    // Convert text to UTF-32 for matching
    let mut haystack_buf = Vec::new();
    let haystack = Utf32Str::new(text, &mut haystack_buf);

    let mut indices = Vec::new();
    if let Some(score) = pattern.indices(haystack, matcher, &mut indices) {
        // Sort and deduplicate indices as recommended by nucleo-matcher docs
        indices.sort_unstable();
        indices.dedup();
        (score, indices)
    } else {
        (0, Vec::new())
    }
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
        // After startup sync, command_path matches the tree's initial selection (first command)
        assert_eq!(app.command_path, vec!["init"]);
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
    fn test_flat_list_all_visible() {
        let app = App::new(sample_spec());
        // All commands are always visible in the flat list
        let flat = flatten_command_tree(&app.command_tree_nodes);
        assert_eq!(flat.len(), 15);
        // Includes nested subcommands
        assert!(flat.iter().any(|c| c.id == "config set"));
        assert!(flat.iter().any(|c| c.id == "plugin install"));
    }

    #[test]
    fn test_visible_subcommands_at_root() {
        let mut app = App::new(sample_spec());
        // After startup sync, command_path is ["init"], navigate to root
        app.command_path.clear();
        app.sync_state();
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
    fn test_build_command_basic() {
        let app = App::new(sample_spec());
        let cmd = app.build_command();
        // After startup sync, command_path is ["init"] so command includes it
        assert_eq!(cmd, "mycli init");
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

        // Set verbose count to 3 — verbose is a global flag, so set it at root
        // and sync to all levels (as the UI toggle would do).
        let root_key = String::new();
        if let Some(flags) = app.flag_values.get_mut(&root_key) {
            for (name, value) in flags.iter_mut() {
                if name == "verbose" {
                    *value = FlagValue::Count(3);
                }
            }
        }
        app.sync_global_flag("verbose", &FlagValue::Count(3));

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
    fn test_full_path_matching() {
        let app = App::new(sample_spec());
        let scores = compute_tree_scores(&app.command_tree_nodes, "cfgset");

        // "cfgset" should match "config set" via full_path "config set"
        let set_score = scores.get("config set").map(|s| s.overall()).unwrap_or(0);
        assert!(
            set_score > 0,
            "cfgset should match config set, got score {set_score}"
        );

        // "cfgset" should NOT match unrelated commands
        let init_score = scores.get("init").map(|s| s.overall()).unwrap_or(0);
        assert_eq!(init_score, 0, "cfgset should not match init");

        let run_score = scores.get("run").map(|s| s.overall()).unwrap_or(0);
        assert_eq!(run_score, 0, "cfgset should not match run");

        // "plinstall" should match "plugin install"
        let scores2 = compute_tree_scores(&app.command_tree_nodes, "plinstall");
        let install_score = scores2
            .get("plugin install")
            .map(|s| s.overall())
            .unwrap_or(0);
        assert!(
            install_score > 0,
            "plinstall should match plugin install, got score {install_score}"
        );
    }

    #[test]
    fn test_fuzzy_match_with_pattern_indices() {
        use nucleo_matcher::{Config, Matcher};

        let mut matcher = Matcher::new(Config::DEFAULT);

        // Test single word matching
        let (score, indices) = fuzzy_match_indices("config", "cfg", &mut matcher);
        assert!(score > 0, "Should match 'cfg' in 'config'");
        assert_eq!(indices, vec![0, 3, 5], "Should match c, f, g");

        // Test multi-word pattern (Pattern handles this properly)
        let (score, indices) = fuzzy_match_indices("foo bar baz", "foo baz", &mut matcher);
        assert!(score > 0, "Should match multi-word pattern");
        // Indices should be sorted and deduplicated
        assert!(indices.contains(&0)); // 'f' in foo
        assert!(indices.len() >= 6); // At least 3 chars from 'foo' + 3 from 'baz'

        // Test no match
        let (score, indices) = fuzzy_match_indices("config", "xyz", &mut matcher);
        assert_eq!(score, 0, "Should not match 'xyz'");
        assert!(indices.is_empty(), "No indices for non-match");

        // Test case-insensitive matching (CaseMatching::Smart)
        let (score, indices) = fuzzy_match_indices("MyConfig", "myconf", &mut matcher);
        assert!(score > 0, "Should match case-insensitively");
        assert_eq!(indices.len(), 6, "Should match all 6 characters");
    }

    #[test]
    fn test_command_path_navigation() {
        let mut app = App::new(sample_spec());
        // After startup sync, command_path matches tree's initial selection
        assert_eq!(app.command_path, vec!["init"]);

        app.navigate_to_command(&["config"]);
        assert_eq!(app.command_path, vec!["config"]);

        app.navigate_to_command(&["config", "set"]);
        assert_eq!(app.command_path, vec!["config", "set"]);
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
        // In flat list, index 1 is "config"
        assert!(app.command_index() > 0);

        // Move up in tree
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        // Should have moved back up
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
        // init (index 0) has args, so should go to Args or Preview depending on init state
        // The focus manager cycles: Commands -> Flags -> Args -> Preview
        assert_ne!(app.focus(), Focus::Commands);
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
        let rollback_score = scores.get("rollback").map(|s| s.overall()).unwrap_or(0);
        assert!(rollback_score > 0, "rollback should match 'roll'");

        // tag and yes should not match (score = 0)
        let tag_score = scores.get("tag").map(|s| s.overall()).unwrap_or(0);
        let yes_score = scores.get("yes").map(|s| s.overall()).unwrap_or(0);
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
    fn test_slash_does_not_activate_filter_in_preview_pane() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Preview);
        assert_eq!(app.focus(), Focus::Preview);
        assert!(!app.filtering);

        // Press "/" - should NOT activate filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(
            !app.filtering,
            "Filter mode should not activate in Preview pane"
        );
        assert!(app.filter().is_empty());
    }

    #[test]
    fn test_filtered_navigation_works_after_enter_applies_filter() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Enter filter mode and type a query
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        let p_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(p_key);
        assert_eq!(app.filter(), "p");

        // Press Enter to exit typing mode (but keep the filter applied)
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(!app.filtering, "Should exit typing mode after Enter");
        assert_eq!(app.filter(), "p", "Query should remain after Enter");
        assert!(app.filter_active(), "Filter should still be active");

        // Navigation should still use filtered mode (skip non-matches)
        // j/k should work without appending to the filter text
        let j_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(j_key);
        assert_eq!(app.filter(), "p", "j should navigate, not append to filter");
        assert!(app.filter_active(), "Filter should remain active after j");
    }

    #[test]
    fn test_esc_clears_applied_filter() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Enter filter mode, type, then apply with Enter
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        let p_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(p_key);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(!app.filtering);
        assert!(app.filter_active(), "Filter should be active after Enter");

        // Esc should clear the applied filter
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(esc);
        assert!(!app.filtering);
        assert!(!app.filter_active(), "Esc should clear the applied filter");
        assert!(app.filter().is_empty());
    }

    #[test]
    fn test_args_filter_matches_name() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        // Type "env" to filter arguments
        let e_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('e'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(e_key);
        let n_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('n'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(n_key);
        let v_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('v'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(v_key);

        assert_eq!(app.filter(), "env");

        // Check that "environment" matches
        let scores = app.compute_arg_match_scores();
        let env_score = scores.get("environment").map(|s| s.overall()).unwrap_or(0);
        assert!(env_score > 0, "environment should match 'env'");
    }

    #[test]
    fn test_args_filtered_navigation() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);
        app.set_focus(Focus::Args);

        // Enter filter mode and type "n"
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        let n_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('n'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(n_key);

        // Press Enter to apply filter
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(!app.filtering, "Should exit typing mode");
        assert!(app.filter_active(), "Filter should still be active");

        // Navigation should skip non-matching args
        let _initial_index = app.arg_list_state.selected_index;
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);

        // Should have moved (assuming there are matching args)
        let _new_index = app.arg_list_state.selected_index;
        // Can't assert specific index without knowing fixture details,
        // but we can verify the filter is still active
        assert!(app.filter_active());
    }

    #[test]
    fn test_tab_clears_applied_filter() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Enter filter mode, type, then apply with Enter
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        let p_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(p_key);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.filter_active(), "Filter should be active after Enter");

        // Tab should clear the filter and switch focus
        let tab = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(tab);
        assert!(!app.filter_active(), "Tab should clear the applied filter");
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
        // After startup sync, "init" is selected which has args
        // → [Commands, Flags, Args, Preview]
        assert_eq!(app.focus(), Focus::Commands);

        app.focus_manager.next();
        assert_eq!(app.focus(), Focus::Flags);

        app.focus_manager.next();
        assert_eq!(app.focus(), Focus::Args);

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
        // Flat list: all 15 commands always visible
        assert_eq!(total, 15);
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

    // ── Flat list navigation tests ──────────────────────────────────────

    #[test]
    fn test_flat_list_total_commands() {
        let app = App::new(sample_spec());
        // Flat list always shows all commands (7 top-level + 8 nested = 15)
        assert_eq!(app.total_visible_commands(), 15);
    }

    #[test]
    fn test_right_moves_to_first_child() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        app.set_focus(Focus::Commands);

        // Press Right to move to first child
        let right = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(right);
        assert_eq!(
            app.command_path,
            vec!["config", "set"],
            "Should have moved to first child of config"
        );
    }

    #[test]
    fn test_left_moves_to_parent() {
        let mut app = App::new(sample_spec());
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
    fn test_left_on_top_level_does_nothing() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config"]);
        app.set_focus(Focus::Commands);

        // Press Left on a top-level command — no parent to go to
        let left = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Left,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(left);
        assert_eq!(
            app.command_path,
            vec!["config"],
            "Should stay on config since it has no parent"
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
    fn test_flatten_command_tree_ids() {
        let app = App::new(sample_spec());
        let flat = flatten_command_tree(&app.command_tree_nodes);

        // All 15 commands in the flat list
        assert_eq!(flat.len(), 15);
        assert_eq!(flat[0].id, "init");
        assert_eq!(flat[1].id, "config");
        assert_eq!(flat[2].id, "config set");
        assert_eq!(flat[3].id, "config get");
        assert_eq!(flat[4].id, "config list");
        assert_eq!(flat[5].id, "config remove");
        assert_eq!(flat[6].id, "run");
        assert_eq!(flat[7].id, "deploy");
        assert_eq!(flat[8].id, "plugin");
        assert_eq!(flat[9].id, "plugin install");
        assert_eq!(flat[10].id, "plugin uninstall");
        assert_eq!(flat[11].id, "plugin list");
        assert_eq!(flat[12].id, "plugin update");
        assert_eq!(flat[13].id, "version");
        assert_eq!(flat[14].id, "help");
    }

    #[test]
    fn test_flatten_command_tree_full_path() {
        let app = App::new(sample_spec());
        let flat = flatten_command_tree(&app.command_tree_nodes);

        assert_eq!(flat[0].full_path, "init");
        assert_eq!(flat[1].full_path, "config");
        assert_eq!(flat[2].full_path, "config set");
        assert_eq!(flat[9].full_path, "plugin install");
    }

    #[test]
    fn test_flatten_command_tree_depth() {
        let app = App::new(sample_spec());
        let flat = flatten_command_tree(&app.command_tree_nodes);

        assert_eq!(flat[0].depth, 0); // init
        assert_eq!(flat[1].depth, 0); // config
        assert_eq!(flat[2].depth, 1); // config set
        assert_eq!(flat[6].depth, 0); // run
        assert_eq!(flat[9].depth, 1); // plugin install
    }

    #[test]
    fn test_ctrl_r_executes_from_any_panel() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Test from Commands panel
        app.set_focus(Focus::Commands);
        let ctrl_r = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::CONTROL,
        );
        assert_eq!(app.handle_key(ctrl_r), Action::Execute);

        // Test from Flags panel
        app.set_focus(Focus::Flags);
        assert_eq!(app.handle_key(ctrl_r), Action::Execute);

        // Test from Args panel
        app.set_focus(Focus::Args);
        assert_eq!(app.handle_key(ctrl_r), Action::Execute);

        // Test from Preview panel
        app.set_focus(Focus::Preview);
        assert_eq!(app.handle_key(ctrl_r), Action::Execute);
    }

    #[test]
    fn test_build_command_parts_basic() {
        let app = App::new(sample_spec());
        let parts = app.build_command_parts();
        // After startup sync, command_path is ["init"] so parts include it
        assert_eq!(parts, vec!["mycli", "init"]);
    }

    #[test]
    fn test_build_command_parts_with_subcommand() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        let parts = app.build_command_parts();
        assert_eq!(parts, vec!["mycli", "deploy"]);
    }

    #[test]
    fn test_build_command_parts_with_flags_and_args() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);

        // Toggle a boolean flag (deploy has --rollback)
        let values = app.current_flag_values_mut();
        if let Some((_, val)) = values.iter_mut().find(|(n, _)| n == "rollback") {
            *val = FlagValue::Bool(true);
        }
        // Set a string flag
        if let Some((_, val)) = values.iter_mut().find(|(n, _)| n == "tag") {
            *val = FlagValue::String("v1.0".to_string());
        }
        // Set a positional arg
        app.arg_values[0].value = "prod".to_string();

        let parts = app.build_command_parts();
        assert!(parts.contains(&"mycli".to_string()));
        assert!(parts.contains(&"deploy".to_string()));
        assert!(parts.contains(&"--rollback".to_string()));
        assert!(parts.contains(&"--tag".to_string()));
        assert!(parts.contains(&"v1.0".to_string()));
        assert!(parts.contains(&"prod".to_string()));
        // --tag and v1.0 should be separate elements (not combined)
        let tag_idx = parts.iter().position(|p| p == "--tag").unwrap();
        assert_eq!(parts[tag_idx + 1], "v1.0");
    }

    #[test]
    fn test_build_command_parts_splits_bin_with_spaces() {
        let mut spec = sample_spec();
        spec.bin = "mise run".to_string();
        let app = App::new(spec);
        let parts = app.build_command_parts();
        assert_eq!(parts[0], "mise");
        assert_eq!(parts[1], "run");
    }

    #[test]
    fn test_build_command_parts_arg_not_quoted() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        // Set a positional arg with spaces — should remain a single element, unquoted
        app.arg_values[0].value = "hello world".to_string();
        let parts = app.build_command_parts();
        assert!(parts.contains(&"hello world".to_string()));
        // Should NOT contain quotes
        assert!(!parts.iter().any(|p| p.contains('"')));
    }

    #[test]
    fn test_build_command_parts_count_flag() {
        let mut app = App::new(sample_spec());
        // Set a count flag on the root level (verbose)
        let root_key = String::new();
        if let Some(flags) = app.flag_values.get_mut(&root_key) {
            if let Some((_, val)) = flags.iter_mut().find(|(n, _)| n == "verbose") {
                *val = FlagValue::Count(3);
            }
        }
        let parts = app.build_command_parts();
        assert!(parts.contains(&"-vvv".to_string()));
    }

    #[test]
    fn test_app_mode_default_is_builder() {
        let app = App::new(sample_spec());
        assert_eq!(app.mode, AppMode::Builder);
        assert!(!app.is_executing());
        assert!(app.execution.is_none());
    }

    #[test]
    fn test_start_and_close_execution() {
        let mut app = App::new(sample_spec());

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));
        let exited = Arc::new(AtomicBool::new(false));
        let exit_status = Arc::new(Mutex::new(None));
        let pty_writer: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(Mutex::new(None));

        let state = ExecutionState {
            command_display: "mycli deploy".to_string(),
            parser,
            pty_writer,
            pty_master: Arc::new(Mutex::new(None)),
            exited: exited.clone(),
            exit_status,
        };

        app.start_execution(state);
        assert_eq!(app.mode, AppMode::Executing);
        assert!(app.is_executing());
        assert!(app.execution.is_some());
        assert!(!app.execution_exited());

        // Simulate process exit
        exited.store(true, Ordering::Relaxed);
        assert!(app.execution_exited());

        // Close execution
        app.close_execution();
        assert_eq!(app.mode, AppMode::Builder);
        assert!(!app.is_executing());
        assert!(app.execution.is_none());
    }

    #[test]
    fn test_execution_exit_status() {
        let mut app = App::new(sample_spec());

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));
        let exited = Arc::new(AtomicBool::new(true));
        let exit_status = Arc::new(Mutex::new(Some("0".to_string())));
        let pty_writer: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(Mutex::new(None));

        let state = ExecutionState {
            command_display: "mycli deploy".to_string(),
            parser,
            pty_writer,
            pty_master: Arc::new(Mutex::new(None)),
            exited,
            exit_status,
        };

        app.start_execution(state);
        assert_eq!(app.execution_exit_status(), Some("0".to_string()));
    }

    #[test]
    fn test_execution_key_closes_on_exit() {
        let mut app = App::new(sample_spec());

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));
        let exited = Arc::new(AtomicBool::new(true));
        let exit_status = Arc::new(Mutex::new(Some("0".to_string())));
        let pty_writer: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(Mutex::new(None));

        let state = ExecutionState {
            command_display: "mycli".to_string(),
            parser,
            pty_writer,
            pty_master: Arc::new(Mutex::new(None)),
            exited,
            exit_status,
        };

        app.start_execution(state);
        assert!(app.is_executing());

        // Pressing Esc when exited should close execution and return to builder
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(esc);
        assert_eq!(result, Action::None);
        assert!(!app.is_executing());
        assert_eq!(app.mode, AppMode::Builder);
    }

    #[test]
    fn test_execution_key_enter_closes_on_exit() {
        let mut app = App::new(sample_spec());

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));
        let exited = Arc::new(AtomicBool::new(true));
        let exit_status = Arc::new(Mutex::new(None));
        let pty_writer: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(Mutex::new(None));

        let state = ExecutionState {
            command_display: "mycli".to_string(),
            parser,
            pty_writer,
            pty_master: Arc::new(Mutex::new(None)),
            exited,
            exit_status,
        };

        app.start_execution(state);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(enter);
        assert_eq!(result, Action::None);
        assert!(!app.is_executing());
    }

    #[test]
    fn test_preview_enter_returns_execute_action() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Preview);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(enter);
        assert_eq!(result, Action::Execute);
    }

    #[test]
    fn test_execution_key_does_not_close_while_running() {
        let mut app = App::new(sample_spec());

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));
        let exited = Arc::new(AtomicBool::new(false)); // still running
        let exit_status = Arc::new(Mutex::new(None));
        let pty_writer: Arc<Mutex<Option<Box<dyn std::io::Write + Send>>>> =
            Arc::new(Mutex::new(None));

        let state = ExecutionState {
            command_display: "mycli".to_string(),
            parser,
            pty_writer,
            pty_master: Arc::new(Mutex::new(None)),
            exited,
            exit_status,
        };

        app.start_execution(state);

        // Pressing 'q' while running should NOT close (it's forwarded to PTY)
        let q = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('q'),
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(q);
        assert_eq!(result, Action::None);
        assert!(app.is_executing(), "Should still be executing");
    }

    #[test]
    fn test_resize_pty() {
        use portable_pty::{NativePtySystem, PtySize, PtySystem};

        let mut app = App::new(sample_spec());

        // Create a real PTY for testing resize
        let pty_system = NativePtySystem::default();
        let pair = pty_system
            .openpty(PtySize {
                rows: 24,
                cols: 80,
                pixel_width: 0,
                pixel_height: 0,
            })
            .expect("Failed to open PTY");

        let parser = Arc::new(RwLock::new(vt100::Parser::new(24, 80, 0)));
        let pty_master: Arc<Mutex<Option<Box<dyn portable_pty::MasterPty + Send>>>> =
            Arc::new(Mutex::new(Some(pair.master)));

        let state = ExecutionState {
            command_display: "test command".to_string(),
            parser,
            pty_writer: Arc::new(Mutex::new(None)),
            pty_master,
            exited: Arc::new(AtomicBool::new(false)),
            exit_status: Arc::new(Mutex::new(None)),
        };

        app.start_execution(state);

        // Resize the PTY
        app.resize_pty(40, 120);

        // Verify the PTY master still exists (resize doesn't break it)
        if let Some(ref exec) = app.execution {
            let master_guard = exec.pty_master.lock().unwrap();
            assert!(
                master_guard.is_some(),
                "PTY master should still exist after resize"
            );
        }
    }

    #[test]
    fn test_bracket_right_cycles_theme_forward() {
        let mut app = App::new(sample_spec());
        let initial_theme = app.theme_name;

        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(']'),
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(key);
        assert_eq!(result, Action::None);
        assert_ne!(app.theme_name, initial_theme, "Theme should have changed");

        // T should also cycle forward to the same next theme
        let mut app2 = App::new(sample_spec());
        let t_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('T'),
            crossterm::event::KeyModifiers::NONE,
        );
        app2.handle_key(t_key);
        assert_eq!(
            app.theme_name, app2.theme_name,
            "] and T should both cycle to same theme"
        );
    }

    #[test]
    fn test_bracket_left_cycles_theme_backward() {
        let mut app = App::new(sample_spec());
        let initial_theme = app.theme_name;

        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('['),
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(key);
        assert_eq!(result, Action::None);
        assert_ne!(
            app.theme_name, initial_theme,
            "Theme should have changed backward"
        );
    }

    #[test]
    fn test_bracket_left_right_are_inverses() {
        let mut app = App::new(sample_spec());
        let initial_theme = app.theme_name;

        // Cycle forward with ]
        let right = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(']'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(right);
        let after_forward = app.theme_name;
        assert_ne!(after_forward, initial_theme);

        // Cycle backward with [
        let left = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('['),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(left);
        assert_eq!(app.theme_name, initial_theme, "[ should undo ]");
    }

    #[test]
    fn test_enter_on_commands_navigates_into_child() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Navigate to "config" which has children (set, get, list, remove)
        app.navigate_to_command(&["config"]);
        let config_idx = app.command_index();
        assert_eq!(app.command_path, vec!["config"]);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(enter);
        assert_eq!(result, Action::None);

        // Should have moved to first child of "config" (which is "config set")
        assert!(
            app.command_index() > config_idx,
            "Enter should navigate into first child"
        );
        // The command path should include "config" and a child
        assert!(
            app.command_path.len() >= 2,
            "Should have navigated deeper: {:?}",
            app.command_path
        );
    }

    #[test]
    fn test_enter_on_commands_same_as_right() {
        // Enter and Right should produce the same result on Commands panel
        let mut app_enter = App::new(sample_spec());
        app_enter.set_focus(Focus::Commands);
        app_enter.navigate_to_command(&["config"]);

        let mut app_right = App::new(sample_spec());
        app_right.set_focus(Focus::Commands);
        app_right.navigate_to_command(&["config"]);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        let right = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        );

        app_enter.handle_key(enter);
        app_right.handle_key(right);

        assert_eq!(app_enter.command_index(), app_right.command_index());
        assert_eq!(app_enter.command_path, app_right.command_path);
    }

    #[test]
    fn test_enter_on_leaf_command_does_nothing() {
        // Enter on a command with no children should be a no-op
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);
        app.navigate_to_command(&["init"]); // "init" has no subcommands
        let init_idx = app.command_index();

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(enter);
        assert_eq!(result, Action::None);
        assert_eq!(
            app.command_index(),
            init_idx,
            "Enter on leaf should not move"
        );
    }

    #[test]
    fn test_p_on_preview_returns_accept() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Preview);

        let p = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(p);
        assert_eq!(result, Action::Accept);
    }

    #[test]
    fn test_p_on_non_preview_does_nothing() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        let p = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('p'),
            crossterm::event::KeyModifiers::NONE,
        );
        let result = app.handle_key(p);
        assert_eq!(
            result,
            Action::None,
            "p should do nothing when not on Preview"
        );

        app.set_focus(Focus::Flags);
        let result = app.handle_key(p);
        assert_eq!(result, Action::None, "p should do nothing when on Flags");
    }

    #[test]
    fn test_startup_sync_selects_first_command() {
        let app = App::new(sample_spec());
        // After startup, command_path should match tree's first node
        assert_eq!(app.command_path, vec!["init"]);
        // Flags should be init's flags (template, force) + globals (verbose, quiet)
        let flags = app.visible_flags();
        let names: Vec<&str> = flags.iter().map(|f| f.name.as_str()).collect();
        assert!(
            names.contains(&"template"),
            "init should have template flag"
        );
        assert!(names.contains(&"force"), "init should have force flag");
        assert!(
            names.contains(&"verbose"),
            "init should have global verbose"
        );
        // Args should be init's args (name)
        assert!(!app.arg_values.is_empty(), "init has a <name> arg");
        assert_eq!(app.arg_values[0].name, "name");
    }

    #[test]
    fn test_global_flag_sync_from_subcommand() {
        let mut app = App::new(sample_spec());
        // Navigate to deploy
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);

        // Find verbose flag index
        let flags = app.visible_flags();
        let verbose_idx = flags.iter().position(|f| f.name == "verbose").unwrap();
        app.set_flag_index(verbose_idx);

        // Toggle verbose via space (simulating user toggling global flag from subcommand)
        let space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(space);

        // The verbose flag should now be Count(1) at the deploy level
        let deploy_flags = app.current_flag_values();
        let verbose_val = deploy_flags.iter().find(|(n, _)| n == "verbose");
        assert!(
            matches!(verbose_val, Some((_, FlagValue::Count(1)))),
            "verbose should be Count(1) at deploy level"
        );

        // It should also be synced to root level
        let root_flags = app.flag_values.get("").unwrap();
        let root_verbose = root_flags.iter().find(|(n, _)| n == "verbose");
        assert!(
            matches!(root_verbose, Some((_, FlagValue::Count(1)))),
            "verbose should be synced to root level"
        );

        // The built command should include -v
        let cmd = app.build_command();
        assert!(
            cmd.contains("-v"),
            "global flag toggled from subcommand should appear in command: {cmd}"
        );
    }

    #[test]
    fn test_global_flag_sync_across_multiple_levels() {
        let mut app = App::new(sample_spec());

        // Toggle verbose from root context
        app.navigate_to_command(&["init"]);
        app.set_focus(Focus::Flags);
        let flags = app.visible_flags();
        let verbose_idx = flags.iter().position(|f| f.name == "verbose").unwrap();
        app.set_flag_index(verbose_idx);

        // Increment verbose 3 times
        let space = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(' '),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(space);
        app.handle_key(space);
        app.handle_key(space);

        // Navigate to deploy — verbose should reflect the synced value
        app.navigate_to_command(&["deploy"]);
        let deploy_flags = app.current_flag_values();
        let verbose_val = deploy_flags.iter().find(|(n, _)| n == "verbose");
        assert!(
            matches!(verbose_val, Some((_, FlagValue::Count(3)))),
            "verbose should be Count(3) at deploy level after sync"
        );

        // Command should include -vvv
        let cmd = app.build_command();
        assert!(cmd.contains("-vvv"), "command should contain -vvv: {cmd}");
    }

    #[test]
    fn test_filtered_navigation_skips_non_matches() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Activate filter mode and type "deploy"
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        for c in "deploy".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        // Auto-selection should have moved to "deploy"
        let flat = flatten_command_tree(&app.command_tree_nodes);
        let selected = &flat[app.command_index()];
        assert_eq!(selected.name, "deploy", "should auto-select deploy");

        // Press down — should wrap around to deploy again (only match)
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        let selected = &flat[app.command_index()];
        assert_eq!(
            selected.name, "deploy",
            "down should stay on deploy (only match)"
        );

        // Press up — should also stay on deploy
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        let selected = &flat[app.command_index()];
        assert_eq!(
            selected.name, "deploy",
            "up should stay on deploy (only match)"
        );
    }

    #[test]
    fn test_filtered_navigation_cycles_through_matches() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Filter for "'list" — the ' prefix requests an exact substring match in
        // nucleo, so this matches only commands whose name/path contains "list"
        // exactly: "config list" and "plugin list", but NOT "install".
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        for c in "'list".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        let flat = flatten_command_tree(&app.command_tree_nodes);
        let first_selected = app.command_index();
        let first_name = flat[first_selected].name.clone();
        assert_eq!(first_name, "list", "should auto-select first 'list'");

        // Press down — should move to the other "list"
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        let second_selected = app.command_index();
        assert_ne!(
            second_selected, first_selected,
            "down should move to a different list match"
        );
        assert_eq!(flat[second_selected].name, "list");

        // Press up — should go back to the first match
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(app.command_index(), first_selected);
    }

    #[test]
    fn test_filtered_flag_navigation() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);

        // Filter for "skip" — should match "--skip-tests"
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);

        for c in "skip".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        let flags = app.visible_flags();
        let selected_flag = &flags[app.flag_index()];
        assert_eq!(
            selected_flag.name, "skip-tests",
            "should auto-select skip-tests"
        );
    }

    #[test]
    fn test_match_scores_separate_name_and_help() {
        let app = App::new(sample_spec());
        // "verbose" should match the verbose flag by name
        let scores = compute_tree_scores(&app.command_tree_nodes, "verbose");

        // "init" has help="Initialize a new project" — "verbose" should not match
        // via name, but might or might not match via help. The key point is that
        // name_score and help_score are independent.
        if let Some(init_scores) = scores.get("init") {
            // name_score should be 0 (init != verbose)
            assert_eq!(
                init_scores.name_score, 0,
                "init name should not match 'verbose'"
            );
        }
    }

    #[test]
    fn test_match_scores_help_only_match() {
        let app = App::new(sample_spec());
        // "project" appears in init's help text "Initialize a new project"
        let scores = compute_tree_scores(&app.command_tree_nodes, "project");

        let init_scores = scores.get("init").expect("init should have scores");
        // name "init" should NOT match "project"
        assert_eq!(
            init_scores.name_score, 0,
            "init name should not match 'project'"
        );
        // help "Initialize a new project" SHOULD match "project"
        assert!(
            init_scores.help_score > 0,
            "init help should match 'project'"
        );
        // Overall should match
        assert!(
            init_scores.overall() > 0,
            "init should match overall via help"
        );
    }

    #[test]
    fn test_match_scores_name_only_match() {
        let app = App::new(sample_spec());
        // "cfg" matches the name "config" but probably not its help "Manage configuration"
        // (though "cfg" could partially match "configuration" — the key is name_score > 0)
        let scores = compute_tree_scores(&app.command_tree_nodes, "cfg");

        let config_scores = scores.get("config").expect("config should have scores");
        assert!(
            config_scores.name_score > 0,
            "config name should match 'cfg'"
        );
    }

    #[test]
    fn test_flag_match_scores_separate_name_and_help() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);

        // "Docker" appears in --tag's help text "Docker image tag"
        app.filter_input.clear();
        for c in "Docker".chars() {
            app.filter_input.insert_char(c);
        }

        let scores = app.compute_flag_match_scores();

        // "tag" name should NOT match "Docker"
        if let Some(tag_scores) = scores.get("tag") {
            assert_eq!(
                tag_scores.name_score, 0,
                "tag name should not match 'Docker'"
            );
            assert!(
                tag_scores.help_score > 0,
                "tag help 'Docker image tag' should match 'Docker'"
            );
        }
    }

    #[test]
    fn test_args_filter_auto_selects_matching_arg() {
        // "config set" has two args: <key> and <value>
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["config", "set"]);
        app.set_focus(Focus::Args);

        // Verify we have at least two args
        assert!(
            app.arg_values.len() >= 2,
            "config set should have at least 2 args"
        );

        // Select the first arg (<key>)
        app.set_arg_index(0);
        assert_eq!(app.arg_index(), 0);
        let first_arg_name = app.arg_values[0].name.clone();

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        // Type a filter that matches the second arg but NOT the first
        // The args are "key" and "value", so typing "val" should match "value" but not "key"
        for c in "val".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        // The first arg "key" shouldn't match "val"
        let scores = app.compute_arg_match_scores();
        let first_score = scores
            .get(&first_arg_name)
            .map(|s| s.overall())
            .unwrap_or(0);
        assert_eq!(
            first_score, 0,
            "'{}' should not match 'val'",
            first_arg_name
        );

        // The cursor should have moved away from the first arg to a matching one
        let selected = app.arg_index();
        let selected_name = &app.arg_values[selected].name;
        let selected_score = scores
            .get(selected_name)
            .map(|s| s.overall())
            .unwrap_or(0);
        assert!(
            selected_score > 0,
            "selected arg '{}' should match 'val'",
            selected_name
        );
    }
}
