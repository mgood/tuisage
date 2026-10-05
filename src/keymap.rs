use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crossterm::event::{KeyCode, KeyEvent, KeyEventState, KeyModifiers};
use serde::Deserialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NamedAction {
    Submit,
    Cancel,
    NextField,
    PreviousField,
    Unbound,
}

impl NamedAction {
    fn parse(value: &str) -> color_eyre::Result<Self> {
        match value {
            "submit" => Ok(Self::Submit),
            "cancel" => Ok(Self::Cancel),
            "next-field" => Ok(Self::NextField),
            "previous-field" => Ok(Self::PreviousField),
            "unbound" => Ok(Self::Unbound),
            _ => Err(color_eyre::eyre::eyre!("Unknown keymap action '{value}'")),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Key {
    code: KeyCode,
    modifiers: KeyModifiers,
    keypad: bool,
}

impl Key {
    fn parse(value: &str) -> color_eyre::Result<Self> {
        let mut parts: Vec<_> = value.split('+').collect();
        let code = parts.pop().unwrap_or_default();
        let mut modifiers = KeyModifiers::NONE;
        for part in parts {
            modifiers |= match part.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => KeyModifiers::CONTROL,
                "alt" => KeyModifiers::ALT,
                "shift" => KeyModifiers::SHIFT,
                "cmd" | "super" => KeyModifiers::SUPER,
                _ => {
                    return Err(color_eyre::eyre::eyre!(
                        "Unsupported key modifier in '{value}'"
                    ))
                }
            };
        }

        let keypad = code.eq_ignore_ascii_case("keypad-enter");
        let code = match code.to_ascii_lowercase().as_str() {
            "enter" | "return" | "keypad-enter" => KeyCode::Enter,
            "escape" | "esc" => KeyCode::Esc,
            "tab" => KeyCode::Tab,
            "backtab" => KeyCode::BackTab,
            "backspace" => KeyCode::Backspace,
            "left" => KeyCode::Left,
            "right" => KeyCode::Right,
            "up" => KeyCode::Up,
            "down" => KeyCode::Down,
            "space" => KeyCode::Char(' '),
            _ if code.chars().count() == 1 => KeyCode::Char(code.chars().next().unwrap()),
            _ => return Err(color_eyre::eyre::eyre!("Unsupported key syntax '{value}'")),
        };

        Ok(Self {
            code,
            modifiers,
            keypad,
        })
    }

    fn matches(self, event: KeyEvent) -> bool {
        self.code == event.code
            && self.modifiers == event.modifiers
            && (self.code != KeyCode::Enter
                || self.keypad == event.state.contains(KeyEventState::KEYPAD))
    }
}

#[derive(Clone)]
pub struct Keymap {
    bindings: Vec<(Key, NamedAction)>,
}

impl Default for Keymap {
    fn default() -> Self {
        let bindings = [
            ("ctrl+r", NamedAction::Submit),
            ("ctrl+c", NamedAction::Cancel),
            ("q", NamedAction::Cancel),
            ("tab", NamedAction::NextField),
            ("backtab", NamedAction::PreviousField),
        ];
        Self {
            bindings: bindings
                .into_iter()
                .map(|(key, action)| (Key::parse(key).unwrap(), action))
                .collect(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct KeymapFile {
    bindings: BTreeMap<String, String>,
}

impl Keymap {
    pub fn resolve(&self, event: KeyEvent) -> Option<NamedAction> {
        self.bindings
            .iter()
            .rev()
            .find(|(key, _)| key.matches(event))
            .map(|(_, action)| *action)
    }

    fn apply(&mut self, text: &str) -> color_eyre::Result<()> {
        let file: KeymapFile = toml::from_str(text)?;
        let mut seen = Vec::new();
        for (key, action) in file.bindings {
            let key = Key::parse(&key)?;
            if seen.contains(&key) {
                return Err(color_eyre::eyre::eyre!("Duplicate key binding '{key:?}'"));
            }
            seen.push(key);
            self.bindings.push((key, NamedAction::parse(&action)?));
        }
        Ok(())
    }

    pub fn load(explicit: Option<&Path>) -> color_eyre::Result<Self> {
        let path = explicit.map(Path::to_owned).or_else(|| {
            default_path(
                std::env::var_os("XDG_CONFIG_HOME").as_deref(),
                std::env::var_os("HOME").as_deref(),
            )
        });
        let required = explicit.is_some();
        let mut keymap = Self::default();
        if let Some(path) = path {
            match std::fs::read_to_string(&path) {
                Ok(text) => keymap.apply(&text).map_err(|error| {
                    color_eyre::eyre::eyre!("Keymap '{}': {error}", path.display())
                })?,
                Err(error) if !required && error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return Err(color_eyre::eyre::eyre!(
                        "Cannot read keymap '{}': {error}",
                        path.display()
                    ));
                }
            }
        }
        Ok(keymap)
    }
}

fn default_path(
    xdg_config_home: Option<&std::ffi::OsStr>,
    home: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    let base = xdg_config_home
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|path| PathBuf::from(path).join(".config")))?;
    Some(base.join("tuisage/keymap.toml"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enter_and_keypad_enter_are_not_builtin_submit_bindings() {
        let keymap = Keymap::default();
        assert_eq!(
            keymap.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            None
        );
        let mut keypad = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        keypad.state = KeyEventState::KEYPAD;
        assert_eq!(keymap.resolve(keypad), None);
    }

    #[test]
    fn enter_and_keypad_enter_can_be_enabled_by_configuration() {
        let mut keymap = Keymap::default();
        keymap
            .apply(
                "[bindings]\n\"enter\" = \"submit\"\n\
                 \"keypad-enter\" = \"submit\"",
            )
            .unwrap();
        assert_eq!(
            keymap.resolve(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE)),
            Some(NamedAction::Submit)
        );
        let mut keypad = KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE);
        keypad.state = KeyEventState::KEYPAD;
        assert_eq!(keymap.resolve(keypad), Some(NamedAction::Submit));
    }

    #[test]
    fn overrides_and_unbinding_apply_to_individual_keys() {
        let mut keymap = Keymap::default();
        keymap
            .apply("[bindings]\n\"ctrl+r\" = \"unbound\"\n\"alt+s\" = \"submit\"")
            .unwrap();
        assert_eq!(
            keymap.resolve(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL)),
            Some(NamedAction::Unbound)
        );
        assert_eq!(
            keymap.resolve(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT)),
            Some(NamedAction::Submit)
        );
        assert_eq!(
            keymap.resolve(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE)),
            Some(NamedAction::Cancel)
        );
        assert!(keymap
            .apply("[bindings]\n\"bad+key\" = \"submit\"")
            .is_err());
        assert!(keymap
            .apply("[bindings]\n\"alt+s\" = \"invented\"")
            .is_err());
        assert!(keymap.apply("bad = [").is_err());
    }

    #[test]
    fn default_path_uses_xdg_or_home_fallback() {
        assert_eq!(
            default_path(Some(std::ffi::OsStr::new("/config")), None),
            Some(PathBuf::from("/config/tuisage/keymap.toml"))
        );
        assert_eq!(
            default_path(
                Some(std::ffi::OsStr::new("")),
                Some(std::ffi::OsStr::new("/home/user"))
            ),
            Some(PathBuf::from("/home/user/.config/tuisage/keymap.toml"))
        );
    }

    #[test]
    fn missing_explicit_keymap_is_an_error() {
        let path = std::env::temp_dir().join(format!(
            "missing-tuisage-keymap-{}.toml",
            std::process::id()
        ));
        assert!(Keymap::load(Some(&path)).is_err());
    }
}
