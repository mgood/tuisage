use std::collections::HashMap;

use usage::{Spec, SpecFlag};

use crate::app::{ArgValue, FlagValue};

/// Resolve the flag spec for a given name, checking the provided flags first,
/// then falling back to global flags on the root command.
fn find_flag_spec<'a>(
    name: &str,
    flags: &'a [SpecFlag],
    global_flags: &'a [SpecFlag],
) -> Option<&'a SpecFlag> {
    flags
        .iter()
        .find(|f| f.name == name)
        .or_else(|| global_flags.iter().find(|f| f.name == name && f.global))
}

/// Format a flag and its value as a single display string (for the command preview).
/// Returns `None` if the flag is unset / default.
pub fn format_flag_value(
    name: &str,
    value: &FlagValue,
    flags: &[SpecFlag],
    global_flags: &[SpecFlag],
) -> Option<String> {
    let flag = find_flag_spec(name, flags, global_flags)?;

    match value {
        FlagValue::Bool(true) => {
            let prefix = if let Some(long) = flag.long.first() {
                format!("--{long}")
            } else {
                let short = flag.short.first()?;
                format!("-{short}")
            };
            Some(prefix)
        }
        FlagValue::Bool(false) => None,
        FlagValue::NegBool(None) => None,
        FlagValue::NegBool(Some(true)) => {
            let prefix = if let Some(long) = flag.long.first() {
                format!("--{long}")
            } else {
                let short = flag.short.first()?;
                format!("-{short}")
            };
            Some(prefix)
        }
        FlagValue::NegBool(Some(false)) => flag.negate.clone(),
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
        FlagValue::EmptyString => {
            let prefix = flag
                .long
                .first()
                .map(|long| format!("--{long}"))
                .or_else(|| flag.short.first().map(|short| format!("-{short}")))?;
            Some(format!("{prefix} \"\""))
        }
        FlagValue::Repeated(groups) => {
            let prefix = flag
                .long
                .first()
                .map(|long| format!("--{long}"))
                .or_else(|| flag.short.first().map(|short| format!("-{short}")))?;
            let mut words = Vec::new();
            for group in groups {
                let values: Vec<_> = group
                    .iter()
                    .filter(|input| input.supplied || !input.value.is_empty())
                    .collect();
                if values.is_empty() {
                    continue;
                }
                words.push(prefix.clone());
                words.extend(values.into_iter().map(|input| input.value.clone()));
            }
            (!words.is_empty()).then(|| shell_words::join(words.iter().map(String::as_str)))
        }
        FlagValue::String(s) => {
            let prefix = if let Some(long) = flag.long.first() {
                format!("--{long}")
            } else {
                let short = flag.short.first()?;
                format!("-{short}")
            };
            if s.contains(' ') {
                Some(format!("{prefix} \"{s}\""))
            } else {
                Some(format!("{prefix} {s}"))
            }
        }
    }
}

/// Append flag parts (as separate process arguments) to the parts list.
pub fn format_flag_parts(
    name: &str,
    value: &FlagValue,
    flags: &[SpecFlag],
    global_flags: &[SpecFlag],
    parts: &mut Vec<String>,
) {
    let Some(flag) = find_flag_spec(name, flags, global_flags) else {
        return;
    };

    match value {
        FlagValue::Bool(true) => {
            if let Some(long) = flag.long.first() {
                parts.push(format!("--{long}"));
            } else if let Some(short) = flag.short.first() {
                parts.push(format!("-{short}"));
            }
        }
        FlagValue::Bool(false) => {}
        FlagValue::NegBool(None) => {}
        FlagValue::NegBool(Some(true)) => {
            if let Some(long) = flag.long.first() {
                parts.push(format!("--{long}"));
            } else if let Some(short) = flag.short.first() {
                parts.push(format!("-{short}"));
            }
        }
        FlagValue::NegBool(Some(false)) => {
            if let Some(negate) = &flag.negate {
                parts.push(negate.clone());
            }
        }
        FlagValue::Count(0) => {}
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
        FlagValue::EmptyString => {
            if let Some(long) = flag.long.first() {
                parts.push(format!("--{long}"));
            } else if let Some(short) = flag.short.first() {
                parts.push(format!("-{short}"));
            } else {
                return;
            }
            parts.push(String::new());
        }
        FlagValue::Repeated(groups) => {
            let Some(prefix) = flag
                .long
                .first()
                .map(|long| format!("--{long}"))
                .or_else(|| flag.short.first().map(|short| format!("-{short}")))
            else {
                return;
            };
            for group in groups {
                let values: Vec<_> = group
                    .iter()
                    .filter(|input| input.supplied || !input.value.is_empty())
                    .collect();
                if values.is_empty() {
                    continue;
                }
                parts.push(prefix.clone());
                parts.extend(values.into_iter().map(|input| input.value.clone()));
            }
        }
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

/// State needed for live preview of in-progress arg edits.
pub struct LiveArgPreview<'a> {
    /// Index of the arg currently being edited via choice select, if any.
    pub choice_select_index: Option<usize>,
    /// Current text in the choice select input.
    pub choice_select_text: &'a str,
    /// Whether inline editing is active.
    pub is_editing: bool,
    /// Index of the currently selected arg (for inline editing).
    pub editing_index: usize,
    /// Current text in the inline editor.
    pub editing_text: &'a str,
}

/// Resolve the effective arg value, using live preview state when applicable.
fn effective_arg_value<'a>(
    index: usize,
    arg: &'a ArgValue,
    preview: &'a LiveArgPreview<'a>,
) -> &'a str {
    if preview.choice_select_index == Some(index) {
        preview.choice_select_text
    } else if preview.is_editing && preview.editing_index == index {
        preview.editing_text
    } else {
        &arg.value
    }
}

/// Build the full command string from the current state (for display).
/// Values containing spaces are quoted.
pub fn build_command(
    spec: &Spec,
    flag_values: &HashMap<String, Vec<(String, FlagValue)>>,
    command_path: &[String],
    arg_values: &[ArgValue],
    preview: &LiveArgPreview,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    let bin = if spec.bin.is_empty() {
        &spec.name
    } else {
        &spec.bin
    };
    parts.push(bin.clone());

    // Global flag values from root
    let root_key = String::new();
    if let Some(root_flags) = flag_values.get(&root_key) {
        for (name, value) in root_flags {
            if let Some(flag_str) = format_flag_value(name, value, &spec.cmd.flags, &spec.cmd.flags)
            {
                parts.push(flag_str);
            }
        }
    }

    // Subcommand path with per-level flags
    let mut cmd = &spec.cmd;
    for (i, name) in command_path.iter().enumerate() {
        parts.push(name.clone());

        if let Some(sub) = cmd.find_subcommand(name) {
            cmd = sub;

            let path_key = command_path[..=i].join(" ");
            if let Some(level_flags) = flag_values.get(&path_key) {
                for (fname, fvalue) in level_flags {
                    let is_global = spec.cmd.flags.iter().any(|f| f.global && f.name == *fname);
                    if is_global {
                        continue;
                    }
                    if let Some(flag_str) =
                        format_flag_value(fname, fvalue, &cmd.flags, &spec.cmd.flags)
                    {
                        parts.push(flag_str);
                    }
                }
            }
        }
    }

    // Positional arg values (with live preview)
    for (i, arg) in arg_values.iter().enumerate() {
        let value = effective_arg_value(i, arg, preview);
        if arg.supplied || !value.is_empty() {
            let is_variadic = command_args(spec, command_path)
                .iter()
                .any(|spec_arg| spec_arg.name == arg.name && spec_arg.var);
            if is_variadic {
                parts.push(shell_words::quote(value).into_owned());
            } else if value.is_empty() {
                parts.push("\"\"".to_string());
            } else if value.contains(' ') {
                parts.push(format!("\"{value}\""));
            } else {
                parts.push(value.to_string());
            }
        }
    }

    parts.join(" ")
}

fn command_args<'a>(spec: &'a Spec, command_path: &[String]) -> &'a [usage::SpecArg] {
    let mut command = &spec.cmd;
    for name in command_path {
        let Some(child) = command.find_subcommand(name) else {
            return &[];
        };
        command = child;
    }
    &command.args
}

/// Build the command as a list of separate argument strings (for process execution).
/// Unlike `build_command()`, this does NOT quote values — each element is a separate arg.
pub fn build_command_parts(
    spec: &Spec,
    flag_values: &HashMap<String, Vec<(String, FlagValue)>>,
    command_path: &[String],
    arg_values: &[ArgValue],
) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();

    let bin = if spec.bin.is_empty() {
        &spec.name
    } else {
        &spec.bin
    };
    parts.extend(shell_words::split(bin).expect("validated base command"));

    // Global flag values from root
    let root_key = String::new();
    if let Some(root_flags) = flag_values.get(&root_key) {
        for (name, value) in root_flags {
            format_flag_parts(name, value, &spec.cmd.flags, &spec.cmd.flags, &mut parts);
        }
    }

    // Subcommand path with per-level flags
    let mut cmd = &spec.cmd;
    for (i, name) in command_path.iter().enumerate() {
        parts.push(name.clone());

        if let Some(sub) = cmd.find_subcommand(name) {
            cmd = sub;

            let path_key = command_path[..=i].join(" ");
            if let Some(level_flags) = flag_values.get(&path_key) {
                for (fname, fvalue) in level_flags {
                    let is_global = spec.cmd.flags.iter().any(|f| f.global && f.name == *fname);
                    if is_global {
                        continue;
                    }
                    format_flag_parts(fname, fvalue, &cmd.flags, &spec.cmd.flags, &mut parts);
                }
            }
        }
    }

    // Positional arg values (unquoted — each is a separate process arg)
    for arg in arg_values {
        if arg.supplied || !arg.value.is_empty() {
            parts.push(arg.value.clone());
        }
    }

    parts
}

#[cfg(test)]
mod repeated_tests {
    use super::*;
    use crate::app::RepeatInput;

    #[test]
    fn repeated_flag_axes_and_positional_values_keep_argv_groups() {
        let spec: Spec = r#"
            name "demo"
            flag "-i --include <pattern>" var=#true
            flag "--tag <tag>..."
            flag "--group... <item>..." var=#true {
                arg "<item>..." var=#true
            }
            arg "<file>..." var=#true
        "#
        .parse()
        .unwrap();
        let flags = HashMap::from([(
            String::new(),
            vec![
                (
                    "include".into(),
                    FlagValue::Repeated(vec![
                        vec![RepeatInput::supplied("one")],
                        vec![RepeatInput::omitted()],
                        vec![RepeatInput::supplied("two words")],
                    ]),
                ),
                (
                    "tag".into(),
                    FlagValue::Repeated(vec![vec![
                        RepeatInput::supplied("alpha"),
                        RepeatInput::supplied("beta"),
                    ]]),
                ),
                (
                    "group".into(),
                    FlagValue::Repeated(vec![
                        vec![RepeatInput::supplied("x"), RepeatInput::supplied("y")],
                        vec![RepeatInput::supplied("z")],
                    ]),
                ),
            ],
        )]);
        let args = vec![
            ArgValue {
                name: "file".into(),
                value: "path with spaces".into(),
                supplied: true,
                required: false,
                choices: vec![],
                help: None,
            },
            ArgValue {
                name: "file".into(),
                value: String::new(),
                supplied: false,
                required: false,
                choices: vec![],
                help: None,
            },
            ArgValue {
                name: "file".into(),
                value: String::new(),
                supplied: true,
                required: false,
                choices: vec![],
                help: None,
            },
        ];

        let parts = build_command_parts(&spec, &flags, &[], &args);
        assert_eq!(
            parts,
            [
                "demo",
                "--include",
                "one",
                "--include",
                "two words",
                "--tag",
                "alpha",
                "beta",
                "--group",
                "x",
                "y",
                "--group",
                "z",
                "path with spaces",
                ""
            ]
        );
        let preview = build_command(
            &spec,
            &flags,
            &[],
            &args,
            &LiveArgPreview {
                choice_select_index: None,
                choice_select_text: "",
                is_editing: false,
                editing_index: 0,
                editing_text: "",
            },
        );
        assert_eq!(shell_words::split(&preview).unwrap(), parts);
    }

    #[test]
    fn repeated_empty_state_does_not_add_a_preview_token() {
        let spec: Spec = "name \"demo\"\nflag \"--tag <tag>...\"".parse().unwrap();
        let flags = HashMap::from([(
            String::new(),
            vec![(
                "tag".into(),
                FlagValue::Repeated(vec![vec![RepeatInput::omitted()]]),
            )],
        )]);
        assert_eq!(
            build_command(
                &spec,
                &flags,
                &[],
                &[],
                &LiveArgPreview {
                    choice_select_index: None,
                    choice_select_text: "",
                    is_editing: false,
                    editing_index: 0,
                    editing_text: "",
                }
            ),
            "demo"
        );
    }
}
