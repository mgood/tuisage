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
/// Every token uses POSIX shell quoting; execution still uses argv directly.
pub fn build_command(
    spec: &Spec,
    flag_values: &HashMap<String, Vec<(String, FlagValue)>>,
    command_path: &[String],
    arg_values: &[ArgValue],
    preview: &LiveArgPreview,
) -> String {
    let mut args = arg_values.to_vec();
    for (i, arg) in args.iter_mut().enumerate() {
        arg.value = effective_arg_value(i, &arg_values[i], preview).to_string();
    }
    build_command_parts(spec, flag_values, command_path, &args)
        .iter()
        .map(|part| quote_posix(part))
        .collect::<Vec<_>>()
        .join(" ")
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
    for word in bin.split_whitespace() {
        parts.push(word.to_string());
    }

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
        if !arg.value.is_empty() {
            parts.push(arg.value.clone());
        }
    }

    parts
}

/// Quote one argv token for copying into a POSIX shell.
pub fn quote_posix(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_@%+=:,./-".contains(&b))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preview_tokens_round_trip_through_posix_shell() {
        let tokens = [
            "",
            "two words",
            "single'quote",
            "double\"quote",
            "世界",
            "$HOME",
            "$(printf danger)",
            "`id`",
            "line\nbreak",
            "a;b",
            "safe-token",
        ];
        let script = format!(
            "printf '%s\\0' {}",
            tokens
                .iter()
                .map(|s| quote_posix(s))
                .collect::<Vec<_>>()
                .join(" ")
        );
        let output = std::process::Command::new("sh")
            .args(["-c", &script])
            .output()
            .unwrap();
        assert!(output.status.success());
        let expected = tokens
            .iter()
            .flat_map(|s| s.as_bytes().iter().copied().chain(std::iter::once(0)))
            .collect::<Vec<_>>();
        assert_eq!(output.stdout, expected);
    }

    #[test]
    fn preview_is_quoted_execution_argv() {
        let spec: Spec = "name \"demo\"\narg \"[value]\"".parse().unwrap();
        let args = vec![ArgValue {
            name: "value".into(),
            value: "$HOME's file".into(),
            required: false,
            choices: vec![],
            help: None,
        }];
        let preview = LiveArgPreview {
            choice_select_index: None,
            choice_select_text: "",
            is_editing: false,
            editing_index: 0,
            editing_text: "",
        };
        let flags = HashMap::new();
        assert_eq!(
            build_command(&spec, &flags, &[], &args, &preview),
            build_command_parts(&spec, &flags, &[], &args)
                .iter()
                .map(|s| quote_posix(s))
                .collect::<Vec<_>>()
                .join(" ")
        );
    }
}
