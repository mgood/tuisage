use crate::app::{App, FlagValue};
use crate::fields::field_id;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub fn flag_json(flag: &usage::SpecFlag, value: &FlagValue) -> Value {
    match value {
        FlagValue::Bool(value) => json!(value),
        FlagValue::NegBool(value) => json!(value),
        FlagValue::Count(value) => json!(value),
        FlagValue::String(value) if value.is_empty() => Value::Null,
        FlagValue::String(value) => json!(value),
        FlagValue::EmptyString => json!(""),
        FlagValue::Repeated(groups) => {
            let values: Vec<Vec<Value>> = groups
                .iter()
                .map(|group| {
                    group
                        .iter()
                        .filter(|input| input.supplied || !input.value.is_empty())
                        .map(|input| json!(input.value))
                        .collect()
                })
                .filter(|group: &Vec<Value>| !group.is_empty())
                .collect();
            let flag_repeats = flag.var;
            let arg_repeats = flag.arg.as_ref().is_some_and(|arg| arg.var);
            if flag_repeats && arg_repeats {
                json!(values)
            } else if flag_repeats {
                json!(values
                    .into_iter()
                    .filter_map(|group| group.into_iter().next())
                    .collect::<Vec<_>>())
            } else {
                json!(values.into_iter().next().unwrap_or_default())
            }
        }
    }
}

pub fn context(app: &App, field: Option<&str>) -> Value {
    let mut fields = BTreeMap::new();
    let mut cmd = &app.spec.cmd;
    for depth in 0..=app.command_path.len() {
        let path = &app.command_path[..depth];
        let key = path.join(" ");
        if let Some(values) = app.flag_values.get(&key) {
            for flag in &cmd.flags {
                if let Some((_, value)) = values.iter().find(|(name, _)| name == &flag.name) {
                    fields.insert(
                        field_id(path, "flags", &flag.name, depth == 0 && flag.global),
                        flag_json(flag, value),
                    );
                }
            }
        }
        if depth < app.command_path.len() {
            cmd = cmd.find_subcommand(&app.command_path[depth]).unwrap();
        }
    }
    for arg in &app.arg_values {
        let id = field_id(&app.command_path, "args", &arg.name, false);
        if arg.supplied || !arg.value.is_empty() {
            if app
                .current_command()
                .args
                .iter()
                .any(|a| a.name == arg.name && a.var)
            {
                let entry = fields.entry(id).or_insert(json!([]));
                if entry.is_null() {
                    *entry = json!([]);
                }
                entry.as_array_mut().unwrap().push(json!(arg.value));
            } else {
                fields.insert(id, json!(arg.value));
            }
        } else {
            fields.entry(id).or_insert(Value::Null);
        }
    }
    let parts = app.build_command_parts();
    json!({"version": 1, "executable": parts.first(), "argv": &parts[1..], "command": app.command_path, "field": field, "fields": fields})
}

/// Run an explicit provider with JSON stdin, collecting output concurrently.
pub fn run(program: &std::path::Path, request: &Value) -> color_eyre::Result<Vec<u8>> {
    run_with_args(program, &[], request)
}

pub fn run_with_args(
    program: &std::path::Path,
    argv: &[String],
    request: &Value,
) -> color_eyre::Result<Vec<u8>> {
    run_with_timeout(program, argv, request, Duration::from_secs(5))
}

fn run_with_timeout(
    program: &std::path::Path,
    argv: &[String],
    request: &Value,
    timeout: Duration,
) -> color_eyre::Result<Vec<u8>> {
    let mut child = Command::new(program)
        .args(argv)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().unwrap();
    let input = serde_json::to_vec(request)?;
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let errors = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + timeout;
    let mut status = None;
    let status = loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        if writer.is_finished() && output.is_finished() && errors.is_finished() {
            if let Some(status) = status {
                break status;
            }
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(color_eyre::eyre::eyre!(
                "Provider '{}' timed out after {} milliseconds",
                program.display(),
                timeout.as_millis()
            ));
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    writer
        .join()
        .map_err(|_| color_eyre::eyre::eyre!("Provider input thread failed"))??;
    let output = output
        .join()
        .map_err(|_| color_eyre::eyre::eyre!("Provider output thread failed"))??;
    let errors = errors
        .join()
        .map_err(|_| color_eyre::eyre::eyre!("Provider error thread failed"))??;
    if !status.success() {
        return Err(color_eyre::eyre::eyre!(
            "Provider '{}' failed: {} {}",
            program.display(),
            status,
            String::from_utf8_lossy(&errors).trim()
        ));
    }
    Ok(output)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn repeated_provider_values_keep_each_flag_occurrence() {
        let spec: usage::Spec = r#"
            name "demo"
            flag "--group... <item>..." var=#true {
                arg "<item>..." var=#true
            }
        "#
        .parse()
        .unwrap();
        let flag = &spec.cmd.flags[0];
        let value = FlagValue::Repeated(vec![
            vec![
                crate::app::RepeatInput::supplied("a"),
                crate::app::RepeatInput::omitted(),
                crate::app::RepeatInput::supplied(""),
            ],
            vec![crate::app::RepeatInput::supplied("b")],
        ]);
        assert_eq!(flag_json(flag, &value), json!([["a", ""], ["b"]]));
    }

    #[test]
    fn exited_provider_with_open_descendant_pipes_times_out() {
        let path = std::env::temp_dir().join(format!("tuisage-provider-{}", std::process::id()));
        std::fs::write(&path, "#!/bin/sh\ncat >/dev/null\nsleep 0.3 &\nexit 0\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let started = Instant::now();
        let result = run_with_timeout(
            &path,
            &[],
            &json!({"version": 1}),
            Duration::from_millis(50),
        );
        std::fs::remove_file(path).unwrap();
        assert!(result.unwrap_err().to_string().contains("timed out"));
        assert!(started.elapsed() < Duration::from_millis(250));
    }
}
