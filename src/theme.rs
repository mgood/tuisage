//! Semantic color palette derived from the active terminal theme.
//!
//! Maps abstract UI roles (command, flag, arg, etc.) to concrete `Color`
//! values based on the current `ThemePalette`. This keeps all color derivation
//! logic in one place so widgets and components can reference colors by role.

use ratatui::style::Color;
use ratatui_themes::ThemePalette;

/// Semantic color palette derived from the active theme.
/// Maps abstract UI roles to concrete `Color` values.
pub struct UiColors {
    pub command: Color,
    pub flag: Color,
    pub arg: Color,
    pub value: Color,
    pub required: Color,
    pub help: Color,
    pub active_border: Color,
    pub inactive_border: Color,
    pub selected_bg: Color,
    pub hover_bg: Color,
    pub editing_bg: Color,
    pub preview_cmd: Color,
    pub choice: Color,
    pub default_val: Color,
    pub count: Color,
    pub bg: Color,
    pub bar_bg: Color,
}

impl UiColors {
    pub fn from_palette(p: &ThemePalette) -> Self {
        let bar_bg = match p.bg {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(10),
                g.saturating_add(10),
                b.saturating_add(15),
            ),
            _ => Color::Rgb(30, 30, 40),
        };

        let selected_bg = match p.selection {
            Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
            _ => Color::Rgb(40, 40, 60),
        };

        // Hover bg is a subtler version of selected_bg, blended toward the background
        let hover_bg = match (p.bg, p.selection) {
            (Color::Rgb(br, bg_g, bb), Color::Rgb(sr, sg, sb)) => Color::Rgb(
                ((br as u16 + sr as u16) / 2) as u8,
                ((bg_g as u16 + sg as u16) / 2) as u8,
                ((bb as u16 + sb as u16) / 2) as u8,
            ),
            _ => Color::Rgb(30, 30, 45),
        };

        let editing_bg = match p.selection {
            Color::Rgb(r, g, b) => Color::Rgb(
                r.saturating_add(15),
                g.saturating_sub(5),
                b.saturating_sub(10),
            ),
            _ => Color::Rgb(50, 30, 30),
        };

        Self {
            command: p.info,
            flag: p.warning,
            arg: p.success,
            value: p.accent,
            required: p.error,
            help: p.muted,
            active_border: p.accent,
            inactive_border: p.muted,
            selected_bg,
            hover_bg,
            editing_bg,
            preview_cmd: p.fg,
            choice: p.info,
            default_val: p.muted,
            count: p.secondary,
            bg: p.bg,
            bar_bg,
        }
    }
}

/// CLI appearance selection, reusing the dependency's name parser and registry.
#[derive(Clone, Copy)]
pub enum ThemeSelection {
    Fixed(ratatui_themes::ThemeName),
    Auto {
        light: ratatui_themes::ThemeName,
        dark: ratatui_themes::ThemeName,
    },
}

impl ThemeSelection {
    pub fn parse(
        name: Option<&str>,
        light: Option<ratatui_themes::ThemeName>,
        dark: Option<ratatui_themes::ThemeName>,
    ) -> color_eyre::Result<Self> {
        if name == Some("auto") {
            return match (light, dark) {
                (Some(light), Some(dark)) => Ok(Self::Auto { light, dark }),
                _ => Err(color_eyre::eyre::eyre!(
                    "--theme auto requires both --theme-light and --theme-dark"
                )),
            };
        }
        if light.is_some() || dark.is_some() {
            return Err(color_eyre::eyre::eyre!(
                "--theme-light and --theme-dark require --theme auto"
            ));
        }
        let theme = name
            .map(str::parse)
            .transpose()
            .map_err(|e| color_eyre::eyre::eyre!("{}", e))?
            .unwrap_or_default();
        Ok(Self::Fixed(theme))
    }

    pub fn is_automatic(self) -> bool {
        matches!(self, Self::Auto { .. })
    }

    pub fn initial(self) -> ratatui_themes::ThemeName {
        match self {
            Self::Fixed(name) => name,
            Self::Auto { light, dark } => {
                if system_is_light().unwrap_or(false) {
                    light
                } else {
                    dark
                }
            }
        }
    }

    pub fn watch(self) -> Option<std::sync::mpsc::Receiver<ratatui_themes::ThemeName>> {
        if !self.is_automatic() {
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || loop {
            if tx.send(self.initial()).is_err() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_secs(1));
        });
        Some(rx)
    }
}

/// System appearance takes priority. Terminal COLORFGBG is a separate fallback.
fn system_is_light() -> Option<bool> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/usr/bin/defaults")
            .args(["read", "-g", "AppleInterfaceStyle"])
            .output()
            .ok()?;
        Some(
            !output.status.success()
                || !String::from_utf8_lossy(&output.stdout)
                    .trim()
                    .eq_ignore_ascii_case("dark"),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        if cfg!(target_os = "linux") {
            if let Ok(output) = std::process::Command::new("gdbus")
                .args([
                    "call",
                    "--session",
                    "--dest",
                    "org.freedesktop.portal.Desktop",
                    "--object-path",
                    "/org/freedesktop/portal/desktop",
                    "--method",
                    "org.freedesktop.portal.Settings.Read",
                    "org.freedesktop.appearance",
                    "color-scheme",
                ])
                .output()
            {
                if output.status.success() {
                    let value = String::from_utf8_lossy(&output.stdout);
                    if value.contains("uint32 1") {
                        return Some(false);
                    }
                    if value.contains("uint32 2") {
                        return Some(true);
                    }
                }
            }
        }
        let value = std::env::var("COLORFGBG").ok()?;
        let background: u8 = value.rsplit(';').next()?.parse().ok()?;
        Some(background >= 7)
    }
}

#[cfg(test)]
mod selection_tests {
    use super::*;
    use ratatui_themes::ThemeName;

    #[test]
    fn validates_names_and_auto_pair() {
        assert!(ThemeSelection::parse(Some("missing"), None, None).is_err());
        assert!(
            ThemeSelection::parse(Some("auto"), Some(ThemeName::CatppuccinLatte), None).is_err()
        );
        assert!(ThemeSelection::parse(None, Some(ThemeName::Nord), None).is_err());
        assert_eq!(
            ThemeSelection::parse(Some("catppuccin-latte"), None, None)
                .unwrap()
                .initial(),
            ThemeName::CatppuccinLatte
        );
        assert_eq!(
            ThemeSelection::parse(Some("catppuccin_latte"), None, None)
                .unwrap()
                .initial(),
            ThemeName::CatppuccinLatte
        );
        assert_eq!(
            ThemeSelection::parse(None, None, None).unwrap().initial(),
            ThemeName::default()
        );
    }

    #[test]
    fn automatic_selection_uses_one_of_its_configured_themes() {
        let selection = ThemeSelection::parse(
            Some("auto"),
            Some(ThemeName::CatppuccinLatte),
            Some(ThemeName::Dracula),
        )
        .unwrap();
        let initial = selection.initial();

        assert!(matches!(
            initial,
            ThemeName::CatppuccinLatte | ThemeName::Dracula
        ));
        let updates = selection
            .watch()
            .expect("automatic selection watches appearance");
        assert!(matches!(
            updates
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            ThemeName::CatppuccinLatte | ThemeName::Dracula
        ));
    }
}
