use crate::app::App;
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

impl App {
    pub fn start_completion(&mut self, flag: bool, index: usize, name: &str, column: u16) -> bool {
        let Some(complete) = self.find_completion(name).cloned() else {
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
        let context = crate::provider::context(self, Some(&field));
        self.completion_generation += 1;
        let generation = self.completion_generation;
        let pending = Pending {
            generation,
            ready: false,
            flag,
            index,
            name: name.into(),
            column,
            context: context.clone(),
        };
        let tx = self.completion_tx.clone();
        std::thread::spawn(move || {
            let choices = run(&complete, &context);
            let _ = tx.send(ResultMessage {
                generation,
                choices,
            });
        });
        self.pending_completion = Some(pending);
        let value = self.completion_value(flag, index);
        if flag {
            if self.flag_panel.choice_select_index() == Some(index) {
                self.flag_panel.update_completion_choices(vec![], vec![]);
            } else {
                self.flag_panel
                    .open_completion_select(index, vec![], vec![], &value, column);
            }
        } else if self.arg_panel.choice_select_index() == Some(index) {
            self.arg_panel.update_completion_choices(vec![], vec![]);
        } else {
            self.arg_panel
                .open_completion_select(index, vec![], vec![], &value, column);
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
            } else {
                self.arg_panel
                    .update_completion_choices(choices, descriptions);
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
