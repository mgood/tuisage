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
            (Color::Rgb(br, bg_g, bb), Color::Rgb(sr, sg, sb)) => {
                Color::Rgb(
                    ((br as u16 + sr as u16) / 2) as u8,
                    ((bg_g as u16 + sg as u16) / 2) as u8,
                    ((bb as u16 + sb as u16) / 2) as u8,
                )
            }
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
