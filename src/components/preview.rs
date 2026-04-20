//! Command preview widget — colorized assembled command display.

use std::collections::HashSet;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Padding, Paragraph, Widget, Wrap},
};

use crate::theme::UiColors;

/// A widget that renders the assembled command preview with colorized tokens.
///
/// Displays the command string in a bordered block, with syntax-aware coloring
/// for the binary name, subcommands, flags, and positional arguments.
pub struct CommandPreview<'a> {
    /// The full built command string.
    pub command: &'a str,
    /// Binary name from the spec.
    pub bin: &'a str,
    /// Subcommand names in the current command path.
    pub subcommands: &'a [String],
    /// Whether the preview panel currently has focus.
    pub is_focused: bool,
    pub colors: &'a UiColors,
}

impl<'a> CommandPreview<'a> {
    pub fn new(
        command: &'a str,
        bin: &'a str,
        subcommands: &'a [String],
        is_focused: bool,
        colors: &'a UiColors,
    ) -> Self {
        Self {
            command,
            bin,
            subcommands,
            is_focused,
            colors,
        }
    }

    /// Colorize the command string by categorizing each token.
    fn colorize(&self, bold: Modifier) -> Vec<Span<'static>> {
        let subcommand_names: HashSet<&str> =
            self.subcommands.iter().map(|s| s.as_str()).collect();

        let tokens: Vec<&str> = self.command.split_whitespace().collect();
        let mut spans = Vec::new();
        let mut i = 0;
        let mut expect_flag_value = false;

        while i < tokens.len() {
            if i > 0 {
                spans.push(Span::raw(" "));
            }

            let token = tokens[i];

            if i == 0 && token == self.bin {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default()
                        .fg(self.colors.preview_cmd)
                        .add_modifier(bold | Modifier::BOLD),
                ));
            } else if expect_flag_value {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.value).add_modifier(bold),
                ));
                expect_flag_value = false;
            } else if token.starts_with('-') {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.flag).add_modifier(bold),
                ));
                if let Some(&next) = tokens.get(i + 1) {
                    if !next.starts_with('-') && !subcommand_names.contains(next) {
                        expect_flag_value = true;
                    }
                }
            } else if subcommand_names.contains(token) {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.command).add_modifier(bold),
                ));
            } else {
                spans.push(Span::styled(
                    token.to_string(),
                    Style::default().fg(self.colors.arg).add_modifier(bold),
                ));
            }

            i += 1;
        }

        spans
    }
}

impl Widget for CommandPreview<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let border_color = if self.is_focused {
            self.colors.active_border
        } else {
            self.colors.inactive_border
        };

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(border_color))
            .title(" Command ")
            .title_style(Style::default().fg(border_color).bold())
            .padding(Padding::horizontal(1));

        let prefix = if self.is_focused { "▶ " } else { "$ " };
        let bold = if self.is_focused {
            Modifier::BOLD
        } else {
            Modifier::empty()
        };

        let mut spans = vec![Span::styled(prefix, Style::default().fg(self.colors.command))];
        spans.extend(self.colorize(bold));

        let paragraph = Paragraph::new(Line::from(spans))
            .block(block)
            .wrap(Wrap { trim: false });

        paragraph.render(area, buf);
    }
}
