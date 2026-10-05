use crate::app::App;
use crate::companion::List;
use crate::fields::field_id;
use serde::Deserialize;
use serde_json::Value;
use usage::SpecComplete;

type Choices = (Vec<String>, Vec<Option<String>>);
#[derive(Clone)]
pub struct Pending {
    pub generation: u64,
    pub ready: bool,
    pub flag: bool,
    pub index: usize,
    pub name: String,
    pub column: u16,
    pub context: Value,
}
pub struct ResultMessage {
    pub generation: u64,
    pub choices: Option<Choices>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    choices: Vec<Choice>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Choice {
    value: String,
    #[serde(default)]
    description: Option<String>,
}

fn run(complete: &SpecComplete, context: &Value) -> Option<Choices> {
    if let Some(program) = complete
        .type_
        .as_deref()
        .and_then(|s| s.strip_prefix("tuisage-json-v1:"))
    {
        let bytes = crate::provider::run(std::path::Path::new(program), context).ok()?;
        let response: Response = serde_json::from_slice(&bytes).ok()?;
        if response.version != 1 {
            return None;
        }
        return Some(
            response
                .choices
                .into_iter()
                .map(|choice| (choice.value, choice.description))
                .unzip(),
        );
    }
    let command = complete.run.as_ref()?;
    let output = std::process::Command::new("sh")
        .args(["-c", command])
        .env("TUISAGE_CONTEXT", context.to_string())
        .env("TUISAGE_CONTEXT_VERSION", "1")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(App::parse_completion_output(
        &output.stdout,
        complete.descriptions,
    ))
}

fn run_list(list: &List, context: &Value) -> Option<Choices> {
    match list {
        List::Fixed(choices) => Some(
            choices
                .iter()
                .map(|choice| (choice.value.clone(), choice.description.clone()))
                .unzip(),
        ),
        List::Provider { executable, argv } => {
            let bytes = crate::provider::run_with_args(executable, argv, context).ok()?;
            let response: Response = serde_json::from_slice(&bytes).ok()?;
            (response.version == 1).then(|| {
                response
                    .choices
                    .into_iter()
                    .map(|choice| (choice.value, choice.description))
                    .unzip()
            })
        }
    }
}

impl App {
    pub fn start_completion(&mut self, flag: bool, index: usize, name: &str, column: u16) -> bool {
        let field = if flag {
            let Some(spec) = self.visible_flags().get(index).copied() else {
                return false;
            };
            field_id(&self.command_path, "flags", &spec.name, spec.global)
        } else {
            let Some(arg) = self.arg_values.get(index) else {
                return false;
            };
            field_id(&self.command_path, "args", &arg.name, false)
        };
        let list = self
            .companion
            .as_ref()
            .and_then(|companion| {
                companion
                    .fields
                    .get(&field)
                    .and_then(|id| companion.lists.get(id))
            })
            .cloned();
        let complete = self.find_completion(name).cloned();
        if list.is_none() {
            let Some(complete) = complete.as_ref() else {
                return false;
            };
            if complete.run.is_none()
                && !complete
                    .type_
                    .as_deref()
                    .is_some_and(|s| s.starts_with("tuisage-json-v1:"))
            {
                return false;
            }
        }
        let immediate = match list.as_ref() {
            Some(List::Fixed(choices)) => Some(
                choices
                    .iter()
                    .map(|choice| (choice.value.clone(), choice.description.clone()))
                    .unzip(),
            ),
            _ => None,
        };
        let context = crate::provider::context(self, Some(&field));
        self.completion_generation += 1;
        let generation = self.completion_generation;
        let pending = Pending {
            generation,
            ready: immediate.is_some(),
            flag,
            index,
            name: name.into(),
            column,
            context: context.clone(),
        };
        let tx = self.completion_tx.clone();
        if immediate.is_none() {
            let list = list.clone();
            std::thread::spawn(move || {
                let choices = match list.as_ref() {
                    Some(list) => run_list(list, &context),
                    None => complete
                        .as_ref()
                        .and_then(|complete| run(complete, &context)),
                };
                let _ = tx.send(ResultMessage {
                    generation,
                    choices,
                });
            });
        }
        self.pending_completion = Some(pending);
        let value = self.completion_value(flag, index);
        let (choices, descriptions) = immediate.unwrap_or_default();
        if flag {
            if self.flag_panel.choice_select_index() == Some(index) {
                self.flag_panel
                    .update_completion_choices(choices, descriptions);
            } else {
                self.flag_panel.open_completion_select(
                    index,
                    choices,
                    descriptions,
                    &value,
                    column,
                );
            }
            self.flag_panel
                .set_completion_loading(!self.pending_completion.as_ref().unwrap().ready);
        } else if self.arg_panel.choice_select_index() == Some(index) {
            self.arg_panel
                .update_completion_choices(choices, descriptions);
            self.arg_panel
                .set_completion_loading(!self.pending_completion.as_ref().unwrap().ready);
        } else {
            self.arg_panel
                .open_completion_select(index, choices, descriptions, &value, column);
            self.arg_panel
                .set_completion_loading(!self.pending_completion.as_ref().unwrap().ready);
        }
        true
    }

    #[cfg(test)]
    pub fn wait_for_completion(&mut self) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while self.pending_completion.as_ref().is_some_and(|p| !p.ready) {
            self.poll_completion();
            assert!(
                std::time::Instant::now() < deadline,
                "completion did not finish"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    fn completion_value(&self, flag: bool, index: usize) -> String {
        if flag {
            self.current_flag_values()
                .get(index)
                .map(|(_, value)| match value {
                    crate::app::FlagValue::String(value) => value.clone(),
                    crate::app::FlagValue::EmptyString => String::new(),
                    crate::app::FlagValue::Repeated(_) => {
                        self.current_repeated_flag_value(index).unwrap_or_default()
                    }
                    _ => String::new(),
                })
                .unwrap_or_default()
        } else {
            self.arg_values
                .get(index)
                .map(|arg| arg.value.clone())
                .unwrap_or_default()
        }
    }

    pub fn poll_completion(&mut self) {
        if let Some(pending) = self.pending_completion.clone() {
            let open = if pending.flag {
                self.flag_panel.choice_select_index() == Some(pending.index)
            } else {
                self.arg_panel.choice_select_index() == Some(pending.index)
            };
            if !open {
                self.pending_completion = None;
            } else if crate::provider::context(self, pending.context["field"].as_str())
                != pending.context
            {
                self.start_completion(pending.flag, pending.index, &pending.name, pending.column);
            }
        }
        while let Ok(message) = self.completion_rx.try_recv() {
            let Some(pending) = self.pending_completion.clone() else {
                continue;
            };
            if pending.generation != message.generation {
                continue;
            }
            if crate::provider::context(self, pending.context["field"].as_str()) != pending.context
            {
                continue;
            }
            let (choices, descriptions) = message.choices.unwrap_or_default();
            if pending.flag {
                self.flag_panel
                    .update_completion_choices(choices, descriptions);
                self.flag_panel.set_completion_loading(false);
            } else {
                self.arg_panel
                    .update_completion_choices(choices, descriptions);
                self.arg_panel.set_completion_loading(false);
            }
            if let Some(pending) = self.pending_completion.as_mut() {
                pending.ready = true;
            }
            // Retain context for future dependent changes, without polling a provider repeatedly.
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{FlagValue, Focus};
    use crate::companion::{Choice, Companion};

    #[test]
    fn fixed_presage_list_completes_the_qualified_field_immediately() {
        let spec: usage::Spec = "name \"demo\"\narg \"[region]\"".parse().unwrap();
        let mut app = App::new(spec);
        let mut companion = Companion::default();
        companion.lists.insert(
            "regions".into(),
            List::Fixed(vec![Choice {
                value: "eu-west-1".into(),
                description: Some("Ireland".into()),
            }]),
        );
        companion
            .fields
            .insert("root/args/region".into(), "regions".into());
        app.companion = Some(companion);
        app.set_focus(Focus::Args);

        assert!(app.start_completion(false, 0, "region", 0));
        assert!(app.pending_completion.as_ref().unwrap().ready);
        assert_eq!(
            app.arg_panel.filtered_choices(),
            vec![(0, "eu-west-1".into())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn provider_list_receives_literal_arguments_and_keeps_empty_unicode_values() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("provider");
        std::fs::write(
            &program,
            "#!/bin/sh\nprintf '{\"version\":1,\"choices\":[{\"value\":\"%s\"},{\"value\":\"\"},{\"value\":\"Zürich\"}]}' \"$1\"\n",
        )
        .unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();
        let list = List::Provider {
            executable: program,
            argv: vec!["two words; $(literal)".into()],
        };

        let (values, _) = run_list(&list, &serde_json::json!({})).unwrap();
        assert_eq!(values, ["two words; $(literal)", "", "Zürich"]);
    }

    #[cfg(unix)]
    #[test]
    fn failed_provider_has_no_choices_and_fixed_empty_list_is_valid() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("provider");
        std::fs::write(&program, "#!/bin/sh\nexit 3\n").unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();

        assert!(run_list(
            &List::Provider {
                executable: program,
                argv: vec![],
            },
            &serde_json::json!({}),
        )
        .is_none());
        assert_eq!(
            run_list(&List::Fixed(vec![]), &serde_json::json!({})),
            Some((vec![], vec![]))
        );
    }

    #[cfg(unix)]
    #[test]
    fn failed_presage_provider_keeps_manual_entry_open() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().unwrap();
        let program = directory.path().join("provider");
        std::fs::write(&program, "#!/bin/sh\nexit 3\n").unwrap();
        let mut permissions = std::fs::metadata(&program).unwrap().permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(&program, permissions).unwrap();
        let spec: usage::Spec = "name \"demo\"\narg \"[region]\"".parse().unwrap();
        let mut app = App::new(spec);
        let mut companion = Companion::default();
        companion.lists.insert(
            "regions".into(),
            List::Provider {
                executable: program,
                argv: vec![],
            },
        );
        companion
            .fields
            .insert("root/args/region".into(), "regions".into());
        app.companion = Some(companion);
        app.arg_values[0].value = "typed".into();
        app.set_focus(Focus::Args);

        assert!(app.start_completion(false, 0, "region", 0));
        app.wait_for_completion();
        assert!(app.is_choosing());
        assert_eq!(app.arg_panel.choice_select_text(), "typed");
        assert!(app.arg_panel.filtered_choices().is_empty());
    }

    #[test]
    fn legacy_provider_receives_current_context_and_stale_result_is_ignored() {
        let spec: usage::Spec = "name \"demo\"\nflag \"--backend <backend>\" global=#true\narg \"[service]\"\ncomplete \"service\" run=\"printf '%s' \\\"$TUISAGE_CONTEXT\\\"\"".parse().unwrap();
        let mut app = App::new(spec);
        app.set_focus(Focus::Args);
        app.current_flag_values_mut()[0].1 = FlagValue::String("old".into());
        assert!(app.start_completion(false, 0, "service", 5));
        let old = app.completion_generation;
        app.current_flag_values_mut()[0].1 = FlagValue::String("new".into());
        app.poll_completion();
        assert!(app.completion_generation > old);
        app.completion_tx
            .send(ResultMessage {
                generation: old,
                choices: Some((vec!["stale".into()], vec![None])),
            })
            .unwrap();
        app.poll_completion();
        let context = &app.pending_completion.as_ref().unwrap().context;
        assert_eq!(context["version"], 1);
        assert_eq!(context["command"], serde_json::json!([]));
        assert_eq!(context["field"], "root/args/service");
        assert_eq!(context["fields"]["global/flags/backend"], "new");
        assert_eq!(context["argv"], serde_json::json!(["--backend", "new"]));
        app.wait_for_completion();
        let choices = app.arg_panel.filtered_choices();
        assert_eq!(choices.len(), 1);
        assert!(choices[0].1.contains("new"));
        app.completion_tx
            .send(ResultMessage {
                generation: old,
                choices: Some((vec!["stale".into()], vec![None])),
            })
            .unwrap();
        app.poll_completion();
        assert!(app.arg_panel.filtered_choices()[0].1.contains("new"));
    }

    #[test]
    fn selecting_another_repeatable_positional_row_invalidates_completion() {
        let spec: usage::Spec = r#"
            name "demo"
            arg "<files>..." var=#true
            complete "files" run="printf 'choice\\n'"
        "#
        .parse()
        .unwrap();
        let mut app = App::new(spec);
        app.arg_values.push(app.arg_values[0].clone());
        app.arg_panel.set_total(app.arg_values.len());
        app.set_focus(Focus::Args);
        assert!(app.start_completion(false, 0, "files", 0));
        let generation = app.completion_generation;

        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Esc,
            crossterm::event::KeyModifiers::NONE,
        ));

        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Down,
            crossterm::event::KeyModifiers::NONE,
        ));

        assert_eq!(app.arg_index(), 1);
        assert!(app.completion_generation > generation);
        assert!(app.pending_completion.is_none());
    }

    #[cfg(unix)]
    #[test]
    fn failed_provider_keeps_manual_entry_available() {
        let spec: usage::Spec = r#"
            name "demo"
            arg "[service]"
            complete "service" run="exit 1"
        "#
        .parse()
        .unwrap();
        let mut app = App::new(spec);
        app.set_focus(Focus::Args);
        assert!(app.start_completion(false, 0, "service", 5));
        app.wait_for_completion();
        assert!(app.is_choosing());

        for character in "manual".chars() {
            app.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(character),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.arg_values[0].value, "manual");
    }

    #[cfg(unix)]
    #[test]
    fn slow_provider_does_not_block_form_interaction() {
        let spec: usage::Spec = r#"
            name "demo"
            arg "[service]"
            complete "service" run="sleep 0.3; printf 'ready\n'"
        "#
        .parse()
        .unwrap();
        let mut app = App::new(spec);
        app.set_focus(Focus::Args);
        assert!(app.start_completion(false, 0, "service", 5));
        assert!(!app.pending_completion.as_ref().unwrap().ready);

        for character in "manual".chars() {
            app.handle_key(crossterm::event::KeyEvent::new(
                crossterm::event::KeyCode::Char(character),
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        app.handle_key(crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ));
        assert_eq!(app.arg_values[0].value, "manual");
    }

    #[cfg(unix)]
    #[test]
    fn arriving_completion_preserves_manual_text_and_cursor() {
        let spec: usage::Spec = r#"
            name "demo"
            arg "[service]"
            complete "service" run="sleep 0.3; printf 'ready\n'"
        "#
        .parse()
        .unwrap();
        let mut app = App::new(spec);
        app.set_focus(Focus::Args);
        assert!(app.start_completion(false, 0, "service", 5));
        for code in [
            crossterm::event::KeyCode::Char('世'),
            crossterm::event::KeyCode::Char('界'),
            crossterm::event::KeyCode::Left,
        ] {
            app.handle_key(crossterm::event::KeyEvent::new(
                code,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        app.wait_for_completion();
        assert_eq!(app.arg_panel.choice_select_text(), "世界");
        for code in [
            crossterm::event::KeyCode::Char('!'),
            crossterm::event::KeyCode::Enter,
        ] {
            app.handle_key(crossterm::event::KeyEvent::new(
                code,
                crossterm::event::KeyModifiers::NONE,
            ));
        }
        assert_eq!(app.arg_values[0].value, "世!界");
    }

    #[test]
    fn structured_provider_response_retains_empty_unicode_and_descriptions() {
        let response: Response = serde_json::from_str(
            r#"{"version":1,"choices":[{"value":"","description":"empty"},{"value":"世界"}]}"#,
        )
        .unwrap();
        assert_eq!(response.choices[0].value, "");
        assert_eq!(response.choices[1].value, "世界");
    }
}
