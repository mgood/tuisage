use crate::app::{App, FlagValue};
use crate::fields::field_id;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Response {
    version: u32,
    errors: BTreeMap<String, String>,
}

impl App {
    pub fn validation_errors(&self) -> BTreeMap<String, String> {
        let mut errors = BTreeMap::new();
        let mut cmd = &self.spec.cmd;
        for depth in 0..=self.command_path.len() {
            let path = &self.command_path[..depth];
            let values = self.flag_values.get(&path.join(" "));
            for flag in &cmd.flags {
                let value = values
                    .and_then(|values| values.iter().find(|(name, _)| name == &flag.name))
                    .map(|(_, value)| value);
                let count = match value {
                    Some(FlagValue::Bool(true) | FlagValue::NegBool(Some(_))) => 1,
                    Some(FlagValue::Count(count)) => *count as usize,
                    Some(FlagValue::String(value)) if !value.is_empty() => 1,
                    Some(FlagValue::EmptyString) => 1,
                    _ => 0,
                };
                let id = field_id(path, "flags", &flag.name, depth == 0 && flag.global);
                check_count(
                    &mut errors,
                    &id,
                    count,
                    flag.required,
                    flag.var_min,
                    flag.var_max,
                );
                if let Some(arg) = &flag.arg {
                    let strings = match value {
                        Some(FlagValue::String(s)) if !s.is_empty() => vec![s.as_str()],
                        Some(FlagValue::EmptyString) => vec![""],
                        _ => vec![],
                    };
                    check_choices(&mut errors, &id, &strings, arg.choices.as_ref());
                }
            }
            if depth < self.command_path.len() {
                cmd = cmd.find_subcommand(&self.command_path[depth]).unwrap();
            }
        }
        if cmd.subcommand_required {
            errors.insert("command".into(), "Select a subcommand".into());
        }
        for arg in &cmd.args {
            let values: Vec<_> = self
                .arg_values
                .iter()
                .filter(|value| {
                    value.name == arg.name && (value.supplied || !value.value.is_empty())
                })
                .map(|value| value.value.as_str())
                .collect();
            let id = field_id(&self.command_path, "args", &arg.name, false);
            check_count(
                &mut errors,
                &id,
                values.len(),
                arg.required,
                arg.var_min,
                arg.var_max,
            );
            check_choices(&mut errors, &id, &values, arg.choices.as_ref());
        }
        errors
    }

    pub fn validate_submission(&mut self, provider: Option<&std::path::Path>) -> bool {
        self.finish_editing();
        let mut errors = self.validation_errors();
        if errors.is_empty() {
            if let Some(provider) = provider {
                match crate::provider::run(provider, &crate::provider::context(self, None))
                    .and_then(|output| Ok(serde_json::from_slice::<Response>(&output)?))
                {
                    Ok(response) if response.version == 1 => errors.extend(response.errors),
                    Ok(_) => {
                        errors.insert(
                            "provider".into(),
                            "Unsupported validation response version".into(),
                        );
                    }
                    Err(error) => {
                        errors.insert("provider".into(), format!("Validation failed: {error}"));
                    }
                }
            }
        }
        self.submission_error = if errors.is_empty() {
            None
        } else {
            Some(
                errors
                    .iter()
                    .map(|(field, message)| format!("{field}: {message}"))
                    .collect::<Vec<_>>()
                    .join("; "),
            )
        };
        errors.is_empty()
    }
}

fn check_count(
    errors: &mut BTreeMap<String, String>,
    id: &str,
    count: usize,
    required: bool,
    min: Option<usize>,
    max: Option<usize>,
) {
    let min = min
        .unwrap_or(usize::from(required))
        .max(usize::from(required));
    if count < min {
        errors.insert(id.into(), format!("Provide at least {min} value(s)"));
    }
    if max.is_some_and(|max| count > max) {
        errors.insert(
            id.into(),
            format!("Provide at most {} value(s)", max.unwrap()),
        );
    }
}
fn check_choices(
    errors: &mut BTreeMap<String, String>,
    id: &str,
    values: &[&str],
    choices: Option<&usage::SpecChoices>,
) {
    if let Some(choices) = choices {
        for value in values {
            if !choices.choices.iter().any(|choice| choice == value) {
                errors.insert(
                    id.into(),
                    format!("Choose one of: {}", choices.choices.join(", ")),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn required_fields_and_choices_block_submission() {
        let spec: usage::Spec = "name \"demo\"\narg \"<blueprint>\"\nflag \"--format <format>\" { choices \"json\" \"text\"; }".parse().unwrap();
        let mut app = App::new(spec);
        assert!(!app.validate_submission(None));
        assert!(app.submission_error.as_ref().unwrap().contains("blueprint"));
        app.arg_values[0].value = "demo".into();
        app.current_flag_values_mut()[0].1 = FlagValue::String("invalid".into());
        assert!(!app.validate_submission(None));
        app.current_flag_values_mut()[0].1 = FlagValue::String("json".into());
        assert!(app.validate_submission(None));
    }

    #[test]
    fn supported_count_limits_and_explicit_empty_are_checked() {
        let mut errors = BTreeMap::new();
        check_count(&mut errors, "root/args/item", 0, false, Some(2), Some(3));
        assert!(errors["root/args/item"].contains("at least 2"));
        errors.clear();
        check_count(&mut errors, "root/args/item", 4, false, Some(2), Some(3));
        assert!(errors["root/args/item"].contains("at most 3"));

        let spec: usage::Spec = "name \"demo\"\narg \"[item]\"".parse().unwrap();
        let mut app = App::new(spec);
        app.arg_values[0].supplied = true;
        assert!(app.validation_errors().is_empty());
    }

    #[cfg(unix)]
    fn write_provider(body: &str) -> std::path::PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = std::env::temp_dir().join(format!(
            "tuisage-validation-provider-{}",
            std::process::id()
        ));
        std::fs::write(&path, body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn provider_accepts_rejects_malformed_and_failed_responses() {
        let spec: usage::Spec = "name \"demo\"".parse().unwrap();

        let accepted = write_provider(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"version\":1,\"errors\":{}}'\n",
        );
        assert!(App::new(spec.clone()).validate_submission(Some(&accepted)));

        let rejected = write_provider(
            "#!/bin/sh\ncat >/dev/null\nprintf '%s' '{\"version\":1,\"errors\":{\"root/args/name\":\"Rejected\"}}'\n",
        );
        let mut app = App::new(spec.clone());
        assert!(!app.validate_submission(Some(&rejected)));
        assert!(app
            .submission_error
            .unwrap()
            .contains("root/args/name: Rejected"));

        let malformed = write_provider("#!/bin/sh\ncat >/dev/null\nprintf '%s' broken\n");
        assert!(!App::new(spec.clone()).validate_submission(Some(&malformed)));

        let failed = write_provider("#!/bin/sh\ncat >/dev/null\nexit 3\n");
        assert!(!App::new(spec).validate_submission(Some(&failed)));
        std::fs::remove_file(failed).unwrap();
    }

    #[test]
    fn provider_failure_cannot_validate() {
        let mut app = App::new("name \"demo\"".parse().unwrap());
        assert!(!app.validate_submission(Some(std::path::Path::new(
            "/nonexistent-validation-provider",
        ))));
        assert!(app
            .submission_error
            .as_ref()
            .unwrap()
            .contains("Validation failed"));
    }
}
