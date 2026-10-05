use crate::app::{App, FlagValue};
use crate::fields::{field_id, fields, Field, FieldKind};
use serde::Deserialize;
use serde_json::Value;
use std::collections::{BTreeMap, HashSet};
use usage::{Spec, SpecFlag};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    value: Value,
    #[serde(default)]
    locked: bool,
}

#[derive(Clone)]
pub struct InitialField {
    pub field: Field,
    pub value: Value,
    pub locked: bool,
}

pub fn parse(source: &str, spec: &Spec) -> color_eyre::Result<Vec<InitialField>> {
    let text = if let Some(path) = source.strip_prefix('@') {
        std::fs::read_to_string(path)?
    } else {
        source.to_owned()
    };
    #[derive(Deserialize)]
    struct Document(#[serde(deserialize_with = "unique_fields")] BTreeMap<String, Input>);
    let input = serde_json::from_str::<Document>(&text)?.0;
    let fields = fields(spec);
    let mut used = HashSet::new();
    let mut result = Vec::new();
    for (id, input) in input {
        let matches: Vec<_> = fields
            .iter()
            .filter(|f| f.id == id || (!id.contains('/') && f.name == id))
            .collect();
        if matches.len() != 1 {
            return Err(color_eyre::eyre::eyre!(
                "Default field '{id}' is unknown or ambiguous; use a qualified field identifier"
            ));
        }
        let field = matches[0].clone();
        if !used.insert(field.id.clone()) {
            return Err(color_eyre::eyre::eyre!(
                "Conflicting defaults for '{}'",
                field.id
            ));
        }
        match &field.kind {
            FieldKind::Flag(flag) => {
                flag_value(flag, &input.value)?;
            }
            FieldKind::Arg(arg) if input.value.is_string() => {
                if let Some(choices) = &arg.choices {
                    if !choices
                        .choices
                        .iter()
                        .any(|s| Some(s.as_str()) == input.value.as_str())
                    {
                        return Err(color_eyre::eyre::eyre!(
                            "Default '{}' is not a declared choice",
                            field.id
                        ));
                    }
                }
            }
            _ => {
                return Err(color_eyre::eyre::eyre!(
                    "Default '{}' requires a string",
                    field.id
                ))
            }
        }
        result.push(InitialField {
            field,
            value: input.value,
            locked: input.locked,
        });
    }
    Ok(result)
}

fn unique_fields<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<BTreeMap<String, Input>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, Input>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("an object with unique field identifiers")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> Result<Self::Value, M::Error> {
            let mut fields = BTreeMap::new();
            while let Some((key, value)) = map.next_entry::<String, Input>()? {
                if fields.insert(key.clone(), value).is_some() {
                    return Err(serde::de::Error::custom(format!(
                        "Duplicate default field '{key}'"
                    )));
                }
            }
            Ok(fields)
        }
    }
    deserializer.deserialize_map(Visitor)
}

pub fn flag_value(flag: &SpecFlag, value: &Value) -> color_eyre::Result<FlagValue> {
    let invalid = || color_eyre::eyre::eyre!("Invalid value type for default '{}'", flag.name);
    if flag.count {
        let count = value
            .as_u64()
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(invalid)?;
        Ok(FlagValue::Count(count))
    } else if flag.arg.is_some() {
        let value = value.as_str().ok_or_else(invalid)?;
        if let Some(choices) = flag.arg.as_ref().and_then(|arg| arg.choices.as_ref()) {
            if !choices.choices.iter().any(|s| s == value) {
                return Err(color_eyre::eyre::eyre!(
                    "Default '{}' is not a declared choice",
                    flag.name
                ));
            }
        }
        Ok(if value.is_empty() {
            FlagValue::EmptyString
        } else {
            FlagValue::String(value.into())
        })
    } else if flag.negate.is_some() {
        Ok(FlagValue::NegBool(Some(
            value.as_bool().ok_or_else(invalid)?,
        )))
    } else {
        Ok(FlagValue::Bool(value.as_bool().ok_or_else(invalid)?))
    }
}

impl App {
    pub fn configure_defaults(&mut self, initial: Vec<InitialField>) {
        self.initial_fields = initial;
        self.apply_initial_fields(false);
        self.sync_state();
    }

    pub fn apply_initial_fields(&mut self, locked_only: bool) {
        for initial in self
            .initial_fields
            .clone()
            .into_iter()
            .filter(|f| !locked_only || f.locked)
        {
            let mut cmd = &self.spec.cmd;
            for name in &initial.field.path {
                cmd = cmd.find_subcommand(name).expect("validated field path");
            }
            let key = initial.field.path.join(" ");
            match initial.field.kind {
                FieldKind::Arg(_) => {
                    let args = self
                        .arg_values_by_path
                        .entry(key)
                        .or_insert_with(|| Self::default_arg_values_for_command(cmd));
                    if let Some(arg) = args.iter_mut().find(|a| a.name == initial.field.name) {
                        arg.value = initial.value.as_str().unwrap().into();
                        arg.supplied = true;
                    }
                }
                FieldKind::Flag(ref flag) => {
                    let flags = crate::app::collect_visible_flags(cmd, &self.spec);
                    let values = self.flag_values.entry(key).or_insert_with(|| {
                        flags
                            .iter()
                            .map(|f| (f.name.clone(), crate::app::default_flag_value(f)))
                            .collect()
                    });
                    let value = flag_value(flag, &initial.value).expect("validated default type");
                    if let Some((_, current)) = values
                        .iter_mut()
                        .find(|(name, _)| name == &initial.field.name)
                    {
                        *current = value.clone();
                    }
                    if flag.global {
                        self.sync_global_flag(&flag.name, &value);
                    }
                }
            }
        }
    }

    pub fn flag_locked(&self, index: usize) -> bool {
        self.visible_flags().get(index).is_some_and(|flag| {
            let id = field_id(
                &self.command_path,
                "flags",
                &flag.name,
                flag.global
                    && self
                        .spec
                        .cmd
                        .flags
                        .iter()
                        .any(|f| f.global && f.name == flag.name),
            );
            self.initial_fields
                .iter()
                .any(|f| f.locked && f.field.id == id)
        })
    }
    pub fn arg_locked(&self, index: usize) -> bool {
        self.arg_values.get(index).is_some_and(|arg| {
            let id = field_id(&self.command_path, "args", &arg.name, false);
            self.initial_fields
                .iter()
                .any(|f| f.locked && f.field.id == id)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::Focus;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    fn spec() -> Spec {
        "name \"demo\"\nflag \"--backend <backend>\" global=#true\nflag \"--verbose\"\ncmd \"run\" { arg \"[value]\"; }".parse().unwrap()
    }
    #[test]
    fn empty_flag_default_uses_composition_scalar_representation() {
        let mut app = App::new(spec());
        let initial = parse(r#"{"backend":{"value":""}}"#, &app.spec).unwrap();
        app.configure_defaults(initial);
        assert!(app
            .current_flag_values()
            .iter()
            .any(|(name, value)| name == "backend" && matches!(value, FlagValue::EmptyString)));
        assert!(app
            .build_command_parts()
            .windows(2)
            .any(|parts| parts == ["--backend", ""]));
    }

    #[test]
    fn typed_defaults_and_locked_keyboard_reset() {
        let mut app = App::new(spec());
        let initial = parse(
            r#"{"backend":{"value":"orbitron","locked":true},"verbose":{"value":false}}"#,
            &app.spec,
        )
        .unwrap();
        app.configure_defaults(initial);
        app.navigate_to_command(&["run"]);
        app.set_focus(Focus::Flags);
        let index = app
            .visible_flags()
            .iter()
            .position(|f| f.name == "backend")
            .unwrap();
        app.set_flag_index(index);
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        assert!(!app.is_editing());
        app.handle_key(KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE));
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        assert!(app
            .build_command_parts()
            .windows(2)
            .any(|s| s == ["--backend", "orbitron"]));
    }
    #[test]
    fn explicit_empty_defaults_are_arguments_and_locks_block_mouse() {
        let mut app = App::new(spec());
        let initial = parse(
            r#"{"backend":{"value":"","locked":true},"value":{"value":"","locked":true}}"#,
            &app.spec,
        )
        .unwrap();
        app.configure_defaults(initial);
        app.navigate_to_command(&["run"]);
        assert_eq!(
            app.build_command_parts(),
            ["demo", "--backend", "", "run", ""]
        );
        assert_eq!(app.build_command(), "demo --backend \"\" run \"\"");
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|frame| crate::ui::render(frame, &mut app))
            .unwrap();
        let rect = app
            .layout
            .click_regions
            .regions()
            .iter()
            .find(|r| r.data == Focus::Flags)
            .unwrap()
            .area;
        app.handle_mouse(crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: rect.x + 3,
            row: rect.y + 1,
            modifiers: KeyModifiers::NONE,
        });
        assert!(!app.is_editing());
        assert_eq!(
            app.build_command_parts(),
            ["demo", "--backend", "", "run", ""]
        );
        assert!(parse(
            r#"{"backend":{"value":"a"},"backend":{"value":"b"}}"#,
            &app.spec
        )
        .is_err());
    }

    #[test]
    fn rejects_unknown_ambiguous_conflicting_and_bad_types() {
        let spec: Spec =
            "name \"demo\"\nflag \"--value <value>\"\ncmd \"run\" { flag \"--value <value>\"; }"
                .parse()
                .unwrap();
        assert!(parse(r#"{"value":{"value":"a"}}"#, &spec).is_err());
        assert!(parse(r#"{"missing":{"value":"a"}}"#, &spec).is_err());
        assert!(parse(r#"{"root/flags/value":{"value":false}}"#, &spec).is_err());
        assert!(parse(
            r#"{"backend":{"value":"a"},"global/flags/backend":{"value":"b"}}"#,
            &self::spec()
        )
        .is_err());
        assert!(parse(
            r#"{"root/flags/value":{"value":"a"},"commands/run/flags/value":{"value":"b"}}"#,
            &spec
        )
        .is_ok());
    }

    #[test]
    fn qualified_global_root_and_command_fields_remain_distinct() {
        let spec: Spec = r#"
            name "demo"
            flag "--scope <scope>" global=#true
            arg "[item]"
            cmd "run" {
                flag "--profile <profile>"
                arg "[item]"
            }
        "#
        .parse()
        .unwrap();
        let mut app = App::new(spec.clone());
        let initial = parse(
            r#"{"global/flags/scope":{"value":"all"},"root/args/item":{"value":"root"},"commands/run/flags/profile":{"value":"local"},"commands/run/args/item":{"value":"child"}}"#,
            &spec,
        ).unwrap();
        app.configure_defaults(initial);

        assert_eq!(
            app.build_command_parts(),
            [
                "demo",
                "--scope",
                "all",
                "run",
                "--profile",
                "local",
                "child"
            ]
        );
        assert_eq!(
            app.arg_values_by_path[""]
                .iter()
                .find(|arg| arg.name == "item")
                .unwrap()
                .value,
            "root"
        );
    }

    #[test]
    fn typed_boolean_count_and_choice_values_are_validated() {
        let spec: Spec = r#"
            name "demo"
            flag "--verbose" count=#true
            flag "--quiet"
            flag "--mode <mode>" {
                arg "<mode>" {
                    choices "fast" "safe"
                }
            }
        "#
        .parse()
        .unwrap();

        assert!(parse(
            r#"{"verbose":{"value":2},"quiet":{"value":true},"mode":{"value":"fast"}}"#,
            &spec
        )
        .is_ok());
        assert!(parse(r#"{"verbose":{"value":-1}}"#, &spec).is_err());
        assert!(parse(r#"{"quiet":{"value":"yes"}}"#, &spec).is_err());
        assert!(parse(r#"{"mode":{"value":"unsafe"}}"#, &spec).is_err());
    }
}
