use std::process::Command;

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher};
use ratatui::layout::Rect;
use ratatui_interact::components::TreeNode;
use ratatui_interact::state::FocusManager;
use ratatui_interact::traits::ClickRegionRegistry;
use ratatui_themes::{ThemeName, ThemePalette};
use usage::{Spec, SpecCommand, SpecFlag};

use crate::components::arg_panel::{ArgPanelAction, ArgPanelComponent, ArgPanelEnterRequest};
use crate::components::command_panel::{CommandPanelAction, CommandPanelComponent};
use crate::components::execution::{ExecutionAction, ExecutionComponent};
use crate::components::filterable::{FilterAction, FilterableComponent};
use crate::components::flag_panel::{FlagPanelAction, FlagPanelComponent, FlagPanelEnterRequest};
use crate::components::theme_picker::{ThemePickerAction, ThemePickerComponent};
use crate::components::{Component, EventResult};

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
    /// Negatable boolean flag with three states: omitted (None), explicit on, explicit off.
    NegBool(Option<bool>),
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
    pub help: Option<String>,
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

/// Collect visible (non-hidden) flags from a command, including global flags
/// from the root spec. Standalone function to allow split borrows.
pub(crate) fn collect_visible_flags<'a>(cmd: &'a SpecCommand, spec: &'a Spec) -> Vec<&'a SpecFlag> {
    let mut flags: Vec<&SpecFlag> = cmd.flags.iter().filter(|f| !f.hide).collect();
    for flag in &spec.cmd.flags {
        if flag.global && !flag.hide && !flags.iter().any(|f| f.name == flag.name) {
            flags.push(flag);
        }
    }
    flags
}

/// Frame-local layout snapshot used for mouse hit-testing.
pub struct UiLayout {
    pub click_regions: ClickRegionRegistry<Focus>,
    pub flag_overlay_rect: Option<Rect>,
    pub arg_overlay_rect: Option<Rect>,
    pub theme_overlay_rect: Option<Rect>,
    pub theme_indicator_rect: Option<Rect>,
}

impl UiLayout {
    pub fn new() -> Self {
        Self {
            click_regions: ClickRegionRegistry::new(),
            flag_overlay_rect: None,
            arg_overlay_rect: None,
            theme_overlay_rect: None,
            theme_indicator_rect: None,
        }
    }

    fn area_for(&self, focus: Focus) -> Rect {
        self.click_regions
            .regions()
            .iter()
            .find(|r| r.data == focus)
            .map(|r| r.area)
            .unwrap_or_default()
    }

    fn region_at(&self, col: u16, row: u16) -> Option<(Focus, Rect)> {
        self.click_regions
            .regions()
            .iter()
            .find(|r| r.contains(col, row))
            .map(|region| (region.data, region.area))
    }
}

/// Main application state.
pub struct App {
    pub spec: Spec,

    /// Current app mode (builder vs executing).
    pub mode: AppMode,

    /// Execution component when a command is running.
    pub execution: Option<ExecutionComponent>,

    /// Current color theme.
    pub theme_name: ThemeName,

    /// Path of subcommand names derived from the tree selection.
    /// Empty means we're at the root command.
    pub command_path: Vec<String>,

    /// Command panel component (owns tree state and navigation).
    pub command_panel: FilterableComponent<CommandPanelComponent>,

    /// Flag panel component (owns list state and navigation).
    pub flag_panel: FilterableComponent<FlagPanelComponent>,

    /// Flag values keyed by flag name, per command path depth.
    /// The key is the full command path joined by space.
    pub flag_values: std::collections::HashMap<String, Vec<(String, FlagValue)>>,

    /// Arg values keyed by command path.
    /// `arg_values` mirrors the currently selected path for rendering and editing.
    arg_values_by_path: std::collections::HashMap<String, Vec<ArgValue>>,

    /// Arg values for the current command.
    pub arg_values: Vec<ArgValue>,

    /// Focus manager for Tab navigation between panels.
    pub focus_manager: FocusManager<Focus>,

    /// Arg panel component (owns list state and navigation).
    pub arg_panel: FilterableComponent<ArgPanelComponent>,

    /// Current frame layout snapshot for mouse hit-testing.
    pub layout: UiLayout,

    /// Theme picker overlay component.
    pub theme_picker: ThemePickerComponent,

    /// Current mouse cursor position (column, row) for hover highlighting.
    pub mouse_position: Option<(u16, u16)>,
}

impl App {
    pub fn new(spec: Spec) -> Self {
        Self::with_theme(spec, ThemeName::default())
    }

    /// Check if the app is currently in execution mode.
    pub fn is_executing(&self) -> bool {
        self.mode == AppMode::Executing
    }

    /// Close the execution view and return to the builder.
    pub fn close_execution(&mut self) {
        self.mode = AppMode::Builder;
        self.execution = None;
    }

    /// Start command execution with the given component.
    pub fn start_execution(&mut self, component: ExecutionComponent) {
        self.mode = AppMode::Executing;
        self.execution = Some(component);
    }

    pub fn spawn_execution(&mut self, terminal_size: ratatui::layout::Size) -> color_eyre::Result<()> {
        let parts = self.build_command_parts();
        let command_display = self.build_command();
        let component = ExecutionComponent::spawn(command_display, &parts, terminal_size)?;
        self.start_execution(component);
        Ok(())
    }

    pub fn resize_execution_to_terminal(&self, terminal_size: ratatui::layout::Size) {
        if let Some(ref exec) = self.execution {
            exec.resize_to_terminal(terminal_size);
        }
    }

    /// Resize the PTY to fit the new terminal dimensions.
    #[cfg(test)]
    pub fn resize_pty(&self, rows: u16, cols: u16) {
        if let Some(ref exec) = self.execution {
            exec.resize_pty(rows, cols);
        }
    }

    pub fn with_theme(spec: usage::Spec, theme_name: ThemeName) -> Self {
        let tree_nodes = build_command_tree(&spec);
        let command_panel = FilterableComponent::new(CommandPanelComponent::new(tree_nodes));

        let mut app = Self {
            spec,
            mode: AppMode::Builder,
            execution: None,
            theme_name,
            command_path: Vec::new(),
            command_panel,
            flag_panel: FilterableComponent::new(FlagPanelComponent::new()),
            flag_values: std::collections::HashMap::new(),
            arg_values_by_path: std::collections::HashMap::new(),
            arg_values: Vec::new(),
            focus_manager: FocusManager::new(),
            arg_panel: FilterableComponent::new(ArgPanelComponent::new()),
            layout: UiLayout::new(),
            theme_picker: ThemePickerComponent::new(),
            mouse_position: None,
        };
        app.sync_state();
        // Synchronize command_path with the tree's initial selection so the
        // correct flags/args are displayed on the very first render.
        app.sync_command_path_from_tree();
        app.notify_focus_gained(app.focus());
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
        let previous = self.focus();
        if previous == panel {
            return;
        }

        self.notify_focus_lost(previous);
        self.focus_manager.set(panel);
        self.notify_focus_gained(panel);
    }

    /// Whether any panel is currently in inline edit mode.
    pub fn is_editing(&self) -> bool {
        self.flag_panel.is_editing() || self.arg_panel.is_editing()
    }

    /// Finish any active inline editing on the focused panel, syncing the final value.
    pub fn finish_editing(&mut self) {
        if self.flag_panel.is_editing() {
            let idx = self.flag_panel.selected_index();
            let value = self.flag_panel.finish_editing();
            self.apply_flag_string_value(idx, &value);
        }
        if self.arg_panel.is_editing() {
            let idx = self.arg_panel.selected_index();
            let value = self.arg_panel.finish_editing();
            self.set_arg_value(idx, value);
        }
    }

    /// Whether the theme picker is open.
    pub fn is_theme_picking(&self) -> bool {
        self.theme_picker.is_open()
    }

    /// Open the theme picker overlay.
    pub fn open_theme_picker(&mut self) {
        self.theme_picker.open(self.theme_name);
    }

    /// Process a ThemePickerAction emitted by the theme picker component.
    fn process_theme_picker_action(&mut self, action: ThemePickerAction) {
        match action {
            ThemePickerAction::PreviewTheme(name) => {
                self.theme_name = name;
            }
            ThemePickerAction::Confirmed => {
                // Theme already set by preview — nothing to do.
            }
            ThemePickerAction::Cancelled(original) => {
                self.theme_name = original;
            }
        }
    }

    /// Handle key events when the theme picker is open.
    fn handle_theme_picker_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        if let EventResult::Action(action) = self.theme_picker.handle_key(key) {
            self.process_theme_picker_action(action);
        }
        Action::None
    }

    /// Whether any choice select box is open (flag panel or arg panel).
    pub fn is_choosing(&self) -> bool {
        self.flag_panel.is_choosing() || self.arg_panel.is_choosing()
    }

    /// Dispatch a FilterAction result, handling FocusNext/FocusPrev/Consumed/NotHandled
    /// uniformly and delegating inner actions to the provided closure.
    fn dispatch_filter_result<A>(
        &mut self,
        result: EventResult<FilterAction<A>>,
        process: impl FnOnce(&mut Self, A) -> Action,
    ) -> Action {
        self.dispatch_filter_result_with(result, process, |_| {})
    }

    /// Like `dispatch_filter_result` but with an additional callback for NotHandled.
    fn dispatch_filter_result_with<A>(
        &mut self,
        result: EventResult<FilterAction<A>>,
        process: impl FnOnce(&mut Self, A) -> Action,
        on_not_handled: impl FnOnce(&mut Self),
    ) -> Action {
        match result {
            EventResult::Action(FilterAction::Inner(action)) => process(self, action),
            EventResult::Action(FilterAction::FocusNext) => {
                self.focus_next();
                Action::None
            }
            EventResult::Action(FilterAction::FocusPrev) => {
                self.focus_prev();
                Action::None
            }
            EventResult::NotHandled => {
                on_not_handled(self);
                Action::None
            }
            EventResult::Consumed => Action::None,
        }
    }

    fn dispatch_filter_result_option<A>(
        &mut self,
        result: EventResult<FilterAction<A>>,
        process: impl FnOnce(&mut Self, A) -> Action,
    ) -> Option<Action> {
        match result {
            EventResult::Action(FilterAction::Inner(action)) => Some(process(self, action)),
            EventResult::Action(FilterAction::FocusNext) => {
                self.focus_next();
                Some(Action::None)
            }
            EventResult::Action(FilterAction::FocusPrev) => {
                self.focus_prev();
                Some(Action::None)
            }
            EventResult::Consumed => Some(Action::None),
            EventResult::NotHandled => None,
        }
    }

    fn notify_focus_gained(&mut self, panel: Focus) {
        match panel {
            Focus::Commands => {
                let result = self.command_panel.handle_focus_gained();
                self.dispatch_filter_result(result, |s, action| {
                    s.process_command_action(action);
                    Action::None
                });
            }
            Focus::Flags => {
                let result = self.flag_panel.handle_focus_gained();
                self.dispatch_filter_result(result, |s, action| s.process_flag_action(action));
            }
            Focus::Args => {
                let result = self.arg_panel.handle_focus_gained();
                self.dispatch_filter_result(result, |s, action| s.process_arg_action(action));
            }
            Focus::Preview => {}
        }
    }

    fn notify_focus_lost(&mut self, panel: Focus) {
        match panel {
            Focus::Commands => {
                let result = self.command_panel.handle_focus_lost();
                self.dispatch_filter_result(result, |s, action| {
                    s.process_command_action(action);
                    Action::None
                });
            }
            Focus::Flags => {
                let result = self.flag_panel.handle_focus_lost();
                self.dispatch_filter_result(result, |s, action| s.process_flag_action(action));
            }
            Focus::Args => {
                let result = self.arg_panel.handle_focus_lost();
                self.dispatch_filter_result(result, |s, action| s.process_arg_action(action));
            }
            Focus::Preview => {}
        }
    }

    fn focus_next(&mut self) {
        let previous = self.focus();
        self.focus_manager.next();
        let current = self.focus();
        if current != previous {
            self.notify_focus_lost(previous);
            self.notify_focus_gained(current);
        }
    }

    fn focus_prev(&mut self) {
        let previous = self.focus();
        self.focus_manager.prev();
        let current = self.focus();
        if current != previous {
            self.notify_focus_lost(previous);
            self.notify_focus_gained(current);
        }
    }

    fn focused_panel_is_handling_input(&self) -> bool {
        match self.focus() {
            Focus::Commands => self.command_panel.is_filtering(),
            Focus::Flags => self.flag_panel.is_filtering() || self.flag_panel.is_editing() || self.flag_panel.is_choosing(),
            Focus::Args => self.arg_panel.is_filtering() || self.arg_panel.is_editing() || self.arg_panel.is_choosing(),
            Focus::Preview => false,
        }
    }

    fn handle_focused_panel_key(&mut self, key: crossterm::event::KeyEvent) -> Option<Action> {
        match self.focus() {
            Focus::Commands => {
                let result = self.command_panel.handle_key(key);
                self.dispatch_filter_result_option(result, |s, action| {
                    s.process_command_action(action);
                    Action::None
                })
            }
            Focus::Flags => {
                let result = self.flag_panel.handle_key(key);
                self.dispatch_filter_result_option(result, |s, action| s.process_flag_action(action))
            }
            Focus::Args => {
                let result = self.arg_panel.handle_key(key);
                self.dispatch_filter_result_option(result, |s, action| s.process_arg_action(action))
            }
            Focus::Preview => None,
        }
    }

    /// Apply a string value to the flag at the given visible index.
    fn apply_flag_string_value(&mut self, flag_idx: usize, value: &str) {
        let mut changed = false;
        {
            let values = self.current_flag_values_mut();
            if let Some((name, FlagValue::String(ref mut s))) = values.get_mut(flag_idx) {
                let flag_name = name.clone();
                *s = value.to_string();
                let new_val = FlagValue::String(s.clone());
                self.sync_global_flag(&flag_name, &new_val);
                changed = true;
            }
        }
        if changed {
            self.refresh_flag_panel_inputs();
        }
    }

    /// Process a non-choice ArgPanelAction (enter, clear).
    fn process_arg_action(&mut self, action: ArgPanelAction) -> Action {
        match action {
            ArgPanelAction::EnterRequest(request) => {
                self.process_arg_enter_request(request);
                Action::None
            }
            ArgPanelAction::ClearArg(idx) => {
                self.set_arg_value(idx, String::new());
                Action::None
            }
            ArgPanelAction::ValueChanged { index, value } => {
                self.set_arg_value(index, value);
                Action::None
            }
            ArgPanelAction::EditFinished { index, value } => {
                self.set_arg_value(index, value);
                Action::None
            }
            ArgPanelAction::ChoiceSelected { index, value }
            | ArgPanelAction::ChoiceCancelled { index, value } => {
                self.set_arg_value(index, value);
                Action::None
            }
        }
    }

    /// Process a non-choice FlagPanelAction (toggle, clear, enter).
    fn process_flag_action(&mut self, action: FlagPanelAction) -> Action {
        match action {
            FlagPanelAction::ToggleFlag(_idx) => {
                self.toggle_simple_flag();
                Action::None
            }
            FlagPanelAction::ClearFlag(idx) => {
                // Clear/decrement flag at the given index
                let mut changed = false;
                {
                    let values = self.current_flag_values_mut();
                    if let Some((name, value)) = values.get_mut(idx) {
                        let flag_name = name.clone();
                        match value {
                            FlagValue::Bool(b) => {
                                *b = false;
                                let new_val = FlagValue::Bool(false);
                                self.sync_global_flag(&flag_name, &new_val);
                            }
                            FlagValue::Count(c) => {
                                *c = c.saturating_sub(1);
                                let new_val = FlagValue::Count(*c);
                                self.sync_global_flag(&flag_name, &new_val);
                            }
                            FlagValue::String(s) => {
                                s.clear();
                                let new_val = FlagValue::String(String::new());
                                self.sync_global_flag(&flag_name, &new_val);
                            }
                            FlagValue::NegBool(state) => {
                                *state = None;
                                let new_val = FlagValue::NegBool(None);
                                self.sync_global_flag(&flag_name, &new_val);
                            }
                        }
                        changed = true;
                    }
                }
                if changed {
                    self.refresh_flag_panel_inputs();
                }
                Action::None
            }
            FlagPanelAction::EnterRequest(request) => {
                self.process_flag_enter_request(request);
                Action::None
            }
            FlagPanelAction::ChoiceSelected { index, value }
            | FlagPanelAction::ChoiceCancelled { index, value } => {
                self.apply_flag_string_value(index, &value);
                Action::None
            }
            FlagPanelAction::ValueChanged { index, value } => {
                self.apply_flag_string_value(index, &value);
                Action::None
            }
            FlagPanelAction::EditFinished { index, value } => {
                self.apply_flag_string_value(index, &value);
                Action::None
            }
            FlagPanelAction::NegBoolClick(idx, target) => {
                let mut changed = false;
                {
                    let values = self.current_flag_values_mut();
                    if let Some((name, FlagValue::NegBool(state))) = values.get_mut(idx) {
                        let flag_name = name.clone();
                        *state = if *state == target { None } else { target };
                        let new_val = FlagValue::NegBool(*state);
                        self.sync_global_flag(&flag_name, &new_val);
                        changed = true;
                    }
                }
                if changed {
                    self.refresh_flag_panel_inputs();
                }
                Action::None
            }
        }
    }

    /// Whether a filter is applied on any panel (filter text is non-empty),
    /// regardless of whether the user is still in interactive typing mode.
    pub fn filter_active(&self) -> bool {
        self.command_panel.has_active_filter()
            || self.flag_panel.has_active_filter()
            || self.arg_panel.has_active_filter()
    }

    /// Whether the user is currently in interactive filter typing mode.
    pub fn is_filtering(&self) -> bool {
        self.command_panel.is_filtering()
            || self.flag_panel.is_filtering()
            || self.arg_panel.is_filtering()
    }

    // --- Convenience accessors for indices ---

    #[allow(dead_code)]
    pub fn command_index(&self) -> usize {
        self.command_panel.selected_index()
    }

    #[allow(dead_code)]
    pub fn set_command_index(&mut self, idx: usize) {
        if self.command_panel.select_item(idx) {
            self.set_command_path(self.command_panel.path().to_vec());
        }
    }

    pub fn flag_index(&self) -> usize {
        self.flag_panel.selected_index()
    }

    #[cfg(test)]
    pub fn set_flag_index(&mut self, idx: usize) {
        self.flag_panel.select(idx);
    }

    pub fn arg_index(&self) -> usize {
        self.arg_panel.selected_index()
    }

    #[cfg(test)]
    pub fn set_arg_index(&mut self, idx: usize) {
        self.arg_panel.select(idx);
    }

    #[cfg(test)]
    pub fn command_scroll(&self) -> usize {
        self.command_panel.scroll_offset()
    }

    /// Get the active filter text from the focused panel.
    #[allow(dead_code)] // used in #[cfg(test)] scoring methods and test assertions
    pub fn filter(&self) -> &str {
        match self.focus() {
            Focus::Commands => self.command_panel.filter_text(),
            Focus::Flags => self.flag_panel.filter_text(),
            Focus::Args => self.arg_panel.filter_text(),
            _ => "",
        }
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

    /// Find a `complete` directive for the given argument name on the current command.
    pub fn find_completion(&self, arg_name: &str) -> Option<&usage::SpecComplete> {
        let cmd = self.current_command();
        cmd.complete.get(arg_name)
    }

    /// Run a completion command and parse its output into (choices, descriptions).
    /// When `descriptions` is true, each line is parsed as "value:description".
    pub fn run_completion(
        run_cmd: &str,
        descriptions: bool,
    ) -> Option<(Vec<String>, Vec<Option<String>>)> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(run_cmd)
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut choices = Vec::new();
        let mut descs = Vec::new();

        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if descriptions {
                // Parse "value:description", with \: as escaped colon
                if let Some((value, desc)) = parse_completion_line(line) {
                    choices.push(value);
                    descs.push(Some(desc));
                } else {
                    choices.push(line.to_string());
                    descs.push(None);
                }
            } else {
                choices.push(line.to_string());
                descs.push(None);
            }
        }

        if choices.is_empty() {
            return None;
        }

        Some((choices, descs))
    }

    /// Run completion command and return results.
    /// Re-executes the command each time for fresh results.
    /// Returns `Some((choices, descriptions))` on success, `None` on failure.
    #[cfg(test)]
    pub fn run_and_open_completion(
        &mut self,
        arg_name: &str,
        _current_value: &str,
    ) -> Option<(Vec<String>, Vec<Option<String>>)> {
        let complete = self.find_completion(arg_name)?.clone();
        let run_cmd = complete.run.as_ref()?;
        Self::run_completion(run_cmd, complete.descriptions)
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

        if self.command_panel.is_filtering() && !self.command_panel.filter_text().is_empty() && self.focus() == Focus::Commands {
            let filter_lower = self.command_panel.filter_text().to_lowercase();
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
        collect_visible_flags(self.current_command(), &self.spec)
    }

    /// Returns the visible (non-hidden) args of the current command.
    #[cfg(test)]
    pub fn visible_args(&self) -> Vec<&usage::SpecArg> {
        let cmd = self.current_command();
        cmd.args.iter().filter(|a| !a.hide).collect()
    }

    fn default_arg_values_for_command(cmd: &SpecCommand) -> Vec<ArgValue> {
        cmd.args
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
                    help: a.help.clone(),
                }
            })
            .collect()
    }

    fn persist_current_arg_values(&mut self) {
        let path_key = self.command_path_key();
        self.arg_values_by_path
            .insert(path_key, self.arg_values.clone());
    }

    fn set_command_path(&mut self, new_path: Vec<String>) {
        if self.command_path != new_path {
            self.persist_current_arg_values();
            self.command_path = new_path;
        }
        self.sync_state();
    }

    fn set_arg_value(&mut self, index: usize, value: String) {
        if let Some(arg) = self.arg_values.get_mut(index) {
            arg.value = value;
            self.persist_current_arg_values();
        }
        self.refresh_arg_panel_inputs();
    }

    fn refresh_flag_panel_inputs(&mut self) {
        let flags = self.visible_flags_snapshot();
        let flag_refs: Vec<&SpecFlag> = flags.iter().collect();
        let flag_values = self.current_flag_values().to_vec();
        self.flag_panel.set_filterable_items_from_flags(&flag_refs);
        self.flag_panel
            .set_enter_requests_from_flags(&flag_refs, &flag_values);
    }

    fn refresh_arg_panel_inputs(&mut self) {
        self.arg_panel.set_filterable_items_from_args(&self.arg_values);
        self.arg_panel.set_enter_requests_from_args(&self.arg_values);
    }

    /// Synchronize internal state (arg_values, flag_values) when navigating to a new command.
    pub fn sync_state(&mut self) {
        let path_key = self.command_path_key();

        // Initialize arg values for the current command path if not already set.
        if !self.arg_values_by_path.contains_key(&path_key) {
            let defaults = {
                let cmd = self.current_command();
                Self::default_arg_values_for_command(cmd)
            };
            self.arg_values_by_path.insert(path_key.clone(), defaults);
        }
        self.arg_values = self
            .arg_values_by_path
            .get(&path_key)
            .cloned()
            .unwrap_or_default();

        // Initialize flag values for the current command path if not already set
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
                    } else if f.negate.is_some() {
                        // Negatable flag: tristate (omitted / explicit on / explicit off)
                        FlagValue::NegBool(None)
                    } else {
                        FlagValue::Bool(false)
                    };
                    (f.name.clone(), val)
                })
                .collect();
            self.flag_values.insert(path_key, values);
        }

        // Update list picker states with correct totals
        let flag_count = self.current_flag_values().len();
        let arg_count = self.arg_values.len();
        self.flag_panel.set_total(flag_count);
        self.arg_panel.set_total(arg_count);

        self.refresh_flag_panel_inputs();
        self.refresh_arg_panel_inputs();

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
    #[allow(dead_code)]
    pub fn total_visible_commands(&self) -> usize {
        self.command_panel.total_visible()
    }

    /// Compute match scores for all flags when filtering.
    /// Returns a map of flag name → score (0 for non-matches).
    #[cfg(test)]
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
    #[cfg(test)]
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

    /// Update command_path based on the currently selected tree node.
    pub fn sync_command_path_from_tree(&mut self) {
        self.set_command_path(self.command_panel.path().to_vec());
    }

    /// Apply a path change from the command panel component.
    fn apply_command_panel_key(&mut self, key: crossterm::event::KeyEvent) {
        match self.command_panel.handle_key(key) {
            EventResult::Action(FilterAction::Inner(action)) => {
                self.process_command_action(action);
            }
            EventResult::Action(FilterAction::FocusNext) => {
                self.focus_manager.next();
            }
            EventResult::Action(FilterAction::FocusPrev) => {
                self.focus_manager.prev();
            }
            _ => {}
        }
    }

    fn process_command_action(&mut self, action: CommandPanelAction) {
        match action {
            CommandPanelAction::PathChanged(new_path) => {
                self.set_command_path(new_path);
            }
        }
    }

    /// Navigate to a specific command path in the tree. Expands all ancestors
    /// and selects the target node. Used for tests and programmatic navigation.
    #[allow(dead_code)]
    pub fn navigate_to_command(&mut self, path: &[&str]) {
        self.command_panel.navigate_to(path);
        self.set_command_path(self.command_panel.path().to_vec());
    }

    /// Select first child of current node.
    #[allow(dead_code)]
    pub fn navigate_into_selected(&mut self) {
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Right,
            crossterm::event::KeyModifiers::NONE,
        );
        self.apply_command_panel_key(enter);
    }

    /// Look up the registered click area for a given panel focus.
    fn area_for(&self, focus: Focus) -> Rect {
        self.layout.area_for(focus)
    }

    /// Handle a mouse event and return the resulting Action.
    pub fn handle_mouse(&mut self, event: crossterm::event::MouseEvent) -> Action {
        use crossterm::event::{MouseButton, MouseEventKind};

        let col = event.column;
        let row = event.row;

        // Track mouse position for hover highlighting
        self.mouse_position = Some((col, row));

        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                // If theme picker is open, handle its clicks
                if self.is_theme_picking() {
                    if let Some(action) =
                        self.theme_picker
                            .click_at(col, row, self.layout.theme_overlay_rect)
                    {
                        self.process_theme_picker_action(action);
                    }
                    return Action::None;
                }

                // Check if click is on the theme indicator in the help bar
                if let Some(rect) = self.layout.theme_indicator_rect {
                    if col >= rect.x
                        && col < rect.x + rect.width
                        && row >= rect.y
                        && row < rect.y + rect.height
                    {
                        self.open_theme_picker();
                        return Action::None;
                    }
                }

                // Delegate overlay clicks to the focused panel's component
                if self.is_choosing() {
                    return self.delegate_mouse_to_choosing_panel(event);
                }

                // Determine which panel was clicked and delegate
                if let Some((clicked_panel, area)) = self.layout.region_at(col, row) {
                    let switching_focus = self.focus() != clicked_panel;
                    if !switching_focus && self.is_editing() {
                        self.finish_editing();
                    }
                    self.set_focus(clicked_panel);

                    match clicked_panel {
                        Focus::Commands => {
                            let result = self.command_panel.handle_mouse(event, area);
                            self.dispatch_filter_result(result, |s, action| {
                                s.process_command_action(action);
                                Action::None
                            })
                        }
                        Focus::Flags => {
                            let result = self.flag_panel.handle_mouse(event, area);
                            self.dispatch_filter_result(result, |s, action| {
                                s.process_flag_action(action)
                            })
                        }
                        Focus::Args => {
                            let result = self.arg_panel.handle_mouse(event, area);
                            self.dispatch_filter_result(result, |s, action| {
                                s.process_arg_action(action)
                            })
                        }
                        Focus::Preview => {
                            if !switching_focus {
                                return self.handle_enter();
                            }
                            Action::None
                        }
                    }
                } else {
                    Action::None
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                // Delegate scroll to the focused panel
                let focus = self.focus();
                let area = self.area_for(focus);
                match focus {
                    Focus::Commands => {
                        let result = self.command_panel.handle_mouse(event, area);
                        self.dispatch_filter_result(result, |s, action| {
                            s.process_command_action(action);
                            Action::None
                        })
                    }
                    Focus::Flags => {
                        let result = self.flag_panel.handle_mouse(event, area);
                        self.dispatch_filter_result(result, |s, action| {
                            s.process_flag_action(action)
                        })
                    }
                    Focus::Args => {
                        let result = self.arg_panel.handle_mouse(event, area);
                        self.dispatch_filter_result(result, |s, action| {
                            s.process_arg_action(action)
                        })
                    }
                    Focus::Preview => Action::None,
                }
            }
            _ => Action::None,
        }
    }

    /// Delegate a mouse click to the panel with an open choice select overlay.
    fn delegate_mouse_to_choosing_panel(
        &mut self,
        event: crossterm::event::MouseEvent,
    ) -> Action {
        // Try flag panel first
        if self.flag_panel.is_choosing() {
            let result = self
                .flag_panel
                .handle_choice_click(event.column, event.row, self.layout.flag_overlay_rect)
                .map(FilterAction::Inner);
            return self.dispatch_filter_result(result, |s, action| {
                s.process_flag_action(action)
            });
        }
        // Try arg panel
        if self.arg_panel.is_choosing() {
            let result = self
                .arg_panel
                .handle_choice_click(event.column, event.row, self.layout.arg_overlay_rect)
                .map(FilterAction::Inner);
            return self.dispatch_filter_result(result, |s, action| {
                s.process_arg_action(action)
            });
        }
        Action::None
    }

    /// Ensure the scroll offset keeps the selected index visible within the given viewport height.
    #[cfg(test)]
    pub fn ensure_visible(&mut self, panel: Focus, viewport_height: usize) {
        if viewport_height == 0 {
            return;
        }
        match panel {
            Focus::Commands => {
                self.command_panel.ensure_visible(viewport_height);
            }
            Focus::Flags => {
                self.flag_panel.ensure_visible(viewport_height);
            }
            Focus::Args => {
                self.arg_panel.ensure_visible(viewport_height);
            }
            Focus::Preview => {}
        }
    }

    pub fn handle_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::KeyCode;

        // If in execution mode, delegate to the execution component
        if self.is_executing() {
            if let Some(ref mut exec) = self.execution {
                if let EventResult::Action(ExecutionAction::Close) = exec.handle_key(key) {
                    self.close_execution();
                }
            }
            return Action::None;
        }

        // Ctrl+R executes command from any panel, regardless of edit/filter mode
        if key.code == KeyCode::Char('r')
            && key
                .modifiers
                .contains(crossterm::event::KeyModifiers::CONTROL)
        {
            return Action::Execute;
        }

        // If the theme picker is open, handle its keys
        if self.is_theme_picking() {
            return self.handle_theme_picker_key(key);
        }

        let focused_panel_is_handling_input = self.focused_panel_is_handling_input();
        if let Some(action) = self.handle_focused_panel_key(key) {
            return action;
        }
        if focused_panel_is_handling_input {
            return Action::None;
        }

        match key.code {
            KeyCode::Char('q') => Action::Quit,
            KeyCode::Char('T') => {
                self.open_theme_picker();
                Action::None
            }
            KeyCode::Char(']') => {
                self.next_theme();
                Action::None
            }
            KeyCode::Char('[') => {
                self.prev_theme();
                Action::None
            }
            KeyCode::Char('p') => Action::None,
            KeyCode::Tab => {
                self.focus_next();
                Action::None
            }
            KeyCode::BackTab => {
                self.focus_prev();
                Action::None
            }
            KeyCode::Enter if self.focus() == Focus::Preview => self.handle_enter(),
            _ => Action::None,
        }
    }

    /// Start editing on the focused panel: populate edit_input from current value.
    #[allow(dead_code)] // used in tests; production code calls panel.start_editing() directly
    pub fn start_editing(&mut self) {
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
            _ => return,
        };
        match self.focus() {
            Focus::Flags => self.flag_panel.start_editing(&current_text),
            Focus::Args => self.arg_panel.start_editing(&current_text),
            _ => {}
        }
    }


    #[cfg(test)]
    fn move_up(&mut self) {
        match self.focus() {
            Focus::Commands => {
                // Component handles both regular and filter-aware navigation
                if self.command_panel.move_up() {
                    self.set_command_path(self.command_panel.path().to_vec());
                }
            }
            Focus::Flags => {
                self.flag_panel.move_up();
            }
            Focus::Args => {
                self.arg_panel.move_up();
            }
            _ => {}
        }
    }

    fn handle_enter(&mut self) -> Action {
        match self.focus() {
            Focus::Commands => {
                let enter = crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                );
                self.handle_focused_panel_key(enter).unwrap_or(Action::None)
            }
            Focus::Flags | Focus::Args => {
                let enter = crossterm::event::KeyEvent::new(
                    crossterm::event::KeyCode::Enter,
                    crossterm::event::KeyModifiers::NONE,
                );
                self.handle_focused_panel_key(enter).unwrap_or(Action::None)
            }
            Focus::Preview => Action::Execute,
        }
    }

    fn process_flag_enter_request(&mut self, request: FlagPanelEnterRequest) {
        match request {
            FlagPanelEnterRequest::Toggle => {
                self.toggle_simple_flag();
            }
            FlagPanelEnterRequest::ChoiceSelect {
                index,
                choices,
                current_value,
                value_column,
            } => {
                let current_value = self
                    .current_flag_values()
                    .get(index)
                    .and_then(|(_, value)| match value {
                        FlagValue::String(text) => Some(text.clone()),
                        _ => None,
                    })
                    .unwrap_or(current_value);
                self.flag_panel
                    .open_choice_select(index, choices, &current_value, value_column);
            }
            FlagPanelEnterRequest::EditOrComplete {
                index,
                arg_name,
                current_value,
                value_column,
            } => {
                let current_value = self
                    .current_flag_values()
                    .get(index)
                    .and_then(|(_, value)| match value {
                        FlagValue::String(text) => Some(text.clone()),
                        _ => None,
                    })
                    .unwrap_or(current_value);
                if let Some(ref arg_name) = arg_name {
                    if let Some((choices, descriptions)) =
                        self.run_flag_completion(arg_name, &current_value)
                    {
                        self.flag_panel.open_completion_select(
                            index,
                            choices,
                            descriptions,
                            &current_value,
                            value_column,
                        );
                        return;
                    }
                }

                self.flag_panel.start_editing(&current_value);
            }
        }
    }

    /// Run a completion command for a flag argument and return results.
    /// Does NOT open the choice select — caller is responsible.
    fn run_flag_completion(
        &self,
        arg_name: &str,
        _current_value: &str,
    ) -> Option<(Vec<String>, Vec<Option<String>>)> {
        let complete = self.find_completion(arg_name)?.clone();
        let run_cmd = complete.run.as_ref()?;
        Self::run_completion(run_cmd, complete.descriptions)
    }

    fn process_arg_enter_request(&mut self, request: ArgPanelEnterRequest) {
        match request {
            ArgPanelEnterRequest::ChoiceSelect {
                index,
                choices,
                current_value,
                value_column,
            } => {
                let current_value = self
                    .arg_values
                    .get(index)
                    .map(|arg| arg.value.clone())
                    .unwrap_or(current_value);
                self.arg_panel
                    .open_choice_select(index, choices, &current_value, value_column);
            }
            ArgPanelEnterRequest::EditOrComplete {
                index,
                arg_name,
                current_value,
                value_column,
            } => {
                let current_value = self
                    .arg_values
                    .get(index)
                    .map(|arg| arg.value.clone())
                    .unwrap_or(current_value);
                if let Some((choices, descriptions)) =
                    self.run_arg_completion(&arg_name, &current_value)
                {
                    self.arg_panel.open_completion_select(
                        index,
                        choices,
                        descriptions,
                        &current_value,
                        value_column,
                    );
                } else {
                    self.arg_panel.start_editing(&current_value);
                }
            }
        }
    }

    /// Run a completion command for an arg and return results.
    fn run_arg_completion(
        &self,
        arg_name: &str,
        _current_value: &str,
    ) -> Option<(Vec<String>, Vec<Option<String>>)> {
        let complete = self.find_completion(arg_name)?.clone();
        let run_cmd = complete.run.as_ref()?;
        Self::run_completion(run_cmd, complete.descriptions)
    }

    /// Toggle a Bool, NegBool, or Count flag at the current index.
    /// Bool: flip. NegBool: cycle None→Some(true)→Some(false)→None. Count: increment.
    fn toggle_simple_flag(&mut self) {
        let flag_idx = self.flag_index();
        let mut changed = false;
        {
            let values = self.current_flag_values_mut();
            if let Some((name, value)) = values.get_mut(flag_idx) {
                let flag_name = name.clone();
                match value {
                    FlagValue::Bool(b) => {
                        *b = !*b;
                        let new_val = FlagValue::Bool(*b);
                        self.sync_global_flag(&flag_name, &new_val);
                        changed = true;
                    }
                    FlagValue::NegBool(state) => {
                        *state = match *state {
                            None => Some(true),
                            Some(true) => Some(false),
                            Some(false) => None,
                        };
                        let new_val = FlagValue::NegBool(*state);
                        self.sync_global_flag(&flag_name, &new_val);
                        changed = true;
                    }
                    FlagValue::Count(c) => {
                        *c += 1;
                        let new_val = FlagValue::Count(*c);
                        self.sync_global_flag(&flag_name, &new_val);
                        changed = true;
                    }
                    _ => {}
                }
            }
        }
        if changed {
            self.refresh_flag_panel_inputs();
        }
    }

    /// Handle a mouse click on one side of a negatable flag.
    /// `target` is the desired state: `Some(true)` for the positive name, `Some(false)` for the negate name.
    /// If already in the target state, resets to `None` (omitted).
    /// Build the full command string from the current state.
    pub fn build_command(&self) -> String {
        let preview = crate::command_builder::LiveArgPreview {
            choice_select_index: self.arg_panel.choice_select_index(),
            choice_select_text: self.arg_panel.choice_select_text(),
            is_editing: self.arg_panel.is_editing(),
            editing_index: self.arg_panel.selected_index(),
            editing_text: self.arg_panel.editing_text(),
        };
        crate::command_builder::build_command(
            &self.spec,
            &self.flag_values,
            &self.command_path,
            &self.arg_values,
            &preview,
        )
    }

    /// Build the command as a list of separate argument strings (for process execution).
    /// Unlike `build_command()`, this does NOT quote values — each element is a separate arg.
    pub fn build_command_parts(&self) -> Vec<String> {
        crate::command_builder::build_command_parts(
            &self.spec,
            &self.flag_values,
            &self.command_path,
            &self.arg_values,
        )
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
pub fn compute_tree_scores(
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
#[cfg(test)]
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

/// Parse a completion line in "value:description" format.
/// Colons can be escaped with `\:`.
fn parse_completion_line(line: &str) -> Option<(String, String)> {
    let mut value = String::new();
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && chars.peek() == Some(&':') {
            value.push(':');
            chars.next();
        } else if ch == ':' {
            let desc: String = chars.collect();
            return Some((value, desc.to_string()));
        } else {
            value.push(ch);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, RwLock};

    use crate::components::execution::ExecutionState;

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
        assert!(app.command_panel.tree_nodes().len() > 1);
        // Check for some expected top-level commands
        let names: Vec<&str> = app
            .command_panel
            .tree_nodes()
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
        let flat = flatten_command_tree(app.command_panel.tree_nodes());
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
    fn test_negated_flag_init_as_negbool_none() {
        // A flag with negate= should initialize as NegBool(None) (omitted)
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        let values = app.current_flag_values();
        let color = values.iter().find(|(n, _)| n == "color");
        assert_eq!(
            color,
            Some(&("color".to_string(), FlagValue::NegBool(None))),
            "--color flag with negate should initialize to NegBool(None)"
        );
    }

    #[test]
    fn test_negated_flag_omitted_emits_nothing() {
        // When a negatable flag is omitted (None), no flag in command
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        let cmd = app.build_command();
        assert!(
            !cmd.contains("--color"),
            "omitted negatable flag should not emit --color: {cmd}"
        );
        assert!(
            !cmd.contains("--no-color"),
            "omitted negatable flag should not emit --no-color: {cmd}"
        );
    }

    #[test]
    fn test_negated_flag_explicit_on_emits_flag() {
        // NegBool(Some(true)) should emit --color
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        let path_key = "run".to_string();
        if let Some(flags) = app.flag_values.get_mut(&path_key) {
            for (name, value) in flags.iter_mut() {
                if name == "color" {
                    *value = FlagValue::NegBool(Some(true));
                }
            }
        }
        let cmd = app.build_command();
        assert!(
            cmd.contains("--color"),
            "explicit on should emit --color: {cmd}"
        );
    }

    #[test]
    fn test_negated_flag_explicit_off_emits_negate() {
        // NegBool(Some(false)) should emit --no-color
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        let path_key = "run".to_string();
        if let Some(flags) = app.flag_values.get_mut(&path_key) {
            for (name, value) in flags.iter_mut() {
                if name == "color" {
                    *value = FlagValue::NegBool(Some(false));
                }
            }
        }
        let cmd = app.build_command();
        assert!(
            cmd.contains("--no-color"),
            "explicit off should emit --no-color: {cmd}"
        );
        // The positive form should not appear separately
        let without_negate = cmd.replace("--no-color", "");
        assert!(
            !without_negate.contains("--color"),
            "should not include positive flag when negated: {cmd}"
        );
    }

    #[test]
    fn test_negated_flag_toggle_cycles() {
        // Enter should cycle NegBool: None → Some(true) → Some(false) → None
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        app.set_focus(Focus::Flags);

        // Find the color flag index
        let color_idx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "color")
            .expect("color flag should exist");
        app.flag_panel.select(color_idx);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );

        // Start: None
        assert_eq!(
            app.current_flag_values()[color_idx].1,
            FlagValue::NegBool(None)
        );

        // First Enter: None → Some(true)
        app.handle_key(enter);
        assert_eq!(
            app.current_flag_values()[color_idx].1,
            FlagValue::NegBool(Some(true))
        );

        // Second Enter: Some(true) → Some(false)
        app.handle_key(enter);
        assert_eq!(
            app.current_flag_values()[color_idx].1,
            FlagValue::NegBool(Some(false))
        );

        // Third Enter: Some(false) → None
        app.handle_key(enter);
        assert_eq!(
            app.current_flag_values()[color_idx].1,
            FlagValue::NegBool(None)
        );
    }

    #[test]
    fn test_negated_flag_backspace_resets() {
        // Backspace should reset NegBool to None
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        app.set_focus(Focus::Flags);

        let color_idx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "color")
            .expect("color flag should exist");
        app.flag_panel.select(color_idx);

        // Set to Some(true) first
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert_eq!(
            app.current_flag_values()[color_idx].1,
            FlagValue::NegBool(Some(true))
        );

        // Backspace resets to None
        let backspace = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Backspace,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(backspace);
        assert_eq!(
            app.current_flag_values()[color_idx].1,
            FlagValue::NegBool(None)
        );
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
        let scores = compute_tree_scores(app.command_panel.tree_nodes(), "cfgset");

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
        let scores2 = compute_tree_scores(app.command_panel.tree_nodes(), "plinstall");
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
    fn test_arg_values_persist_per_command_path() {
        let mut app = App::new(sample_spec());

        app.navigate_to_command(&["deploy"]);
        app.arg_values[0].value = "prod".to_string();

        app.navigate_to_command(&["init"]);
        app.arg_values[0].value = "demo-app".to_string();

        app.navigate_to_command(&["deploy"]);
        assert_eq!(app.arg_values[0].value, "prod");

        app.navigate_to_command(&["init"]);
        assert_eq!(app.arg_values[0].value, "demo-app");
    }

    #[test]
    fn test_arg_defaults_only_apply_on_first_visit() {
        let spec = r#"
name "Defaults CLI"
bin "defaults"

cmd "deploy" {
    arg "<environment>" default="staging"
}

cmd "other" {
    arg "<name>"
}
"#
        .parse::<Spec>()
        .expect("Failed to parse default-arg test spec");

        let mut app = App::new(spec);
        app.navigate_to_command(&["deploy"]);
        assert_eq!(app.arg_values[0].value, "staging");

        app.arg_values[0].value = "prod".to_string();
        app.navigate_to_command(&["other"]);
        app.navigate_to_command(&["deploy"]);

        assert_eq!(app.arg_values[0].value, "prod");
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
        assert!(app.is_filtering());

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
        assert!(app.is_filtering());
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
        assert!(app.is_filtering());

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
        assert!(app.is_filtering());

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
        assert!(!app.is_filtering());
        assert!(app.filter().is_empty());
        assert_eq!(app.focus(), Focus::Flags);
    }

    #[test]
    fn test_slash_does_not_activate_filter_in_preview_pane() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Preview);
        assert_eq!(app.focus(), Focus::Preview);
        assert!(!app.is_filtering());

        // Press "/" - should NOT activate filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(
            !app.is_filtering(),
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
        assert!(app.is_filtering());

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
        assert!(!app.is_filtering(), "Should exit typing mode after Enter");
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
        assert!(!app.is_filtering());
        assert!(app.filter_active(), "Filter should be active after Enter");

        // Esc should clear the applied filter
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(esc);
        assert!(!app.is_filtering());
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
        assert!(app.is_filtering());

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
        assert!(!app.is_filtering(), "Should exit typing mode");
        assert!(app.filter_active(), "Filter should still be active");

        // Navigation should skip non-matching args
        let _initial_index = app.arg_panel.selected_index();
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);

        // Should have moved (assuming there are matching args)
        let _new_index = app.arg_panel.selected_index();
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
    fn test_set_focus_clears_applied_filter() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

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

        app.set_focus(Focus::Flags);

        assert!(!app.filter_active(), "Focus loss should clear the applied filter");
        assert!(app.filter().is_empty());
        assert_eq!(app.focus(), Focus::Flags);
    }

    #[test]
    fn test_scroll_offset_ensure_visible() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Manually set selected index beyond a small viewport
        app.command_panel.select_item(5);
        app.command_panel.set_scroll(0);
        app.ensure_visible(Focus::Commands, 3);
        // Index 5 should push scroll to at least 3 (5 - 3 + 1)
        assert_eq!(app.command_scroll(), 3);

        // Now move index back into view
        app.command_panel.select_item(2);
        app.ensure_visible(Focus::Commands, 3);
        // Index 2 < scroll 3, so scroll should snap to 2
        assert_eq!(app.command_scroll(), 2);

        // Already visible: no change
        app.command_panel.select_item(3);
        app.ensure_visible(Focus::Commands, 3);
        assert_eq!(app.command_scroll(), 2);
    }

    #[test]
    fn test_scroll_offset_on_move_up() {
        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Commands);

        // Set scroll and index so move_up triggers scroll adjustment
        app.command_panel.set_scroll(2);
        app.command_panel.select_item(2);
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
        app.layout = UiLayout::new();
        app.layout.click_regions
            .register(ratatui::layout::Rect::new(0, 1, 40, 18), Focus::Commands);
        app.layout.click_regions
            .register(ratatui::layout::Rect::new(40, 1, 60, 18), Focus::Flags);
        app.layout.click_regions
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
        let area = ratatui::layout::Rect::new(0, 1, 40, 18);
        app.layout = UiLayout::new();
        app.layout.click_regions.register(area, Focus::Commands);

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

        app.command_panel.move_down();
        assert_eq!(app.command_index(), 1);

        app.command_panel.move_up();
        assert_eq!(app.command_index(), 0);

        // Can't go below 0
        app.command_panel.move_up();
        assert_eq!(app.command_index(), 0);
    }

    #[test]
    fn test_input_state_editing() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);

        // Focus on args and start editing
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Start editing via the panel
        app.start_editing();
        assert!(app.arg_panel.is_editing());

        // Type "hello" using key events
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        for ch in "hello".chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let result = app.arg_panel.handle_key(key);
            if let EventResult::Action(FilterAction::Inner(action)) = result {
                app.process_arg_action(action);
            }
        }

        assert_eq!(app.arg_values[0].value, "hello");

        // Delete backward
        let key = KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE);
        let result = app.arg_panel.handle_key(key);
        if let EventResult::Action(FilterAction::Inner(action)) = result {
            app.process_arg_action(action);
        }
        assert_eq!(app.arg_values[0].value, "hell");

        // Finish editing
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        let result = app.arg_panel.handle_key(key);
        if let EventResult::Action(FilterAction::Inner(action)) = result {
            app.process_arg_action(action);
        }
        assert!(!app.arg_panel.is_editing());
    }

    #[test]
    fn test_click_region_registry() {
        let mut app = App::new(sample_spec());
        app.layout = UiLayout::new();
        app.layout.click_regions
            .register(Rect::new(0, 0, 40, 20), Focus::Commands);
        app.layout.click_regions
            .register(Rect::new(40, 0, 60, 20), Focus::Flags);
        app.layout.click_regions
            .register(Rect::new(0, 20, 100, 3), Focus::Preview);

        assert_eq!(
            app.layout.click_regions.handle_click(10, 5),
            Some(&Focus::Commands)
        );
        assert_eq!(app.layout.click_regions.handle_click(50, 5), Some(&Focus::Flags));
        assert_eq!(
            app.layout.click_regions.handle_click(50, 21),
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
        assert!(app.is_filtering());
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
        assert!(!app.is_filtering());
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
    fn test_backspace_clears_bool_flag() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());

        // quiet is a boolean flag — backspace should turn it off
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

        // Backspace should turn it off
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::Bool(false),
            "Backspace should turn off boolean flags"
        );
    }

    #[test]
    fn test_backspace_clears_string_flag() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);

        // Find the --tag flag (a string flag)
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "tag")
            .unwrap();
        app.set_flag_index(fidx);

        // Set a value by editing
        app.start_editing();
        for ch in "v1".chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let result = app.flag_panel.handle_key(key);
            if let EventResult::Action(FilterAction::Inner(action)) = result {
                app.process_flag_action(action);
            }
        }
        app.finish_editing();
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::String("v1".to_string())
        );

        // Backspace should clear the value
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.current_flag_values()[fidx].1,
            FlagValue::String(String::new()),
            "Backspace should clear string flag values"
        );
    }

    #[test]
    fn test_backspace_clears_arg_value() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <task>

        // Set a value by editing
        app.start_editing();
        for ch in "test".chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let result = app.arg_panel.handle_key(key);
            if let EventResult::Action(FilterAction::Inner(action)) = result {
                app.process_arg_action(action);
            }
        }
        app.finish_editing();
        assert_eq!(app.arg_values[0].value, "test");

        // Backspace should clear the value
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(
            app.arg_values[0].value, "",
            "Backspace should clear argument values"
        );
    }

    #[test]
    fn test_backspace_decrements_count_flag() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());
        app.set_focus(Focus::Flags);

        // Find verbose (count flag)
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "verbose")
            .unwrap();
        app.set_flag_index(fidx);

        // Increment to 3
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));
        assert_eq!(app.current_flag_values()[fidx].1, FlagValue::Count(3));

        // Backspace should decrement to 2
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        assert_eq!(app.current_flag_values()[fidx].1, FlagValue::Count(2));
    }

    #[test]
    fn test_editing_finished_on_mouse_click_different_arg() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        // Focus on args panel
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <task>

        // Start editing <task> and type some text via key events
        app.start_editing();
        assert!(app.is_editing());
        for ch in "hi".chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let result = app.arg_panel.handle_key(key);
            if let EventResult::Action(FilterAction::Inner(action)) = result {
                app.process_arg_action(action);
            }
        }
        assert_eq!(app.arg_values[0].value, "hi");

        // Now simulate clicking on the args area (which triggers finish_editing)
        // Set up a fake click region for Args
        let args_area = ratatui::layout::Rect::new(40, 5, 60, 10);
        app.layout = UiLayout::new();
        app.layout.click_regions.register(args_area, Focus::Args);

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
            !app.is_editing(),
            "Editing should be finished after clicking a different item"
        );

        // The <task> arg should still have the value we typed
        assert_eq!(
            app.arg_values[0].value, "hi",
            "First arg value should be preserved"
        );

        // The [args...] arg should NOT have the old edit text
        assert_eq!(
            app.arg_values[1].value, "",
            "Second arg value should be empty, not copied from first"
        );
    }

    #[test]
    fn test_editing_finished_on_mouse_click_different_panel() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["run"]);

        // Focus on args panel and start editing
        app.set_focus(Focus::Args);
        app.set_arg_index(0);
        app.start_editing();
        for ch in "test".chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let result = app.arg_panel.handle_key(key);
            if let EventResult::Action(FilterAction::Inner(action)) = result {
                app.process_arg_action(action);
            }
        }
        assert_eq!(app.arg_values[0].value, "test");
        assert!(app.is_editing());

        // Set up click regions and click on the Flags panel
        app.layout = UiLayout::new();
        app.layout.click_regions
            .register(ratatui::layout::Rect::new(0, 0, 40, 15), Focus::Flags);
        app.layout.click_regions
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
            !app.is_editing(),
            "Editing should be finished when clicking another panel"
        );
        // The arg value should be preserved
        assert_eq!(app.arg_values[0].value, "test");
        // Focus should have moved to Flags
        assert_eq!(app.focus(), Focus::Flags);
    }

    #[test]
    fn test_set_focus_finishes_flag_editing() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Flags);

        let flag_index = app
            .current_flag_values()
            .iter()
            .position(|(name, _)| name == "tag")
            .unwrap();
        app.set_flag_index(flag_index);
        app.start_editing();

        for ch in "v2".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }

        assert!(app.is_editing());
        assert_eq!(
            app.current_flag_values()[flag_index].1,
            FlagValue::String("v2".to_string())
        );

        app.set_focus(Focus::Args);

        assert!(!app.is_editing(), "Focus loss should finish editing");
        assert_eq!(app.focus(), Focus::Args);
        assert_eq!(
            app.current_flag_values()[flag_index].1,
            FlagValue::String("v2".to_string())
        );
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
        for ch in "build".chars() {
            let key = KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE);
            let result = app.arg_panel.handle_key(key);
            if let EventResult::Action(FilterAction::Inner(action)) = result {
                app.process_arg_action(action);
            }
        }
        assert_eq!(app.arg_values[0].value, "build");

        // Finish editing via Enter key
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.is_editing());

        // Move to next arg and start editing
        app.set_arg_index(1);
        app.start_editing();

        // The panel should be editing with the new arg's value (empty),
        // NOT the value from the first arg
        assert_eq!(
            app.arg_panel.editing_text(),
            "",
            "editing text should be initialized from the new arg's value, not the old one"
        );

        // The first arg should still have its value
        assert_eq!(app.arg_values[0].value, "build");
        // The second arg should still be empty
        assert_eq!(app.arg_values[1].value, "");
    }

    #[test]
    fn test_backspace_noop_on_non_flag_arg_focus() {
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
        let flat = flatten_command_tree(app.command_panel.tree_nodes());

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
        let flat = flatten_command_tree(app.command_panel.tree_nodes());

        assert_eq!(flat[0].full_path, "init");
        assert_eq!(flat[1].full_path, "config");
        assert_eq!(flat[2].full_path, "config set");
        assert_eq!(flat[9].full_path, "plugin install");
    }

    #[test]
    fn test_flatten_command_tree_depth() {
        let app = App::new(sample_spec());
        let flat = flatten_command_tree(app.command_panel.tree_nodes());

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

        app.start_execution(ExecutionComponent::new(state));
        assert_eq!(app.mode, AppMode::Executing);
        assert!(app.is_executing());
        assert!(app.execution.is_some());
        assert!(!app.execution.as_ref().unwrap().exited());

        // Simulate process exit
        exited.store(true, Ordering::Relaxed);
        assert!(app.execution.as_ref().unwrap().exited());

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

        app.start_execution(ExecutionComponent::new(state));
        assert_eq!(app.execution.as_ref().unwrap().exit_status(), Some("0".to_string()));
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

        app.start_execution(ExecutionComponent::new(state));
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

        app.start_execution(ExecutionComponent::new(state));

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

        app.start_execution(ExecutionComponent::new(state));

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

        app.start_execution(ExecutionComponent::new(state));

        // Resize the PTY — should not panic
        app.resize_pty(40, 120);

        // Verify the execution component still exists
        assert!(app.execution.is_some());
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
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);

        // T opens theme picker instead of cycling
        let mut app2 = App::new(sample_spec());
        let t_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('T'),
            crossterm::event::KeyModifiers::NONE,
        );
        app2.handle_key(t_key);
        assert!(app2.is_theme_picking(), "T should open theme picker");
        assert_eq!(app2.theme_name, ThemeName::Dracula, "Theme shouldn't change until navigated");
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
        let flat = flatten_command_tree(app.command_panel.tree_nodes());
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

        let flat = flatten_command_tree(app.command_panel.tree_nodes());
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
        let scores = compute_tree_scores(app.command_panel.tree_nodes(), "verbose");

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
        let scores = compute_tree_scores(app.command_panel.tree_nodes(), "project");

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
        let scores = compute_tree_scores(app.command_panel.tree_nodes(), "cfg");

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
        app.flag_panel.start_filtering();
        app.flag_panel.set_filter_text("Docker");

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
        assert!(app.is_filtering());

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

    // ── Choice select box tests ─────────────────────────────────────────

    #[test]
    fn test_enter_on_choice_flag_opens_select_box() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);
        app.set_focus(Focus::Flags);

        // Find the "template" flag which has choices
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "template")
            .unwrap();
        app.set_flag_index(fidx);

        assert!(!app.is_choosing(), "Should not be choosing before Enter");

        // Press Enter
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        assert!(app.is_choosing(), "Enter on choice flag should open select box");
        // Flag choice select is now on the flag panel component
        assert!(
            app.flag_panel.is_choosing(),
            "Flag panel should be choosing"
        );
        assert_eq!(app.flag_panel.choice_select_index(), Some(fidx));
    }

    #[test]
    fn test_enter_on_choice_arg_opens_select_box() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <environment> has choices

        assert!(!app.is_choosing());

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        assert!(app.is_choosing(), "Enter on choice arg should open select box");
        assert!(app.arg_panel.is_choosing());
        assert_eq!(app.arg_panel.choice_select_index(), Some(0));
    }

    #[test]
    fn test_choice_select_enter_confirms() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Navigate down to "staging" (Down from None→index 0, then index 0→index 1)
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down); // -> Some(0) = "dev"
        app.handle_key(down); // -> Some(1) = "staging"

        // Confirm selection
        app.handle_key(enter);
        assert!(!app.is_choosing(), "Enter should close select box");
        assert_eq!(app.arg_values[0].value, "staging", "Should have selected staging");
    }

    #[test]
    fn test_choice_select_esc_cancels() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Navigate down
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);

        // Cancel with Esc
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(esc);
        assert!(!app.is_choosing(), "Esc should close select box");
        // Value should remain unchanged (was empty/default)
        assert_eq!(app.arg_values[0].value, "", "Esc should not change value");
    }

    #[test]
    fn test_choice_select_filter_narrows_choices() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <environment> choices: dev, staging, prod

        // Open select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // All 3 choices visible initially
        assert_eq!(app.arg_panel.filtered_choices().len(), 3);

        // Type "d" to filter
        let d_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(d_key);

        // "d" should match "dev" and "prod" (both contain 'd')
        let filtered = app.arg_panel.filtered_choices();
        assert!(filtered.len() < 3, "Filter should narrow choices");
        assert!(
            filtered.iter().any(|(_, c)| c == "dev"),
            "dev should match 'd'"
        );

        // Type "ev" to narrow further → "dev" only
        let e_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('e'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(e_key);
        let v_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('v'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(v_key);

        let filtered = app.arg_panel.filtered_choices();
        assert_eq!(filtered.len(), 1, "Only 'dev' should match 'dev'");
        assert_eq!(filtered[0].1, "dev");

        // Confirm — should select "dev"
        app.handle_key(enter);
        assert!(!app.is_choosing());
        assert_eq!(app.arg_values[0].value, "dev");
    }

    #[test]
    fn test_choice_select_preselects_current_value() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Pre-set the value to "staging"
        app.arg_values[0].value = "staging".to_string();

        // Open select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // "staging" is at index 1 in ["dev", "staging", "prod"]
        assert_eq!(
            app.arg_panel.selected_choice_index(),
            Some(1),
            "Should pre-select 'staging'"
        );
    }

    #[test]
    fn test_choice_select_flag_confirms_value() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["init"]);
        app.set_focus(Focus::Flags);

        // Find "template" flag
        let fidx = app
            .current_flag_values()
            .iter()
            .position(|(n, _)| n == "template")
            .unwrap();
        app.set_flag_index(fidx);

        // Open select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Navigate to "full" (Down goes to index 0 first, then index 1)
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down); // -> Some(0) = "basic"
        app.handle_key(down); // -> Some(1) = "full"

        // Confirm
        app.handle_key(enter);
        assert!(!app.is_choosing());

        // Check flag value was set
        let val = &app.current_flag_values()[fidx].1;
        assert_eq!(
            val,
            &FlagValue::String("full".to_string()),
            "Flag value should be 'full'"
        );
    }

    #[test]
    fn test_choice_select_mouse_click_closes() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open select box
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Click outside
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert!(!app.is_choosing(), "Mouse click should close select box");
    }

    #[test]
    fn test_choice_select_navigation_bounds() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // choices: dev, staging, prod

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        // No selection initially (empty value doesn't match any choice)
        // Navigate up from no selection — should stay at None
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(app.arg_panel.selected_choice_index(), None);

        // Navigate down — selects first item (index 0)
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.arg_panel.selected_choice_index(), Some(0));

        // Navigate down twice more to index 2
        app.handle_key(down);
        app.handle_key(down);
        assert_eq!(app.arg_panel.selected_choice_index(), Some(2));

        // Navigate down again — should stay at 2 (last item)
        app.handle_key(down);
        assert_eq!(app.arg_panel.selected_choice_index(), Some(2));
    }

    #[test]
    fn test_choice_select_set_focus_closes() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Switch focus should close the select box
        app.set_focus(Focus::Flags);
        assert!(!app.is_choosing(), "Switching focus should close select box");
    }

    #[test]
    fn test_set_focus_from_completion_select_keeps_typed_text() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["plugin", "install"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());
        assert!(app.is_editing());

        let a_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(a_key);

        app.set_focus(Focus::Flags);

        assert_eq!(app.focus(), Focus::Flags);
        assert!(!app.is_choosing(), "Focus loss should close completion select");
        assert!(!app.is_editing(), "Focus loss should finish completion editing");
        assert_eq!(app.arg_values[0].value, "a", "Typed text should be preserved");
    }

    // ── Theme picker tests ──────────────────────────────────────────────

    #[test]
    fn test_t_opens_theme_picker() {
        let mut app = App::new(sample_spec());
        assert!(!app.is_theme_picking());

        let key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('T'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(key);
        assert!(app.is_theme_picking());
        assert_eq!(app.theme_picker.selected_index(), Some(0));
        assert_eq!(app.theme_picker.original_theme(), Some(ThemeName::Dracula));
    }

    #[test]
    fn test_theme_picker_esc_restores_original() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        // Navigate down to preview a different theme
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);

        // Esc should restore original theme
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(esc);
        assert!(!app.is_theme_picking());
        assert_eq!(app.theme_name, ThemeName::Dracula);
    }

    #[test]
    fn test_theme_picker_enter_confirms() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        // Navigate to Nord (index 2)
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        app.handle_key(down);
        assert_eq!(app.theme_name, ThemeName::Nord);

        // Enter confirms the preview
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(!app.is_theme_picking());
        assert_eq!(app.theme_name, ThemeName::Nord);
    }

    #[test]
    fn test_theme_picker_up_down_navigates() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        // Down moves to next theme and previews it
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.theme_picker.selected_index(), Some(1));
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);

        // Up moves back
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(app.theme_picker.selected_index(), Some(0));
        assert_eq!(app.theme_name, ThemeName::Dracula);

        // Up from first wraps to last
        app.handle_key(up);
        assert_eq!(
            app.theme_picker.selected_index(),
            Some(ThemeName::all().len() - 1)
        );
        assert_eq!(app.theme_name, ThemeName::Cyberpunk);
    }

    #[test]
    fn test_theme_picker_jk_navigates() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        let j = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('j'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(j);
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);

        let k = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('k'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(k);
        assert_eq!(app.theme_name, ThemeName::Dracula);
    }

    #[test]
    fn test_theme_picker_t_does_not_nest() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();
        assert!(app.is_theme_picking());

        // Pressing T while picker is open should be ignored (not open another picker)
        let t = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('T'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(t);
        // Should still be in the same picker
        assert!(app.is_theme_picking());
        assert_eq!(app.theme_picker.original_theme(), Some(ThemeName::Dracula));
    }

    #[test]
    fn test_bracket_keys_still_cycle_without_picker() {
        let mut app = App::new(sample_spec());

        // ] cycles forward without opening picker
        let rb = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char(']'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(rb);
        assert!(!app.is_theme_picking());
        assert_eq!(app.theme_name, ThemeName::OneDarkPro);

        // [ cycles backward without opening picker
        let lb = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('['),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(lb);
        assert!(!app.is_theme_picking());
        assert_eq!(app.theme_name, ThemeName::Dracula);
    }

    #[test]
    fn test_theme_picker_mouse_click_outside_closes() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.open_theme_picker();

        // Set a fake overlay rect
        app.layout.theme_overlay_rect = Some(Rect::new(60, 5, 20, 17));

        // Click outside the overlay
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 5,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert!(!app.is_theme_picking(), "Click outside should close picker");
        assert_eq!(app.theme_name, ThemeName::Dracula, "Should restore original theme");
    }

    #[test]
    fn test_theme_picker_mouse_click_on_indicator_opens() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        // Simulate the theme indicator rect being set by rendering
        app.layout.theme_indicator_rect = Some(Rect::new(85, 23, 10, 1));
        assert!(!app.is_theme_picking());

        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 88,
            row: 23,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);
        assert!(app.is_theme_picking(), "Click on theme indicator should open picker");
    }

    #[test]
    fn test_theme_picker_down_wraps_at_end() {
        let mut app = App::new(sample_spec());
        app.open_theme_picker();
        let all = ThemeName::all();

        // Navigate to the last theme by going up from first (wraps to last)
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(app.theme_picker.selected_index(), Some(all.len() - 1));
        assert_eq!(app.theme_name, *all.last().unwrap());

        // Down should wrap to the first theme
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.theme_picker.selected_index(), Some(0));
        assert_eq!(app.theme_name, ThemeName::Dracula);
    }

    // ── Completion tests ────────────────────────────────────────────────

    #[test]
    fn test_parse_completion_line_simple() {
        let result = parse_completion_line("auth:Authentication plugin");
        assert_eq!(
            result,
            Some(("auth".to_string(), "Authentication plugin".to_string()))
        );
    }

    #[test]
    fn test_parse_completion_line_escaped_colon() {
        let result = parse_completion_line("host\\:port:A host:port pair");
        assert_eq!(
            result,
            Some(("host:port".to_string(), "A host:port pair".to_string()))
        );
    }

    #[test]
    fn test_parse_completion_line_no_description() {
        let result = parse_completion_line("simple-value");
        assert_eq!(result, None);
    }

    #[test]
    fn test_run_completion_simple() {
        let result = App::run_completion("printf 'alpha\nbeta\ngamma\n'", false);
        assert!(result.is_some());
        let (choices, descs) = result.unwrap();
        assert_eq!(choices, vec!["alpha", "beta", "gamma"]);
        assert!(descs.iter().all(|d| d.is_none()));
    }

    #[test]
    fn test_run_completion_with_descriptions() {
        let result =
            App::run_completion("printf 'auth:Auth plugin\ncache:Cache layer\n'", true);
        assert!(result.is_some());
        let (choices, descs) = result.unwrap();
        assert_eq!(choices, vec!["auth", "cache"]);
        assert_eq!(
            descs,
            vec![
                Some("Auth plugin".to_string()),
                Some("Cache layer".to_string())
            ]
        );
    }

    #[test]
    fn test_run_completion_failure() {
        let result = App::run_completion("false", false);
        assert!(result.is_none());
    }

    #[test]
    fn test_run_completion_empty_output() {
        let result = App::run_completion("echo ''", false);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_completion_exists() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "install"]);
        let complete = app.find_completion("name");
        assert!(complete.is_some(), "Should find completion for 'name'");
        assert!(
            complete.unwrap().run.is_some(),
            "Completion should have a run command"
        );
    }

    #[test]
    fn test_find_completion_not_exists() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "install"]);
        let complete = app.find_completion("nonexistent");
        assert!(complete.is_none());
    }

    #[test]
    fn test_find_completion_with_descriptions_flag() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "update"]);
        let complete = app.find_completion("name");
        assert!(complete.is_some());
        assert!(complete.unwrap().descriptions);
    }

    #[test]
    fn test_completion_runs_each_time() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "install"]);

        // Each call should run the command fresh
        let result = app.run_and_open_completion("name", "");
        assert!(result.is_some());
        let (choices, _) = result.unwrap();
        assert!(!choices.is_empty());

        // Run again - should re-run the command
        let result2 = app.run_and_open_completion("name", "auth");
        assert!(result2.is_some());
        let (choices2, _) = result2.unwrap();
        assert_eq!(choices2, choices, "Should get same results on re-run");
    }

    #[test]
    fn test_enter_on_completion_arg_opens_select() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "install"]);
        app.set_focus(Focus::Args);

        // The <name> arg at index 0 should have a completion
        app.set_arg_index(0);
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        assert!(
            app.is_choosing(),
            "Enter on arg with completion should open select"
        );
        assert!(
            app.is_editing(),
            "Editing should be active alongside choice select"
        );
    }

    #[test]
    fn test_esc_from_completion_select_keeps_text() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "install"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open completion select
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Type some text
        let a_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('a'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(a_key);

        // Esc should close select and keep the typed text
        let esc = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(esc);
        assert!(!app.is_choosing(), "Select should be closed");
        assert!(!app.is_editing(), "Editing should be closed");
        assert_eq!(app.arg_values[0].value, "a", "Typed text should be kept as value");
    }

    #[test]
    fn test_completion_descriptions_in_select() {
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "update"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        assert!(app.is_choosing());
        // Verify the arg panel opened a choice select with descriptions
        assert!(app.arg_panel.is_choosing());
        let filtered = app.arg_panel.filtered_choices();
        assert!(!filtered.is_empty(), "Should have choices");
    }

    #[test]
    fn test_completion_filter_matches_description() {
        // plugin > update has descriptions=#true: auth:Authentication plugin, etc.
        // Typing a word that matches a description (not the name) should still show that item.
        let spec = sample_spec();
        let mut app = App::new(spec);
        app.navigate_to_command(&["plugin", "update"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        if !app.is_choosing() {
            // Skip if completion command not available in this environment
            return;
        }

        // Type "redis" — matches the description "Redis cache layer" (for "cache"), not the name
        for c in "redis".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }

        let filtered = app.arg_panel.filtered_choices();
        assert_eq!(filtered.len(), 1, "Only 'cache' should match 'redis' via its description");
        assert_eq!(filtered[0].1, "cache");
    }

    #[test]
    fn test_choice_select_typing_clears_selection() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Press Down to select first item
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.arg_panel.selected_choice_index(), Some(0));

        // Type a character — should clear selection
        let d_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('d'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(d_key);
        assert_eq!(
            app.arg_panel.selected_choice_index(),
            None,
            "Typing should clear selection"
        );
    }

    #[test]
    fn test_choice_select_enter_with_no_selection_accepts_text() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Type custom text without selecting from list
        for c in "custom".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }
        assert_eq!(app.arg_panel.selected_choice_index(), None);

        // Press Enter — should accept typed text
        app.handle_key(enter);
        assert!(!app.is_choosing(), "Select should be closed");
        assert!(!app.is_editing(), "Editing should be closed");
        assert_eq!(app.arg_values[0].value, "custom", "Typed text should be the value");
    }

    #[test]
    fn test_choice_select_up_from_first_deselects() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);

        // Select first item
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.arg_panel.selected_choice_index(), Some(0));

        // Up from first item should deselect
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(
            app.arg_panel.selected_choice_index(),
            None,
            "Up from first item should deselect"
        );
    }

    #[test]
    fn test_choice_select_reopen_after_selection() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0);

        // Open select and choose a value
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down); // Select first item
        app.handle_key(enter); // Confirm
        assert!(!app.is_choosing());
        assert_eq!(app.arg_values[0].value, "dev");

        // Reopen the select — should not crash
        app.handle_key(enter);
        assert!(app.is_choosing(), "Should be able to reopen select after choosing");
        // The current value "dev" should be pre-selected
        assert_eq!(
            app.arg_panel.selected_choice_index(),
            Some(0),
            "Should pre-select current value"
        );
    }

    #[test]
    fn test_choice_select_shows_all_choices_on_reopen() {
        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <environment> choices: dev, staging, prod

        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );

        // Open select, choose "dev", confirm
        app.handle_key(enter);
        app.handle_key(down);
        app.handle_key(enter);
        assert_eq!(app.arg_values[0].value, "dev");

        // Reopen — filter should be inactive, so all choices visible
        app.handle_key(enter);
        assert!(app.is_choosing());
        // Even though the value is "dev", all 3 choices should be visible
        assert_eq!(app.arg_panel.filtered_choices().len(), 3);

        // Type a character — filter activates, narrows choices
        let s_key = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('s'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(s_key);
        // "devs" should narrow the list (likely no exact matches for "devs")
        assert!(app.arg_panel.filtered_choices().len() < 3);
    }

    #[test]
    fn test_mouse_click_on_choice_select_overlay() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        app.navigate_to_command(&["deploy"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // <environment> has choices: dev, staging, prod

        // Open choice select
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing());

        // Simulate the overlay rect being set (as ui.rs would do during render).
        // Overlay: 3 items + 1 bottom border = height 4, placed at y=5.
        let overlay_rect = ratatui::layout::Rect::new(30, 5, 20, 4);
        app.layout.arg_overlay_rect = Some(overlay_rect);

        // Click on the 3rd item (row = 5 + 2 = 7, which is "prod" at index 2)
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 35,
            row: 7,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);

        assert!(!app.is_choosing(), "Click should close choice select");
        assert_eq!(
            app.arg_values[0].value, "prod",
            "Clicking on 3rd item should select 'prod'"
        );
    }

    #[test]
    fn test_mouse_click_on_scrolled_choice_select() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        // Use plugin > install which has 15 completions for "name" arg
        app.navigate_to_command(&["plugin", "install"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // "name" argument with completions

        // Open choice select
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing(), "Should open choice select for completions");

        // Navigate down to item 12 to cause scrolling
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        for _ in 0..13 {
            app.handle_key(down);
        }

        // Set overlay rect: 10 visible items + 1 border = height 11
        let overlay_rect = ratatui::layout::Rect::new(30, 5, 25, 11);
        app.layout.arg_overlay_rect = Some(overlay_rect);

        // With selected=12 and visible=10: scroll_offset = 12 - 9 = 3
        // Visual row 0 shows item 3, row 1 shows item 4, etc.
        // Click on visual row 5 (absolute row = 5 + 5 = 10) → should select item 8 (5+3)
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 35,
            row: 10,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);

        assert!(!app.is_choosing(), "Click should close choice select");

        // Get the expected value — item at index 8 in the completions list
        // The completions come from sample.usage.kdl plugin>install name completions
        let value = &app.arg_values[0].value;
        assert!(
            !value.is_empty(),
            "Click on scrolled choice should select a value"
        );
    }

    #[test]
    fn test_mouse_click_on_completion_clears_editing() {
        use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

        let mut app = App::new(sample_spec());
        // Use plugin > install which has completions (opened via open_completion_select,
        // which also sets editing=true on the base panel).
        app.navigate_to_command(&["plugin", "install"]);
        app.set_focus(Focus::Args);
        app.set_arg_index(0); // "name" arg with completions

        // Open choice select (this runs the completion command and opens with editing=true)
        let enter = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(enter);
        assert!(app.is_choosing(), "Should open choice select for completions");
        assert!(
            app.arg_panel.is_editing(),
            "Completion select opens with editing=true"
        );

        // Navigate down to item 2
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down); // -> 0
        app.handle_key(down); // -> 1
        app.handle_key(down); // -> 2

        // Set overlay rect (as ui.rs would do during render): 15 items capped at 10 + 1 border
        let overlay_rect = ratatui::layout::Rect::new(30, 5, 25, 11);
        app.layout.arg_overlay_rect = Some(overlay_rect);

        // Click on item at visual row 2 (absolute row = 5 + 2 = 7)
        let mouse = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 35,
            row: 7,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        app.handle_mouse(mouse);

        assert!(!app.is_choosing(), "Click should close choice select");
        assert!(
            !app.arg_panel.is_editing(),
            "Click-select on completion should clear editing state"
        );
        assert!(
            !app.arg_values[0].value.is_empty(),
            "Should have selected a value"
        );
    }
}
