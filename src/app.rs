use usage::{Spec, SpecCommand, SpecFlag};

/// Actions that the event loop should take after handling a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    None,
    Quit,
    Accept,
}

/// Which panel currently has focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Commands,
    Flags,
    Args,
    Preview,
}

/// Tracks the value set for a flag.
#[derive(Debug, Clone)]
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

/// Main application state.
pub struct App {
    pub spec: Spec,

    /// Breadcrumb path of subcommand names the user has navigated into.
    /// Empty means we're at the root command.
    pub command_path: Vec<String>,

    /// Index of the currently highlighted subcommand in the subcommand list.
    pub command_index: usize,

    /// Flag values keyed by flag name, per command path depth.
    /// The key is the full command path joined by space.
    pub flag_values: std::collections::HashMap<String, Vec<(String, FlagValue)>>,

    /// Arg values for the current command.
    pub arg_values: Vec<ArgValue>,

    /// Index of the currently highlighted flag.
    pub flag_index: usize,

    /// Index of the currently highlighted arg.
    pub arg_index: usize,

    /// Which panel has focus.
    pub focus: Focus,

    /// Whether we are currently editing a text field (flag value or arg value).
    pub editing: bool,

    /// Filter text for fzf-style matching in the currently focused list.
    pub filter: String,

    /// Whether the filter input is active.
    pub filtering: bool,

    /// Scroll offset for the command list.
    pub command_scroll: usize,

    /// Scroll offset for the flag list.
    pub flag_scroll: usize,
}

impl App {
    pub fn new(spec: Spec) -> Self {
        let mut app = Self {
            spec,
            command_path: Vec::new(),
            command_index: 0,
            flag_values: std::collections::HashMap::new(),
            arg_values: Vec::new(),
            flag_index: 0,
            arg_index: 0,
            focus: Focus::Commands,
            editing: false,
            filter: String::new(),
            filtering: false,
            command_scroll: 0,
            flag_scroll: 0,
        };
        app.sync_state();
        app
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

    /// Returns the visible (non-hidden) subcommands of the current command,
    /// optionally filtered by the current filter string.
    pub fn visible_subcommands(&self) -> Vec<(&String, &SpecCommand)> {
        let cmd = self.current_command();
        let items: Vec<(&String, &SpecCommand)> =
            cmd.subcommands.iter().filter(|(_, c)| !c.hide).collect();

        if self.filtering && !self.filter.is_empty() {
            let filter_lower = self.filter.to_lowercase();
            items
                .into_iter()
                .filter(|(name, c)| {
                    fuzzy_match(&name.to_lowercase(), &filter_lower)
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
    /// including global flags from ancestors.
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

        // Fix up focus if current panel has no items
        self.adjust_focus();
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

    fn adjust_focus(&mut self) {
        let has_commands = !self.visible_subcommands().is_empty();
        let has_flags = !self.visible_flags().is_empty();
        let has_args = !self.arg_values.is_empty();

        match self.focus {
            Focus::Commands if !has_commands => {
                if has_flags {
                    self.focus = Focus::Flags;
                } else if has_args {
                    self.focus = Focus::Args;
                } else {
                    self.focus = Focus::Preview;
                }
            }
            Focus::Flags if !has_flags => {
                if has_commands {
                    self.focus = Focus::Commands;
                } else if has_args {
                    self.focus = Focus::Args;
                } else {
                    self.focus = Focus::Preview;
                }
            }
            Focus::Args if !has_args => {
                if has_commands {
                    self.focus = Focus::Commands;
                } else if has_flags {
                    self.focus = Focus::Flags;
                } else {
                    self.focus = Focus::Preview;
                }
            }
            _ => {}
        }
    }

    /// Handle a key event, returning the action the event loop should take.
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
            KeyCode::Char('/') => {
                self.filtering = true;
                self.filter.clear();
                Action::None
            }
            KeyCode::Tab => {
                self.cycle_focus_forward();
                Action::None
            }
            KeyCode::BackTab => {
                self.cycle_focus_backward();
                Action::None
            }
            KeyCode::Esc => {
                if !self.command_path.is_empty() {
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
                if !self.command_path.is_empty() {
                    self.navigate_up();
                }
                Action::None
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if self.focus == Focus::Commands {
                    self.navigate_into_selected();
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
                self.editing = false;
                Action::None
            }
            KeyCode::Enter => {
                self.editing = false;
                Action::None
            }
            KeyCode::Backspace => {
                let flag_idx = self.flag_index;
                let arg_idx = self.arg_index;
                match self.focus {
                    Focus::Flags => {
                        let values = self.current_flag_values_mut();
                        if let Some((_, FlagValue::String(ref mut s))) = values.get_mut(flag_idx) {
                            s.pop();
                        }
                    }
                    Focus::Args => {
                        if let Some(arg) = self.arg_values.get_mut(arg_idx) {
                            arg.value.pop();
                        }
                    }
                    _ => {}
                }
                Action::None
            }
            KeyCode::Char(c) => {
                let flag_idx = self.flag_index;
                let arg_idx = self.arg_index;
                match self.focus {
                    Focus::Flags => {
                        let values = self.current_flag_values_mut();
                        if let Some((_, FlagValue::String(ref mut s))) = values.get_mut(flag_idx) {
                            s.push(c);
                        }
                    }
                    Focus::Args => {
                        if let Some(arg) = self.arg_values.get_mut(arg_idx) {
                            arg.value.push(c);
                        }
                    }
                    _ => {}
                }
                Action::None
            }
            _ => Action::None,
        }
    }

    fn handle_filter_key(&mut self, key: crossterm::event::KeyEvent) -> Action {
        use crossterm::event::KeyCode;

        match key.code {
            KeyCode::Esc => {
                self.filtering = false;
                self.filter.clear();
                Action::None
            }
            KeyCode::Enter => {
                self.filtering = false;
                // Keep the filter active to show filtered results
                Action::None
            }
            KeyCode::Backspace => {
                self.filter.pop();
                // Reset selection index when filter changes
                self.command_index = 0;
                Action::None
            }
            KeyCode::Char(c) => {
                self.filter.push(c);
                self.command_index = 0;
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

    fn cycle_focus_forward(&mut self) {
        let panels = self.available_panels();
        if panels.is_empty() {
            return;
        }
        let current_idx = panels.iter().position(|&p| p == self.focus).unwrap_or(0);
        let next_idx = (current_idx + 1) % panels.len();
        self.focus = panels[next_idx];
    }

    fn cycle_focus_backward(&mut self) {
        let panels = self.available_panels();
        if panels.is_empty() {
            return;
        }
        let current_idx = panels.iter().position(|&p| p == self.focus).unwrap_or(0);
        let next_idx = if current_idx == 0 {
            panels.len() - 1
        } else {
            current_idx - 1
        };
        self.focus = panels[next_idx];
    }

    fn available_panels(&self) -> Vec<Focus> {
        let mut panels = Vec::new();
        if !self.visible_subcommands().is_empty() {
            panels.push(Focus::Commands);
        }
        if !self.visible_flags().is_empty() {
            panels.push(Focus::Flags);
        }
        if !self.arg_values.is_empty() {
            panels.push(Focus::Args);
        }
        panels.push(Focus::Preview);
        panels
    }

    fn move_up(&mut self) {
        match self.focus {
            Focus::Commands => {
                if self.command_index > 0 {
                    self.command_index -= 1;
                }
            }
            Focus::Flags => {
                if self.flag_index > 0 {
                    self.flag_index -= 1;
                }
            }
            Focus::Args => {
                if self.arg_index > 0 {
                    self.arg_index -= 1;
                }
            }
            Focus::Preview => {}
        }
    }

    fn move_down(&mut self) {
        match self.focus {
            Focus::Commands => {
                let len = self.visible_subcommands().len();
                if len > 0 && self.command_index < len - 1 {
                    self.command_index += 1;
                }
            }
            Focus::Flags => {
                let len = self.current_flag_values().len();
                if len > 0 && self.flag_index < len - 1 {
                    self.flag_index += 1;
                }
            }
            Focus::Args => {
                let len = self.arg_values.len();
                if len > 0 && self.arg_index < len - 1 {
                    self.arg_index += 1;
                }
            }
            Focus::Preview => {}
        }
    }

    fn handle_enter(&mut self) -> Action {
        match self.focus {
            Focus::Commands => {
                self.navigate_into_selected();
                Action::None
            }
            Focus::Flags => {
                let flag_idx = self.flag_index;

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
                                self.editing = true;
                            }
                        }
                    }
                }
                Action::None
            }
            Focus::Args => {
                let arg = &self.arg_values[self.arg_index];
                if !arg.choices.is_empty() {
                    // Cycle through choices
                    let current = arg.value.clone();
                    let choices = arg.choices.clone();
                    let idx = choices
                        .iter()
                        .position(|c| c == &current)
                        .map(|i| (i + 1) % choices.len())
                        .unwrap_or(0);
                    self.arg_values[self.arg_index].value = choices[idx].clone();
                } else {
                    self.editing = true;
                }
                Action::None
            }
            Focus::Preview => Action::Accept,
        }
    }

    fn handle_space(&mut self) {
        match self.focus {
            Focus::Flags => {
                let flag_idx = self.flag_index;
                let values = self.current_flag_values_mut();
                if let Some((_, value)) = values.get_mut(flag_idx) {
                    match value {
                        FlagValue::Bool(b) => *b = !*b,
                        FlagValue::Count(c) => *c += 1,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    pub fn navigate_into_selected(&mut self) {
        let subs = self.visible_subcommands();
        if let Some((name, _)) = subs.get(self.command_index) {
            let name = (*name).clone();
            self.command_path.push(name);
            self.command_index = 0;
            self.flag_index = 0;
            self.arg_index = 0;
            self.filter.clear();
            self.filtering = false;
            self.sync_state();
        }
    }

    pub fn navigate_up(&mut self) {
        if !self.command_path.is_empty() {
            self.command_path.pop();
            self.command_index = 0;
            self.flag_index = 0;
            self.arg_index = 0;
            self.filter.clear();
            self.filtering = false;
            self.sync_state();
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
        match self.focus {
            Focus::Commands => {
                let subs = self.visible_subcommands();
                subs.get(self.command_index)
                    .and_then(|(_, cmd)| cmd.help.clone())
            }
            Focus::Flags => {
                let flags = self.visible_flags();
                flags.get(self.flag_index).and_then(|f| f.help.clone())
            }
            Focus::Args => self.arg_values.get(self.arg_index).and_then(|_| {
                let cmd = self.current_command();
                cmd.args
                    .iter()
                    .filter(|a| !a.hide)
                    .nth(self.arg_index)
                    .and_then(|a| a.help.clone())
            }),
            Focus::Preview => Some("Press Enter to accept the command, Esc to go back".to_string()),
        }
    }

    /// Returns the display title for the current command context.
    pub fn breadcrumb(&self) -> String {
        let mut parts = vec![if self.spec.bin.is_empty() {
            self.spec.name.clone()
        } else {
            self.spec.bin.clone()
        }];
        parts.extend(self.command_path.clone());
        parts.join(" > ")
    }
}

/// Simple fuzzy matching: checks if all characters in the pattern appear in order in the text.
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
        assert_eq!(app.focus, Focus::Commands);
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
    fn test_navigate_into_subcommand() {
        let mut app = App::new(sample_spec());
        // Navigate into "config"
        // Find the index of "config"
        let subs = app.visible_subcommands();
        let config_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.command_index = config_idx;
        app.navigate_into_selected();

        assert_eq!(app.command_path, vec!["config"]);
        let subs = app.visible_subcommands();
        let names: Vec<&str> = subs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"set"));
        assert!(names.contains(&"get"));
        assert!(names.contains(&"list"));
        assert!(names.contains(&"remove"));
    }

    #[test]
    fn test_navigate_up() {
        let mut app = App::new(sample_spec());
        let subs = app.visible_subcommands();
        let config_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.command_index = config_idx;
        app.navigate_into_selected();
        assert_eq!(app.command_path, vec!["config"]);

        app.navigate_up();
        assert!(app.command_path.is_empty());
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
        let subs = app.visible_subcommands();
        let init_idx = subs.iter().position(|(n, _)| n.as_str() == "init").unwrap();
        app.command_index = init_idx;
        app.navigate_into_selected();

        let cmd = app.build_command();
        assert!(cmd.starts_with("mycli init"));
    }

    #[test]
    fn test_build_command_with_flags_and_args() {
        let mut app = App::new(sample_spec());

        // Navigate to "init"
        let subs = app.visible_subcommands();
        let init_idx = subs.iter().position(|(n, _)| n.as_str() == "init").unwrap();
        app.command_index = init_idx;
        app.navigate_into_selected();

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
    fn test_breadcrumb() {
        let mut app = App::new(sample_spec());
        assert_eq!(app.breadcrumb(), "mycli");

        let subs = app.visible_subcommands();
        let config_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "config")
            .unwrap();
        app.command_index = config_idx;
        app.navigate_into_selected();
        assert_eq!(app.breadcrumb(), "mycli > config");

        let subs = app.visible_subcommands();
        let set_idx = subs.iter().position(|(n, _)| n.as_str() == "set").unwrap();
        app.command_index = set_idx;
        app.navigate_into_selected();
        assert_eq!(app.breadcrumb(), "mycli > config > set");
    }

    #[test]
    fn test_current_help() {
        let mut app = App::new(sample_spec());
        app.focus = Focus::Commands;
        app.command_index = 0;

        // Should return help for the first subcommand
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

        // Navigate to "init" which has a required arg <name>
        let subs = app.visible_subcommands();
        let init_idx = subs.iter().position(|(n, _)| n.as_str() == "init").unwrap();
        app.command_index = init_idx;
        app.navigate_into_selected();

        assert!(!app.arg_values.is_empty());
        assert_eq!(app.arg_values[0].name, "name");
        assert!(app.arg_values[0].required);
    }

    #[test]
    fn test_deploy_has_choices() {
        let mut app = App::new(sample_spec());

        let subs = app.visible_subcommands();
        let deploy_idx = subs
            .iter()
            .position(|(n, _)| n.as_str() == "deploy")
            .unwrap();
        app.command_index = deploy_idx;
        app.navigate_into_selected();

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

        // Navigate to "run" which has --jobs with default "4"
        let subs = app.visible_subcommands();
        let run_idx = subs.iter().position(|(n, _)| n.as_str() == "run").unwrap();
        app.command_index = run_idx;
        app.navigate_into_selected();

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

        // Move down
        let down = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(down);
        assert_eq!(app.command_index, 1);

        // Move up
        let up = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Up,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(up);
        assert_eq!(app.command_index, 0);
    }

    #[test]
    fn test_tab_cycles_focus() {
        let mut app = App::new(sample_spec());
        assert_eq!(app.focus, Focus::Commands);

        let tab = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(tab);
        assert_eq!(app.focus, Focus::Flags);

        let tab = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(tab);
        // Root has no args, so should skip to Preview
        assert_eq!(app.focus, Focus::Preview);
    }

    #[test]
    fn test_filter_mode() {
        let mut app = App::new(sample_spec());

        // Enter filter mode
        let slash = crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('/'),
            crossterm::event::KeyModifiers::NONE,
        );
        app.handle_key(slash);
        assert!(app.filtering);

        // Type "cfg"
        for c in "cfg".chars() {
            let key = crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(c),
                crossterm::event::KeyModifiers::NONE,
            );
            app.handle_key(key);
        }
        assert_eq!(app.filter, "cfg");

        // Should filter subcommands - "config" should match "cfg"
        let subs = app.visible_subcommands();
        let names: Vec<&str> = subs.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"config"));
        // "init" should not match "cfg"
        assert!(!names.contains(&"init"));
    }
}
