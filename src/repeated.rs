use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::app::{App, FlagValue, Focus, RepeatInput};
use crate::fields::field_id;

impl App {
    pub(crate) fn repeated_flag_id(&self, index: usize) -> Option<String> {
        let flag = self.visible_flags().get(index).copied()?;
        Some(field_id(
            &self.command_path,
            "flags",
            &flag.name,
            flag.global,
        ))
    }

    pub fn flag_repeat_position(&self, index: usize) -> (usize, usize) {
        self.repeated_flag_id(index)
            .and_then(|id| self.flag_repeat_positions.get(&id).copied())
            .unwrap_or((0, 0))
    }

    pub fn flag_repeatable(&self, index: usize) -> bool {
        self.visible_flags().get(index).is_some_and(|flag| {
            flag.arg.is_some() && (flag.var || flag.arg.as_ref().is_some_and(|arg| arg.var))
        })
    }

    pub fn flag_has_repeated_occurrences(&self, index: usize) -> bool {
        self.visible_flags()
            .get(index)
            .is_some_and(|flag| flag.var && flag.arg.is_some())
    }

    pub fn flag_has_repeated_values(&self, index: usize) -> bool {
        self.visible_flags()
            .get(index)
            .is_some_and(|flag| flag.arg.as_ref().is_some_and(|arg| arg.var))
    }

    pub fn current_repeated_flag_value(&self, index: usize) -> Option<String> {
        self.current_repeated_flag_input(index)
            .map(|input| input.value)
    }

    pub fn current_repeated_flag_input(&self, index: usize) -> Option<RepeatInput> {
        let (group_index, value_index) = self.flag_repeat_position(index);
        let (_, value) = self.current_flag_values().get(index)?;
        match value {
            FlagValue::Repeated(groups) => groups
                .get(group_index)
                .and_then(|group| group.get(value_index))
                .cloned(),
            FlagValue::String(value) => Some(if value.is_empty() {
                RepeatInput::omitted()
            } else {
                RepeatInput::supplied(value.clone())
            }),
            FlagValue::EmptyString => Some(RepeatInput::supplied(String::new())),
            _ => None,
        }
    }

    fn set_flag_repeat_position(&mut self, index: usize, position: (usize, usize)) {
        if let Some(id) = self.repeated_flag_id(index) {
            self.flag_repeat_positions.insert(id, position);
            self.invalidate_repeated_completion();
            self.refresh_flag_panel_inputs();
        }
    }

    fn invalidate_repeated_completion(&mut self) {
        self.completion_generation += 1;
        self.pending_completion = None;
    }

    pub fn move_flag_occurrence(&mut self, forward: bool) {
        if self.focus() != Focus::Flags {
            return;
        }
        let index = self.flag_index();
        let Some(flag) = self.visible_flags().get(index).copied() else {
            return;
        };
        if !flag.var {
            return;
        }
        if flag.arg.is_none() {
            return;
        }
        let count = self
            .current_flag_values()
            .get(index)
            .and_then(|(_, value)| match value {
                FlagValue::Repeated(groups) => Some(groups.len()),
                _ => None,
            })
            .unwrap_or(0);
        if count == 0 {
            return;
        }
        let (group, value) = self.flag_repeat_position(index);
        let group = if forward {
            (group + 1).min(count - 1)
        } else {
            group.saturating_sub(1)
        };
        let value_count = self
            .current_flag_values()
            .get(index)
            .and_then(|(_, value)| match value {
                FlagValue::Repeated(groups) => groups.get(group).map(Vec::len),
                _ => None,
            })
            .unwrap_or(0);
        self.set_flag_repeat_position(index, (group, value.min(value_count.saturating_sub(1))));
    }

    pub fn move_flag_value(&mut self, forward: bool) {
        if self.focus() != Focus::Flags {
            return;
        }
        let index = self.flag_index();
        if !self
            .visible_flags()
            .get(index)
            .is_some_and(|flag| flag.arg.as_ref().is_some_and(|arg| arg.var))
        {
            return;
        }
        let (group, value) = self.flag_repeat_position(index);
        let count = self
            .current_flag_values()
            .get(index)
            .and_then(|(_, value)| match value {
                FlagValue::Repeated(groups) => groups.get(group).map(Vec::len),
                _ => None,
            })
            .unwrap_or(0);
        if count == 0 {
            return;
        }
        let value = if forward {
            (value + 1).min(count - 1)
        } else {
            value.saturating_sub(1)
        };
        self.set_flag_repeat_position(index, (group, value));
    }

    pub fn change_flag_occurrence(&mut self, add: bool) {
        if self.focus() != Focus::Flags {
            return;
        }
        let index = self.flag_index();
        if self.flag_locked(index)
            || !self
                .visible_flags()
                .get(index)
                .is_some_and(|flag| flag.var && flag.arg.is_some())
        {
            return;
        }
        let Some(id) = self.repeated_flag_id(index) else {
            return;
        };
        let Some((name, current)) = self.current_flag_values().get(index).cloned() else {
            return;
        };
        let FlagValue::Repeated(mut groups) = current else {
            return;
        };
        let (mut group, _) = self
            .flag_repeat_positions
            .get(&id)
            .copied()
            .unwrap_or((0, 0));
        if add {
            let insert_at = (group + 1).min(groups.len());
            groups.insert(insert_at, vec![RepeatInput::omitted()]);
            group = insert_at;
        } else if group < groups.len() {
            groups.remove(group);
            group = group.min(groups.len().saturating_sub(1));
        }
        let updated = FlagValue::Repeated(groups);
        if let Some((_, stored)) = self.current_flag_values_mut().get_mut(index) {
            *stored = updated.clone();
        }
        self.sync_global_flag(&name, &updated);
        self.flag_repeat_positions.insert(id, (group, 0));
        self.invalidate_repeated_completion();
        self.refresh_flag_panel_inputs();
    }

    pub fn change_repeated_value(&mut self, add: bool) {
        match self.focus() {
            Focus::Args => self.change_repeated_arg(add),
            Focus::Flags => self.change_repeated_flag_value(add),
            _ => {}
        }
    }

    fn change_repeated_arg(&mut self, add: bool) {
        let index = self.arg_index();
        if self.arg_locked(index) {
            return;
        }
        let Some(arg) = self.arg_values.get(index).cloned() else {
            return;
        };
        if !self
            .current_command()
            .args
            .iter()
            .any(|spec| spec.name == arg.name && spec.var)
        {
            return;
        }
        if add {
            self.arg_values.insert(
                index + 1,
                crate::app::ArgValue {
                    value: String::new(),
                    supplied: false,
                    ..arg
                },
            );
            self.arg_panel.set_total(self.arg_values.len());
            self.arg_panel.select(index + 1);
        } else {
            let count = self
                .arg_values
                .iter()
                .filter(|value| value.name == arg.name)
                .count();
            if count > 1 {
                self.arg_values.remove(index);
                self.arg_panel.set_total(self.arg_values.len());
                self.arg_panel
                    .select(index.min(self.arg_values.len().saturating_sub(1)));
            } else if let Some(value) = self.arg_values.get_mut(index) {
                value.value.clear();
                value.supplied = false;
            }
        }
        self.persist_current_arg_values();
        self.refresh_arg_panel_inputs();
        self.completion_generation += 1;
        self.pending_completion = None;
    }

    fn change_repeated_flag_value(&mut self, add: bool) {
        let index = self.flag_index();
        if self.flag_locked(index) {
            return;
        }
        let Some(flag) = self.visible_flags().get(index).copied() else {
            return;
        };
        let values_per_occurrence = flag.arg.as_ref().is_some_and(|arg| arg.var);
        if !values_per_occurrence {
            return;
        }
        let Some(id) = self.repeated_flag_id(index) else {
            return;
        };
        let Some((name, current)) = self.current_flag_values().get(index).cloned() else {
            return;
        };
        let mut groups = match current {
            FlagValue::Repeated(groups) => groups,
            FlagValue::String(value) if !value.is_empty() => {
                vec![vec![RepeatInput::supplied(value)]]
            }
            FlagValue::EmptyString => vec![vec![RepeatInput::supplied("")]],
            FlagValue::String(_) => Vec::new(),
            _ => return,
        };
        let (mut group, mut value) = self
            .flag_repeat_positions
            .get(&id)
            .copied()
            .unwrap_or((0, 0));

        if flag.var {
            if add {
                if groups.is_empty() {
                    groups.push(vec![RepeatInput::omitted()]);
                    value = 0;
                } else {
                    let values = groups.get_mut(group).unwrap();
                    values.insert(value + 1, RepeatInput::omitted());
                    value += 1;
                }
            } else if let Some(values) = groups.get_mut(group) {
                if value < values.len() {
                    values.remove(value);
                    if values.is_empty() {
                        groups.remove(group);
                        group = group.min(groups.len().saturating_sub(1));
                        value = 0;
                    } else {
                        value = value.min(values.len() - 1);
                    }
                }
            }
        } else {
            if groups.is_empty() {
                if add {
                    groups.push(vec![RepeatInput::omitted()]);
                    value = 0;
                }
            } else if let Some(values) = groups.get_mut(0) {
                if add {
                    values.insert(value + 1, RepeatInput::omitted());
                    value += 1;
                } else if value < values.len() {
                    values.remove(value);
                    value = value.min(values.len().saturating_sub(1));
                }
            }
            group = 0;
        }

        let updated = FlagValue::Repeated(groups);
        if let Some((_, stored)) = self.current_flag_values_mut().get_mut(index) {
            *stored = updated.clone();
        }
        self.sync_global_flag(&name, &updated);
        self.flag_repeat_positions.insert(id, (group, value));
        self.invalidate_repeated_completion();
        self.refresh_flag_panel_inputs();
    }

    pub fn handle_repeated_key(&mut self, key: KeyEvent) -> bool {
        if self.focused_panel_is_handling_input() {
            return false;
        }
        match key.code {
            KeyCode::Char('n') if key.modifiers == KeyModifiers::CONTROL => {
                self.change_repeated_value(true);
                true
            }
            KeyCode::Char('d') if key.modifiers == KeyModifiers::CONTROL => {
                self.change_repeated_value(false);
                true
            }
            KeyCode::Char('n') if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.change_flag_occurrence(true);
                true
            }
            KeyCode::Char('d') if key.modifiers == (KeyModifiers::CONTROL | KeyModifiers::ALT) => {
                self.change_flag_occurrence(false);
                true
            }
            KeyCode::Right if key.modifiers == KeyModifiers::ALT => {
                self.move_flag_occurrence(true);
                true
            }
            KeyCode::Left if key.modifiers == KeyModifiers::ALT => {
                self.move_flag_occurrence(false);
                true
            }
            KeyCode::Right if key.modifiers == KeyModifiers::CONTROL => {
                self.move_flag_value(true);
                true
            }
            KeyCode::Left if key.modifiers == KeyModifiers::CONTROL => {
                self.move_flag_value(false);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_navigation_and_removal_preserve_each_occurrence() {
        let spec: usage::Spec = r#"
            name "demo"
            flag "--group... <item>..." var=#true {
                arg "<item>..." var=#true
            }
        "#
        .parse()
        .unwrap();
        let mut app = App::new(spec);
        app.current_flag_values_mut()[0].1 = FlagValue::Repeated(vec![
            vec![RepeatInput::supplied("a")],
            vec![RepeatInput::supplied("b"), RepeatInput::supplied("c")],
        ]);
        app.set_focus(Focus::Flags);

        let generation = app.completion_generation;
        assert!(app.handle_repeated_key(KeyEvent::new(KeyCode::Right, KeyModifiers::ALT)));
        assert!(app.handle_repeated_key(KeyEvent::new(KeyCode::Right, KeyModifiers::CONTROL)));
        assert_eq!(app.flag_repeat_position(0), (1, 1));
        assert_eq!(app.current_repeated_flag_value(0).as_deref(), Some("c"));
        assert!(app.completion_generation > generation);

        assert!(app.handle_repeated_key(KeyEvent::new(
            KeyCode::Char('d'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )));
        assert_eq!(app.flag_repeat_position(0), (0, 0));
        assert_eq!(
            app.current_flag_values()[0].1,
            FlagValue::Repeated(vec![vec![RepeatInput::supplied("a")]])
        );
    }

    #[test]
    fn no_argument_var_flag_keeps_boolean_toggle_state() {
        let spec: usage::Spec = "name \"demo\"\nflag \"--verbose\" var=#true"
            .parse()
            .unwrap();
        let mut app = App::new(spec);
        assert_eq!(app.current_flag_values()[0].1, FlagValue::Bool(false));
        assert!(!app.flag_repeatable(0));
        app.set_focus(Focus::Flags);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert_eq!(app.current_flag_values()[0].1, FlagValue::Bool(true));
    }
}
