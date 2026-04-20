//! Help bar widget — context-sensitive keyboard shortcut display.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::Style,
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::theme::UiColors;

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
        let theme_indicator_len = theme_indicator.len() as u16;
        let indicator_x = area.x + area.width.saturating_sub(theme_indicator_len);
        Rect::new(indicator_x, area.y, theme_indicator_len, 1)
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
            total_len += (kb.key.chars().count() + 1 + kb.desc.chars().count()) as u16;
        }

        (spans, total_len)
    }
}

impl Widget for HelpBar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme_indicator = format!("T: [{}] ", self.theme_display);
        let theme_indicator_len = theme_indicator.len() as u16;

        let (mut spans, keybinds_len) = self.styled_keybind_spans();
        let padding_len = area.width.saturating_sub(keybinds_len + theme_indicator_len);
        let padding = " ".repeat(padding_len as usize);

        spans.push(Span::styled(padding, Style::default()));
        spans.push(Span::styled(
            theme_indicator,
            Style::default().fg(self.colors.active_border).italic(),
        ));

        let paragraph = Paragraph::new(Line::from(spans))
            .style(Style::default().bg(self.colors.bar_bg));

        paragraph.render(area, buf);
    }
}
