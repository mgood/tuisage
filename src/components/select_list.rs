//! Select list widget — bordered selectable list overlay.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, StatefulWidget, Widget},
};

use crate::theme::UiColors;

/// A widget that renders a bordered selectable list overlay.
///
/// Used for both the choice select dropdown and the theme picker.
/// Clears the area behind it, draws a bordered block with a title,
/// and renders selectable items with a `▶` cursor indicator.
/// Optionally shows descriptions alongside items.
pub struct SelectList<'a> {
    /// Title shown in the block border (empty string = no title).
    pub title: String,
    /// Item labels to display.
    pub items: &'a [String],
    /// Optional descriptions for each item (parallel to `items`).
    pub descriptions: &'a [Option<String>],
    /// Currently selected index (None = no selection).
    pub selected: Option<usize>,
    /// Currently hovered index (None = no hover).
    pub hovered: Option<usize>,
    /// Whether to show the ▶ selection cursor prefix.
    pub show_cursor: bool,
    /// Which borders to show (defaults to ALL).
    pub borders: Borders,
    /// Style for unselected items.
    pub item_color: Color,
    /// Style for the selected item (text color).
    pub selected_color: Color,
    pub colors: &'a UiColors,
}

impl<'a> SelectList<'a> {
    pub fn new(
        title: String,
        items: &'a [String],
        selected: Option<usize>,
        item_color: Color,
        selected_color: Color,
        colors: &'a UiColors,
    ) -> Self {
        Self {
            title,
            items,
            descriptions: &[],
            selected,
            hovered: None,
            show_cursor: false,
            borders: Borders::ALL,
            item_color,
            selected_color,
            colors,
        }
    }

    /// Set descriptions to display alongside items.
    pub fn with_descriptions(mut self, descriptions: &'a [Option<String>]) -> Self {
        self.descriptions = descriptions;
        self
    }

    /// Show ▶ prefix cursor for the selected item.
    pub fn with_cursor(mut self) -> Self {
        self.show_cursor = true;
        self
    }

    /// Set which borders to show.
    pub fn with_borders(mut self, borders: Borders) -> Self {
        self.borders = borders;
        self
    }

    /// Set the hovered index.
    #[allow(dead_code)]
    pub fn with_hovered(mut self, hovered: Option<usize>) -> Self {
        self.hovered = hovered;
        self
    }
}

impl Widget for SelectList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut state = SelectListScrollState::default();
        StatefulWidget::render(self, area, buf, &mut state);
    }
}

/// Scroll state for `SelectList`, tracking the computed scroll offset and visible items.
/// Pass this to `StatefulWidget::render` to get the scroll offset back for mouse handling.
#[derive(Debug, Clone, Default)]
pub struct SelectListScrollState {
    /// The scroll offset computed during rendering (items scrolled past the top).
    pub scroll_offset: usize,
    /// Number of visible item rows in the viewport.
    pub visible_items: usize,
}

impl StatefulWidget for SelectList<'_> {
    type State = SelectListScrollState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        use ratatui::widgets::{Scrollbar, ScrollbarOrientation, ScrollbarState};

        // Clear area behind the overlay
        ratatui::widgets::Clear.render(area, buf);

        let mut block = Block::default()
            .borders(self.borders)
            .border_style(Style::default().fg(self.colors.active_border));
        if !self.title.is_empty() {
            block = block
                .title(self.title)
                .title_style(
                    Style::default()
                        .fg(self.colors.active_border)
                        .add_modifier(Modifier::BOLD),
                );
        }

        let items: Vec<ratatui::widgets::ListItem> = if self.items.is_empty() {
            vec![ratatui::widgets::ListItem::new(Line::from(Span::styled(
                "(no matches)",
                Style::default().fg(self.colors.help).italic(),
            )))]
        } else {
            self.items
                .iter()
                .enumerate()
                .map(|(i, label)| {
                    let is_selected = self.selected == Some(i);
                    let style = if is_selected {
                        Style::default()
                            .fg(self.selected_color)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(self.item_color)
                    };

                    let mut spans = Vec::new();
                    if self.show_cursor {
                        let prefix = if is_selected { "▶ " } else { "  " };
                        spans.push(Span::styled(
                            prefix,
                            if is_selected {
                                Style::default()
                                    .fg(self.colors.active_border)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default()
                            },
                        ));
                    }
                    spans.push(Span::styled(label.clone(), style));

                    // Add description if present
                    if let Some(Some(desc)) = self.descriptions.get(i) {
                        spans.push(Span::styled(
                            format!("  {}", desc),
                            Style::default().fg(self.colors.help),
                        ));
                    }

                    let mut item = ratatui::widgets::ListItem::new(Line::from(spans));
                    if is_selected {
                        item = item.style(Style::default().bg(self.colors.selected_bg));
                    } else if self.hovered == Some(i) {
                        item = item.style(Style::default().bg(self.colors.hover_bg));
                    }
                    item
                })
                .collect()
        };

        // Calculate visible items based on which borders are present
        let border_height = if self.borders.contains(Borders::TOP) { 1 } else { 0 }
            + if self.borders.contains(Borders::BOTTOM) { 1 } else { 0 };
        let visible_items = area.height.saturating_sub(border_height) as usize;
        state.visible_items = visible_items;

        let mut list_state = ratatui::widgets::ListState::default().with_selected(
            if self.items.is_empty() {
                None
            } else {
                self.selected
            },
        );

        let scroll_offset = if let Some(sel) = self.selected {
            if visible_items > 0 && sel >= visible_items {
                sel.saturating_sub(visible_items - 1)
            } else {
                0
            }
        } else {
            0
        };
        list_state = list_state.with_offset(scroll_offset);
        state.scroll_offset = scroll_offset;

        let list = ratatui::widgets::List::new(items).block(block);
        ratatui::widgets::StatefulWidget::render(list, area, buf, &mut list_state);

        // Render scrollbar if content overflows
        let total_items = self.items.len();
        if total_items > visible_items && visible_items > 0 {
            let inner = area.inner(ratatui::layout::Margin {
                horizontal: 0,
                vertical: if self.borders.contains(Borders::TOP) { 1 } else { 0 },
            });
            // Adjust inner height if bottom border is present
            let scrollbar_area = if self.borders.contains(Borders::BOTTOM) {
                Rect::new(inner.x, inner.y, inner.width, inner.height.saturating_sub(
                    if self.borders.contains(Borders::BOTTOM) { 1 } else { 0 }
                ))
            } else {
                inner
            };
            let mut scrollbar_state = ScrollbarState::new(total_items.saturating_sub(visible_items))
                .position(scroll_offset);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(Some("│"))
                .thumb_symbol("┃")
                .track_style(Style::default().fg(self.colors.inactive_border))
                .thumb_style(Style::default().fg(self.colors.active_border));
            StatefulWidget::render(scrollbar, scrollbar_area, buf, &mut scrollbar_state);
        }
    }
}
