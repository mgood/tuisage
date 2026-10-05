//! Optional command-owned form settings. Usage remains the command grammar.

use kdl::{KdlDocument, KdlNode, KdlValue};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub value: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub enum List {
    Fixed(Vec<Choice>),
    Provider {
        executable: PathBuf,
        argv: Vec<String>,
    },
}

#[derive(Clone, Debug, Default)]
pub struct Companion {
    pub defaults: BTreeMap<String, Value>,
    pub fields: BTreeMap<String, String>,
    pub lists: BTreeMap<String, List>,
    pub validator: Option<PathBuf>,
    pub theme: Option<String>,
    pub theme_light: Option<String>,
    pub theme_dark: Option<String>,
    pub compose: Option<bool>,
}

fn error(message: impl std::fmt::Display) -> color_eyre::Report {
    color_eyre::eyre::eyre!("{message}")
}

fn valid_selector(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn name(node: &KdlNode) -> &str {
    node.name().value()
}

fn string(node: &KdlNode, index: usize) -> color_eyre::Result<String> {
    node.get(index)
        .and_then(KdlValue::as_string)
        .map(str::to_owned)
        .ok_or_else(|| {
            error(format!(
                "{} requires string argument {}",
                name(node),
                index + 1
            ))
        })
}

fn only(
    node: &KdlNode,
    arguments: usize,
    properties: &[&str],
    children: bool,
) -> color_eyre::Result<()> {
    let mut seen = BTreeSet::new();
    let mut count = 0;
    for entry in node.entries() {
        if let Some(property) = entry.name() {
            let property = property.value();
            if !properties.contains(&property) || !seen.insert(property) {
                return Err(error(format!(
                    "{} has unknown or duplicate property {property}",
                    name(node)
                )));
            }
        } else {
            count += 1;
        }
    }
    if count != arguments || (!children && node.children().is_some()) {
        return Err(error(format!(
            "{} has invalid arguments or children",
            name(node)
        )));
    }
    Ok(())
}

fn prop_string(node: &KdlNode, key: &str) -> color_eyre::Result<Option<String>> {
    node.get(key)
        .map(|value| {
            value
                .as_string()
                .map(str::to_owned)
                .ok_or_else(|| error(format!("{}.{key} must be a string", name(node))))
        })
        .transpose()
}

fn document_path(path: &Path, value: String) -> PathBuf {
    let candidate = PathBuf::from(value);
    if candidate.is_absolute() {
        candidate
    } else {
        path.parent().unwrap().join(candidate)
    }
}

impl Companion {
    #[cfg(test)]
    pub fn parse(source: &str, path: &Path, spec: &usage::Spec) -> color_eyre::Result<Self> {
        Self::parse_with_selectors(source, path, spec, &BTreeMap::new())
    }

    pub fn parse_with_selectors(
        source: &str,
        path: &Path,
        spec: &usage::Spec,
        selectors: &BTreeMap<String, PathBuf>,
    ) -> color_eyre::Result<Self> {
        let doc: KdlDocument = source.parse()?;
        let mut result = Self::default();
        let mut seen = BTreeSet::new();
        let mut version = false;
        for node in doc.nodes() {
            match name(node) {
                "version" => {
                    only(node, 1, &[], false)?;
                    if version || node.get(0).and_then(KdlValue::as_integer) != Some(1) {
                        return Err(error("companion requires exactly one version 1 node"));
                    }
                    version = true;
                }
                "mode" => {
                    only(node, 1, &[], false)?;
                    if !seen.insert("mode") {
                        return Err(error("duplicate mode"));
                    }
                    result.compose = Some(match string(node, 0)?.as_str() {
                        "compose" => true,
                        "execute" => false,
                        _ => return Err(error("mode must be compose or execute")),
                    });
                }
                "theme" => {
                    only(node, 1, &["light", "dark"], false)?;
                    if !seen.insert("theme") {
                        return Err(error("duplicate theme"));
                    }
                    result.theme = Some(string(node, 0)?);
                    result.theme_light = prop_string(node, "light")?;
                    result.theme_dark = prop_string(node, "dark")?;
                }
                "validate" => {
                    only(node, 1, &[], false)?;
                    if !seen.insert("validate") {
                        return Err(error("duplicate validate"));
                    }
                    result.validator = Some(document_path(path, string(node, 0)?));
                }
                "default" => {
                    only(node, 1, &["json", "locked"], false)?;
                    let id = string(node, 0)?;
                    let raw = prop_string(node, "json")?
                        .ok_or_else(|| error(format!("default {id} requires json")))?;
                    let locked = match node.get("locked") {
                        Some(value) => value
                            .as_bool()
                            .ok_or_else(|| error(format!("default {id} locked must be boolean")))?,
                        None => false,
                    };
                    let value =
                        json!({"value": serde_json::from_str::<Value>(&raw)?, "locked": locked});
                    if result.defaults.insert(id.clone(), value).is_some() {
                        return Err(error(format!("duplicate default {id}")));
                    }
                }
                "list" => {
                    only(node, 1, &[], true)?;
                    let id = string(node, 0)?;
                    let children = node
                        .children()
                        .ok_or_else(|| error(format!("list {id} requires children")))?;
                    let mut choices = Vec::new();
                    let mut provider = None;
                    let mut selector_list = false;
                    for child in children.nodes() {
                        match name(child) {
                            "choice" => {
                                only(child, 1, &["description"], false)?;
                                choices.push(Choice {
                                    value: string(child, 0)?,
                                    description: prop_string(child, "description")?,
                                });
                            }
                            "provider" => {
                                if provider.is_some() {
                                    return Err(error(format!("duplicate provider in {id}")));
                                }
                                let args = child
                                    .entries()
                                    .iter()
                                    .filter(|entry| entry.name().is_none())
                                    .count();
                                if args == 0 {
                                    return Err(error(format!(
                                        "provider in {id} needs executable"
                                    )));
                                }
                                only(child, args, &[], false)?;
                                provider = Some(List::Provider {
                                    executable: document_path(path, string(child, 0)?),
                                    argv: (1..args)
                                        .map(|i| string(child, i))
                                        .collect::<Result<_, _>>()?,
                                });
                            }
                            "selectors" => {
                                only(child, 0, &[], false)?;
                                if selector_list {
                                    return Err(error(format!("duplicate selectors in {id}")));
                                }
                                selector_list = true;
                            }
                            other => return Err(error(format!("unknown list child {other}"))),
                        }
                    }
                    if (provider.is_some() && !choices.is_empty())
                        || (selector_list && (provider.is_some() || !choices.is_empty()))
                    {
                        return Err(error(format!("list {id} mixes choice sources")));
                    }
                    let list = if selector_list {
                        List::Fixed(
                            selectors
                                .keys()
                                .map(|value| Choice {
                                    value: value.clone(),
                                    description: None,
                                })
                                .collect(),
                        )
                    } else {
                        provider.unwrap_or(List::Fixed(choices))
                    };
                    if result.lists.insert(id.clone(), list).is_some() {
                        return Err(error(format!("duplicate list {id}")));
                    }
                }
                "field" => {
                    only(node, 1, &["list"], false)?;
                    let id = string(node, 0)?;
                    let list = prop_string(node, "list")?
                        .ok_or_else(|| error(format!("field {id} requires list")))?;
                    if result.fields.insert(id.clone(), list).is_some() {
                        return Err(error(format!("duplicate field {id}")));
                    }
                }
                other => return Err(error(format!("unknown companion node {other}"))),
            }
        }
        if !version {
            return Err(error("companion requires version 1"));
        }
        let ids: BTreeSet<_> = crate::fields::fields(spec)
            .into_iter()
            .map(|field| field.id)
            .collect();
        for (field, list) in &result.fields {
            if !ids.contains(field) || !result.lists.contains_key(list) {
                return Err(error(format!(
                    "field {field} refers to unknown Usage field or list {list}"
                )));
            }
        }
        let defaults = Value::Object(result.defaults.clone().into_iter().collect::<Map<_, _>>());
        crate::defaults::parse(&defaults.to_string(), spec)?;
        if let Some(ref theme) = result.theme {
            let light = result
                .theme_light
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|e| error(format!("invalid light theme: {e}")))?;
            let dark = result
                .theme_dark
                .as_deref()
                .map(str::parse)
                .transpose()
                .map_err(|e| error(format!("invalid dark theme: {e}")))?;
            crate::theme::ThemeSelection::parse(Some(theme), light, dark)?;
        } else if result.theme_light.is_some() || result.theme_dark.is_some() {
            return Err(error("theme light/dark requires theme auto"));
        }
        Ok(result)
    }

    pub fn load_with_selectors(
        path: &Path,
        spec: &usage::Spec,
        selectors: &BTreeMap<String, PathBuf>,
    ) -> color_eyre::Result<Self> {
        let source = std::fs::read_to_string(path)?;
        Self::parse_with_selectors(&source, path, spec, selectors)
            .map_err(|cause| error(format!("invalid companion {}: {cause}", path.display())))
    }
}

pub fn executable(command: &str) -> Option<PathBuf> {
    let command = Path::new(command);
    if command.components().count() > 1 {
        return command.canonicalize().ok();
    }
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        if !directory.is_absolute() {
            continue;
        }
        let candidate = directory.join(command);
        if candidate.is_file() {
            return candidate.canonicalize().ok();
        }
    }
    None
}

/// Return the first existing document. A malformed document never falls through.
pub fn discover(
    command: &Path,
    explicit: Option<&Path>,
    selector: Option<&str>,
) -> color_eyre::Result<Option<PathBuf>> {
    let name = command
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| error("command has no filename"))?;
    if let Some(value) = selector {
        if !valid_selector(value) {
            return Err(error("companion selector must contain only letters, digits, dots, underscores, or hyphens"));
        }
    }
    let filename = match selector {
        Some(value) => format!("{name}.{value}.tuisage.kdl"),
        None => format!("{name}.tuisage.kdl"),
    };
    let mut locations = directories(command);
    let explicit_file = match explicit {
        Some(path) if path.is_dir() => {
            locations.insert(0, path.to_owned());
            None
        }
        path => path,
    };
    select(explicit_file, &locations, &filename)
}

fn directories(command: &Path) -> Vec<PathBuf> {
    directory_candidates(
        command,
        if cfg!(target_os = "macos") {
            Platform::MacOS
        } else if cfg!(windows) {
            Platform::Windows
        } else {
            Platform::Linux
        },
        std::env::var_os("HOME").map(PathBuf::from),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        std::env::var_os("XDG_DATA_DIRS"),
        std::env::var_os("LOCALAPPDATA").map(PathBuf::from),
        std::env::var_os("PROGRAMDATA").map(PathBuf::from),
    )
}

#[derive(Clone, Copy)]
enum Platform {
    MacOS,
    Linux,
    Windows,
}

fn directory_candidates(
    command: &Path,
    platform: Platform,
    home: Option<PathBuf>,
    data_home: Option<PathBuf>,
    data_dirs: Option<std::ffi::OsString>,
    local_app_data: Option<PathBuf>,
    program_data: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    let mut shared = Vec::new();
    let home = home.filter(|path| path.is_absolute());
    match platform {
        Platform::MacOS => {
            if let Some(ref home) = home {
                paths.push(home.join("Library/Application Support/TuiSage/commands"));
            }
        }
        Platform::Linux => {
            if let Some(root) = data_home.filter(|path| path.is_absolute()) {
                paths.push(root.join("tuisage/commands"));
            } else if let Some(ref home) = home {
                paths.push(home.join(".local/share/tuisage/commands"));
            }
        }
        Platform::Windows => {
            if let Some(root) = local_app_data.filter(|path| path.is_absolute()) {
                paths.push(root.join("TuiSage/commands"));
            }
        }
    }
    if command.is_absolute() {
        paths.push(command.parent().unwrap().to_owned());
    }
    match platform {
        Platform::MacOS => {
            shared.push(PathBuf::from(
                "/Library/Application Support/TuiSage/commands",
            ));
        }
        Platform::Linux => {
            let dirs = data_dirs
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            for dir in std::env::split_paths(&dirs) {
                if dir.is_absolute() {
                    shared.push(dir.join("tuisage/commands"));
                }
            }
        }
        Platform::Windows => {
            if let Some(root) = program_data.filter(|path| path.is_absolute()) {
                shared.push(root.join("TuiSage/commands"));
            }
        }
    }
    paths.extend(shared);
    paths
}

fn select(
    explicit: Option<&Path>,
    directories: &[PathBuf],
    filename: &str,
) -> color_eyre::Result<Option<PathBuf>> {
    if let Some(path) = explicit {
        if !path.is_file() {
            return Err(error(format!("companion not found: {}", path.display())));
        }
        return Ok(Some(path.to_owned()));
    }
    Ok(directories
        .iter()
        .map(|dir| dir.join(filename))
        .find(|path| path.is_file()))
}

pub fn discover_selectors_in(
    command: &Path,
    explicit: Option<&Path>,
) -> color_eyre::Result<BTreeMap<String, PathBuf>> {
    let name = command
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| error("command has no filename"))?;
    let mut locations = directories(command);
    if let Some(path) = explicit.filter(|path| path.is_dir()) {
        locations.insert(0, path.to_owned());
    }
    Ok(collect_selectors(name, &locations))
}

fn collect_selectors(name: &str, directories: &[PathBuf]) -> BTreeMap<String, PathBuf> {
    let prefix = format!("{name}.");
    let suffix = ".tuisage.kdl";
    let mut found = BTreeMap::new();
    for directory in directories {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(filename) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(selector) = filename
                .strip_prefix(&prefix)
                .and_then(|value| value.strip_suffix(suffix))
            else {
                continue;
            };
            if valid_selector(selector) && path.is_file() {
                found.entry(selector.to_owned()).or_insert(path);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> usage::Spec {
        "name \"demo\"\nflag \"--backend <backend>\" global=#true\ncmd \"run\" { arg \"[target]\"; }"
            .parse().unwrap()
    }

    #[test]
    #[cfg(unix)]
    fn invalid_user_roots_never_introduce_working_directory_lookup() {
        let command = std::env::temp_dir().join("tuisage-test/bin/demo");
        for platform in [Platform::Linux, Platform::MacOS, Platform::Windows] {
            for home in [None, Some(PathBuf::new()), Some(PathBuf::from("relative"))] {
                let paths = directory_candidates(
                    &command,
                    platform,
                    home,
                    None,
                    None,
                    Some(PathBuf::from("relative-local")),
                    Some(PathBuf::from("relative-shared")),
                );
                assert!(paths.iter().all(|path| path.is_absolute()), "{paths:?}");
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn linux_invalid_data_home_uses_absolute_home_fallback() {
        for data_home in [None, Some(PathBuf::new()), Some(PathBuf::from("relative"))] {
            let command = std::env::temp_dir().join("tuisage-test/bin/demo");
            let paths = directory_candidates(
                &command,
                Platform::Linux,
                Some(PathBuf::from("/home/user")),
                data_home,
                Some("relative:/shared".into()),
                None,
                None,
            );
            assert_eq!(
                paths,
                vec![
                    PathBuf::from("/home/user/.local/share/tuisage/commands"),
                    command.parent().unwrap().to_owned(),
                    PathBuf::from("/shared/tuisage/commands")
                ]
            );
        }
    }

    #[test]
    #[cfg(unix)]
    fn platform_data_directories_keep_user_sidecar_shared_order() {
        let command = std::env::temp_dir().join("tuisage-test/bin/demo");
        let linux = directory_candidates(
            &command,
            Platform::Linux,
            None,
            Some(PathBuf::from("/data")),
            Some("".into()),
            None,
            None,
        );
        assert_eq!(
            linux,
            vec![
                PathBuf::from("/data/tuisage/commands"),
                command.parent().unwrap().to_owned(),
                PathBuf::from("/usr/local/share/tuisage/commands"),
                PathBuf::from("/usr/share/tuisage/commands")
            ]
        );
        let mac = directory_candidates(
            &command,
            Platform::MacOS,
            Some(PathBuf::from("/Users/user")),
            Some(PathBuf::from("/ignored")),
            Some("/ignored".into()),
            None,
            None,
        );
        assert_eq!(
            mac,
            vec![
                PathBuf::from("/Users/user/Library/Application Support/TuiSage/commands"),
                command.parent().unwrap().to_owned(),
                PathBuf::from("/Library/Application Support/TuiSage/commands")
            ]
        );
    }

    #[test]
    fn windows_data_directories_keep_user_sidecar_shared_order() {
        let root = std::env::temp_dir().join("tuisage-windows-paths");
        let local = root.join("AppData/Local");
        let shared = root.join("ProgramData");
        let command = root.join("bin/demo.exe");
        let windows = directory_candidates(
            &command,
            Platform::Windows,
            Some(root.join("ignored-home")),
            Some(root.join("ignored-xdg")),
            Some("ignored".into()),
            Some(local.clone()),
            Some(shared.clone()),
        );
        assert_eq!(
            windows,
            vec![
                local.join("TuiSage/commands"),
                command.parent().unwrap().to_owned(),
                shared.join("TuiSage/commands")
            ]
        );

        let no_roots = directory_candidates(
            &command,
            Platform::Windows,
            None,
            None,
            None,
            Some(PathBuf::from("relative-local")),
            Some(PathBuf::from("relative-shared")),
        );
        assert_eq!(no_roots, vec![command.parent().unwrap().to_owned()]);
    }

    #[test]
    fn parses_lists_defaults_and_command_settings() {
        let source = r#"version 1
mode "compose"
theme "auto" light="catppuccin-latte" dark="tokyo-night"
validate "check.py"
default "global/flags/backend" json="\"cloud\"" locked=#true
list "backends" { choice "cloud" description="Cloud backend" }
field "global/flags/backend" list="backends"
list "targets" { provider "choices.py" "targets" }
field "commands/run/args/target" list="targets"
"#;
        let path = Path::new("/tmp/demo.tuisage.kdl");
        let doc = Companion::parse(source, path, &spec()).unwrap();
        assert_eq!(doc.compose, Some(true));
        assert_eq!(doc.validator.as_deref(), Some(Path::new("/tmp/check.py")));
        assert_eq!(doc.defaults["global/flags/backend"]["locked"], true);
        assert_eq!(doc.fields["commands/run/args/target"], "targets");
        assert!(
            matches!(&doc.lists["targets"], List::Provider { executable, argv }
            if executable == Path::new("/tmp/choices.py") && argv == &["targets"])
        );
    }

    #[test]
    fn rejects_unknown_fields_and_nodes() {
        let path = Path::new("/tmp/demo.tuisage.kdl");
        assert!(Companion::parse(
            "version 1\nfield \"missing\" list=\"x\"\nlist \"x\" {}",
            path,
            &spec()
        )
        .is_err());
        assert!(Companion::parse("version 1\nunsupported \"x\"", path, &spec()).is_err());
        assert!(Companion::parse("version 1\nversion 1", path, &spec()).is_err());
    }

    #[test]
    fn selector_list_uses_discovered_documents() {
        let path = Path::new("/tmp/demo.orbitron.tuisage.kdl");
        let selectors = BTreeMap::from([
            ("orbitron".to_string(), path.to_owned()),
            (
                "proxmoxlxc".to_string(),
                PathBuf::from("/tmp/demo.proxmoxlxc.tuisage.kdl"),
            ),
        ]);
        let source = "version 1\nlist \"blueprints\" { selectors }\nfield \"commands/run/args/target\" list=\"blueprints\"";
        let doc = Companion::parse_with_selectors(source, path, &spec(), &selectors).unwrap();
        assert!(matches!(&doc.lists["blueprints"], List::Fixed(values)
            if values.iter().map(|choice| choice.value.as_str()).collect::<Vec<_>>() == ["orbitron", "proxmoxlxc"]));
        assert!(Companion::parse_with_selectors(
            "version 1\nlist \"bad\" { selectors; choice \"x\" }",
            path,
            &spec(),
            &selectors
        )
        .is_err());
    }

    #[test]
    fn cascade_selects_one_document_in_order() {
        let temp = tempfile::tempdir().unwrap();
        let user_dir = temp.path().join("user");
        let shared_dir = temp.path().join("shared");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::create_dir_all(&shared_dir).unwrap();
        let user = user_dir.join("demo.tuisage.kdl");
        let shared = shared_dir.join("demo.tuisage.kdl");
        let explicit = temp.path().join("explicit.kdl");
        let binary = temp.path().join("bin/demo");
        std::fs::create_dir_all(binary.parent().unwrap()).unwrap();
        let sidecar = binary.parent().unwrap().join("demo.tuisage.kdl");
        for path in [&user, &shared, &explicit, &sidecar] {
            std::fs::write(path, "version 1\n").unwrap();
        }
        let directories = vec![user_dir, binary.parent().unwrap().to_owned(), shared_dir];
        let choose =
            |override_path| select(override_path, &directories, "demo.tuisage.kdl").unwrap();
        assert_eq!(choose(Some(&explicit)), Some(explicit.clone()));
        assert_eq!(choose(None), Some(user.clone()));
        std::fs::remove_file(&user).unwrap();
        assert_eq!(choose(None), Some(sidecar.clone()));
        std::fs::remove_file(&sidecar).unwrap();
        assert_eq!(choose(None), Some(shared));
    }

    #[test]
    fn finds_all_selectors_with_per_selector_cascade_precedence() {
        let temp = tempfile::tempdir().unwrap();
        let user = temp.path().join("user");
        let sidecar = temp.path().join("sidecar");
        let shared = temp.path().join("shared");
        for dir in [&user, &sidecar, &shared] {
            std::fs::create_dir_all(dir).unwrap();
        }
        let user_orbitron = user.join("demo.orbitron.tuisage.kdl");
        let sidecar_orbitron = sidecar.join("demo.orbitron.tuisage.kdl");
        let sidecar_proxmox = sidecar.join("demo.proxmoxlxc.tuisage.kdl");
        let shared_other = shared.join("demo.other.tuisage.kdl");
        for path in [
            &user_orbitron,
            &sidecar_orbitron,
            &sidecar_proxmox,
            &shared_other,
            &sidecar.join("demo.tuisage.kdl"),
            &sidecar.join("demo.bad name.tuisage.kdl"),
        ] {
            std::fs::write(path, "version 1\n").unwrap();
        }
        let result = collect_selectors("demo", &[user, sidecar, shared]);
        assert_eq!(result.len(), 3);
        assert_eq!(result["orbitron"], user_orbitron);
        assert_eq!(result["proxmoxlxc"], sidecar_proxmox);
        assert_eq!(result["other"], shared_other);
    }

    #[test]
    fn selector_selects_only_its_named_companion() {
        let temp = tempfile::tempdir().unwrap();
        let binary = temp.path().join("demo");
        std::fs::write(&binary, "").unwrap();
        let generic = temp.path().join("demo.tuisage.kdl");
        let one = temp.path().join("demo.one.tuisage.kdl");
        std::fs::write(&generic, "version 1\n").unwrap();
        std::fs::write(&one, "version 1\n").unwrap();
        assert_eq!(discover(&binary, None, Some("one")).unwrap(), Some(one));
        assert_eq!(discover(&binary, None, Some("two")).unwrap(), None);
        assert!(discover(&binary, None, Some("../other")).is_err());
        assert_eq!(discover(&binary, None, None).unwrap(), Some(generic));
    }
}
