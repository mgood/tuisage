//! Help bar widget — context-sensitive keyboard shortcut display.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::theme::UiColors;
use unicode_width::UnicodeWidthStr;

/// A single keyboard shortcut entry: a key (e.g. `"↑↓"`) and its description
/// (e.g. `"navigate"`).
pub struct Keybind<'a> {
    pub key: &'a str,
    pub desc: &'a str,
}

/// A widget that renders the context-sensitive help/status bar.
///
/// Shows keyboard shortcuts for the current mode on the left and
/// the active theme indicator on the right.
pub struct HelpBar<'a> {
    /// Structured key/description pairs to display.
    pub keybinds: &'a [Keybind<'a>],
    /// Theme display name for the right-aligned indicator.
    pub theme_display: &'a str,
    pub colors: &'a UiColors,
}

impl<'a> HelpBar<'a> {
    pub fn new(keybinds: &'a [Keybind<'a>], theme_display: &'a str, colors: &'a UiColors) -> Self {
        Self {
            keybinds,
            theme_display,
            colors,
        }
    }

    /// Returns the Rect where the theme indicator is rendered,
    /// for use in mouse click hit-testing.
    pub fn theme_indicator_rect(&self, area: Rect) -> Rect {
        let theme_indicator = format!("T: [{}] ", self.theme_display);
        let theme_indicator_len = (theme_indicator.width() as u16).min(area.width);
        let indicator_x = area.x + area.width.saturating_sub(theme_indicator_len);
        Rect::new(indicator_x, area.y, theme_indicator_len, 1)
    }

    /// Returns visible help-bar areas and the keyboard events they represent.
    pub fn keybind_regions(&self, area: Rect) -> Vec<(Rect, crossterm::event::KeyEvent)> {
        let limit = self.theme_indicator_rect(area).x;
        let mut x = area.x.saturating_add(1);
        let mut regions = Vec::new();

        for (index, keybind) in self.keybinds.iter().enumerate() {
            if index > 0 {
                x = x.saturating_add(2);
            }
            let key_width = keybind.key.width() as u16;
            let item_width = key_width.saturating_add(1 + keybind.desc.width() as u16);
            if x.saturating_add(item_width) > limit {
                break;
            }

            let parts = key_events(keybind.key);
            if parts.len() == 1 && parts[0].0 == 0 && parts[0].1 == key_width {
                regions.push((Rect::new(x, area.y, item_width, 1), parts[0].2));
            } else {
                for (offset, width, event) in parts {
                    regions.push((Rect::new(x + offset, area.y, width, 1), event));
                }
            }
            x = x.saturating_add(item_width);
        }
        regions
    }

    /// Build styled spans for the keybinds.
    ///
    /// Each entry's key is rendered in the high-contrast `active_border` color
    /// and its description in the subdued `help` color.  Entries are separated
    /// by two spaces.  Returns the spans and their total display width.
    fn styled_keybind_spans(&self) -> (Vec<Span<'a>>, u16) {
        let mut spans: Vec<Span<'a>> = vec![Span::raw(" ")];
        let mut total_len: u16 = 1; // leading space

        for (i, kb) in self.keybinds.iter().enumerate() {
            if i > 0 {
                spans.push(Span::raw("  "));
                total_len += 2;
            }
            spans.push(Span::styled(kb.key, Style::default().fg(self.colors.active_border)));
            spans.push(Span::raw(" "));
            spans.push(Span::styled(kb.desc, Style::default().fg(self.colors.help)));
            total_len += (kb.key.width() + 1 + kb.desc.width()) as u16;
        }

        (spans, total_len)
    }
}

impl Widget for HelpBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme_indicator = format!("T: [{}] ", self.theme_display);
        let theme_indicator_len = (theme_indicator.width() as u16).min(area.width);

        let (spans, keybinds_len) = self.styled_keybind_spans();
        let padding_len = area.width.saturating_sub(keybinds_len + theme_indicator_len);
        let padding = " ".repeat(padding_len as usize);
        let mut spans = spans;
        spans.push(Span::styled(padding, Style::default()));
        let theme_area = self.theme_indicator_rect(area);
        let key_area = Rect::new(area.x, area.y, theme_area.x.saturating_sub(area.x), 1);
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(self.colors.bar_bg))
            .render(key_area, buf);
        Paragraph::new(theme_indicator)
            .style(
                Style::default()
                    .fg(self.colors.active_border)
                    .bg(self.colors.bar_bg)
                    .italic(),
            )
            .render(theme_area, buf);
    }
}

fn key_events(key: &str) -> Vec<(u16, u16, crossterm::event::KeyEvent)> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let event = |code, modifiers| KeyEvent::new(code, modifiers);
    let simple = match key {
        "⇥" => Some(event(KeyCode::Tab, KeyModifiers::NONE)),
        "⏎" => Some(event(KeyCode::Enter, KeyModifiers::NONE)),
        "⌫" => Some(event(KeyCode::Backspace, KeyModifiers::NONE)),
        "^u" => Some(event(KeyCode::Char('u'), KeyModifiers::CONTROL)),
        "^r" => Some(event(KeyCode::Char('r'), KeyModifiers::CONTROL)),
        "q" => Some(event(KeyCode::Char('q'), KeyModifiers::NONE)),
        "Esc" => Some(event(KeyCode::Esc, KeyModifiers::NONE)),
        "/" => Some(event(KeyCode::Char('/'), KeyModifiers::NONE)),
        _ => None,
    };
    if let Some(event) = simple {
        return vec![(0, key.width() as u16, event)];
    }

    if key == "⏎/Space" {
        return vec![
            (
                0,
                "⏎".width() as u16,
                event(KeyCode::Enter, KeyModifiers::NONE),
            ),
            (
                "⏎/".width() as u16,
                "Space".width() as u16,
                event(KeyCode::Char(' '), KeyModifiers::NONE),
            ),
        ];
    }

    let mut offset = 0;
    let mut events = Vec::new();
    for character in key.chars() {
        let width = character.to_string().width() as u16;
        let code = match character {
            '↑' => Some(KeyCode::Up),
            '↓' => Some(KeyCode::Down),
            'j' => Some(KeyCode::Char('j')),
            'k' => Some(KeyCode::Char('k')),
            _ => None,
        };
        if let Some(code) = code {
            events.push((offset, width, event(code, KeyModifiers::NONE)));
        }
        offset += width;
    }
    events
}
