//! Self-contained theme picker overlay component.
//!
//! Manages open/close state, keyboard navigation (with wrapping),
//! live theme preview, and rendering as an overlay.

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};
use ratatui_themes::ThemeName;

use super::select_list::SelectList;
use super::{Component, EventResult, OverlayContent, OverlayRequest};
use crate::theme::UiColors;

/// Actions emitted by the theme picker for the parent to process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThemePickerAction {
    /// Preview a theme (set as active). Emitted on navigation and click.
    PreviewTheme(ThemeName),
    /// Confirm the current preview (close picker, keep theme).
    Confirmed,
    /// Cancel and restore the original theme.
    Cancelled(ThemeName),
}

/// Self-contained theme picker overlay.
pub struct ThemePickerComponent {
    state: Option<ThemePickerInner>,
    /// The viewport area, set by the UI coordinator before collecting overlays.
    viewport: Rect,
    /// Mouse position from parent, used for hover highlighting.
    mouse_position: Option<(u16, u16)>,
}

struct ThemePickerInner {
    original_theme: ThemeName,
    selected_index: usize,
}

impl ThemePickerComponent {
    pub fn new() -> Self {
        Self {
            state: None,
            viewport: Rect::ZERO,
            mouse_position: None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.state.is_some()
    }

    /// Open the picker, recording the current theme for Esc restoration.
    pub fn open(&mut self, current_theme: ThemeName) {
        let all = ThemeName::all();
        let selected_index = all.iter().position(|t| *t == current_theme).unwrap_or(0);
        self.state = Some(ThemePickerInner {
            original_theme: current_theme,
            selected_index,
        });
    }

    pub fn close(&mut self) {
        self.state = None;
    }

    /// Set the viewport so collect_overlays can compute the anchor position.
    pub fn set_viewport(&mut self, viewport: Rect) {
        self.viewport = viewport;
    }

    /// Update the mouse position for hover highlighting in the overlay.
    pub fn set_mouse_position(&mut self, pos: Option<(u16, u16)>) {
        self.mouse_position = pos;
    }

    #[cfg(test)]
    pub fn selected_index(&self) -> Option<usize> {
        self.state.as_ref().map(|s| s.selected_index)
    }

    #[cfg(test)]
    pub fn original_theme(&self) -> Option<ThemeName> {
        self.state.as_ref().map(|s| s.original_theme)
    }

    /// Handle a mouse click. Returns an action if the picker is open.
    pub fn click_at(
        &mut self,
        col: u16,
        row: u16,
        overlay_rect: Option<Rect>,
    ) -> Option<ThemePickerAction> {
        if !self.is_open() {
            return None;
        }

        if let Some(rect) = overlay_rect {
            let inner_top = rect.y + 1;
            let inner_bottom = rect.y + rect.height.saturating_sub(1);
            if col >= rect.x
                && col < rect.x + rect.width
                && row >= inner_top
                && row < inner_bottom
            {
                let clicked_index = (row - inner_top) as usize;
                let all = ThemeName::all();
                if clicked_index < all.len() {
                    let theme = all[clicked_index];
                    self.close();
                    return Some(ThemePickerAction::PreviewTheme(theme));
                }
            }
        }

        // Click outside — cancel
        let original = self.state.as_ref().unwrap().original_theme;
        self.close();
        Some(ThemePickerAction::Cancelled(original))
    }

    fn overlay_size(&self) -> (u16, u16) {
        let all = ThemeName::all();
        let max_name_len = all
            .iter()
            .map(|t: &ThemeName| t.display_name().len())
            .max()
            .unwrap_or(10) as u16;
        let width = max_name_len + 6; // "▶ " prefix (2) + padding (2) + borders (2)
        let num_themes = all.len() as u16;
        let height = (num_themes + 2).min(self.viewport.height.saturating_sub(2)); // +2 for borders
        (width, height)
    }
}

impl Component for ThemePickerComponent {
    type Action = ThemePickerAction;

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<Self::Action> {
        let Some(ref mut inner) = self.state else {
            return EventResult::NotHandled;
        };

        let all = ThemeName::all();
        let len = all.len();

        match key.code {
            KeyCode::Esc => {
                let original = inner.original_theme;
                self.close();
                EventResult::Action(ThemePickerAction::Cancelled(original))
            }
            KeyCode::Enter => {
                self.close();
                EventResult::Action(ThemePickerAction::Confirmed)
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if inner.selected_index > 0 {
                    inner.selected_index -= 1;
                } else {
                    inner.selected_index = len - 1;
                }
                EventResult::Action(ThemePickerAction::PreviewTheme(all[inner.selected_index]))
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if inner.selected_index + 1 < len {
                    inner.selected_index += 1;
                } else {
                    inner.selected_index = 0;
                }
                EventResult::Action(ThemePickerAction::PreviewTheme(all[inner.selected_index]))
            }
            _ => EventResult::Consumed,
        }
    }

    fn handle_mouse(&mut self, _event: MouseEvent, _area: Rect) -> EventResult<Self::Action> {
        EventResult::NotHandled
    }

    fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        let Some(ref inner) = self.state else {
            return vec![];
        };

        let (width, height) = self.overlay_size();

        // Right-aligned, above the help bar (last row of viewport)
        let anchor_x = self.viewport.right().saturating_sub(width);
        let help_bar_y = self.viewport.bottom().saturating_sub(1);
        let anchor_y = help_bar_y.saturating_sub(height);
        let anchor = Rect::new(anchor_x, anchor_y, 0, 0);

        let labels: Vec<String> = ThemeName::all()
            .iter()
            .map(|t: &ThemeName| t.display_name().to_string())
            .collect();

        vec![OverlayRequest {
            anchor,
            size: (width, height),
            content: Box::new(ThemePickerOverlay {
                labels,
                selected_index: inner.selected_index,
                mouse_position: self.mouse_position,
            }),
        }]
    }
}

struct ThemePickerOverlay {
    labels: Vec<String>,
    selected_index: usize,
    mouse_position: Option<(u16, u16)>,
}

impl ThemePickerOverlay {
    /// Compute hovered item index from mouse position relative to the overlay area.
    fn hovered_index(&self, area: Rect) -> Option<usize> {
        let (col, row) = self.mouse_position?;
        let inner_top = area.y + 1; // skip top border
        let inner_bottom = area.y + area.height.saturating_sub(1);
        if col >= area.x
            && col < area.x + area.width
            && row >= inner_top
            && row < inner_bottom
        {
            let idx = (row - inner_top) as usize;
            if idx < self.labels.len() {
                Some(idx)
            } else {
                None
            }
        } else {
            None
        }
    }
}

impl OverlayContent for ThemePickerOverlay {
    fn render(&self, area: Rect, buf: &mut Buffer, colors: &UiColors) {
        let hovered = self.hovered_index(area);
        let widget = SelectList::new(
            " Theme ".to_string(),
            &self.labels,
            Some(self.selected_index),
            colors.choice,
            colors.value,
            colors,
        )
        .with_cursor()
        .with_hovered(hovered);
        Widget::render(widget, area, buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn test_open_close_lifecycle() {
        let mut tp = ThemePickerComponent::new();
        assert!(!tp.is_open());

        tp.open(ThemeName::Dracula);
        assert!(tp.is_open());
        assert_eq!(tp.selected_index(), Some(0));
        assert_eq!(tp.original_theme(), Some(ThemeName::Dracula));

        tp.close();
        assert!(!tp.is_open());
        assert_eq!(tp.selected_index(), None);
    }

    #[test]
    fn test_navigate_down_up() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Dracula);

        let result = tp.handle_key(key(KeyCode::Down));
        assert!(matches!(result, EventResult::Action(ThemePickerAction::PreviewTheme(_))));
        assert_eq!(tp.selected_index(), Some(1));

        let result = tp.handle_key(key(KeyCode::Up));
        assert!(matches!(result, EventResult::Action(ThemePickerAction::PreviewTheme(_))));
        assert_eq!(tp.selected_index(), Some(0));
    }

    #[test]
    fn test_wrap_around() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Dracula);

        // Up from first wraps to last
        let result = tp.handle_key(key(KeyCode::Up));
        let all = ThemeName::all();
        assert_eq!(tp.selected_index(), Some(all.len() - 1));
        assert!(matches!(result, EventResult::Action(ThemePickerAction::PreviewTheme(t)) if t == *all.last().unwrap()));

        // Down from last wraps to first
        let result = tp.handle_key(key(KeyCode::Down));
        assert_eq!(tp.selected_index(), Some(0));
        assert!(matches!(result, EventResult::Action(ThemePickerAction::PreviewTheme(t)) if t == ThemeName::Dracula));
    }

    #[test]
    fn test_jk_navigation() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Dracula);

        tp.handle_key(key(KeyCode::Char('j')));
        assert_eq!(tp.selected_index(), Some(1));

        tp.handle_key(key(KeyCode::Char('k')));
        assert_eq!(tp.selected_index(), Some(0));
    }

    #[test]
    fn test_enter_confirms() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Dracula);
        tp.handle_key(key(KeyCode::Down)); // preview next

        let result = tp.handle_key(key(KeyCode::Enter));
        assert_eq!(result, EventResult::Action(ThemePickerAction::Confirmed));
        assert!(!tp.is_open());
    }

    #[test]
    fn test_esc_cancels_with_original() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Nord);

        let result = tp.handle_key(key(KeyCode::Esc));
        assert_eq!(
            result,
            EventResult::Action(ThemePickerAction::Cancelled(ThemeName::Nord))
        );
        assert!(!tp.is_open());
    }

    #[test]
    fn test_not_handled_when_closed() {
        let mut tp = ThemePickerComponent::new();
        let result = tp.handle_key(key(KeyCode::Down));
        assert_eq!(result, EventResult::NotHandled);
    }

    #[test]
    fn test_unknown_key_consumed() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Dracula);

        let result = tp.handle_key(key(KeyCode::Char('x')));
        assert_eq!(result, EventResult::Consumed);
        assert!(tp.is_open()); // still open
    }

    #[test]
    fn test_collect_overlays_empty_when_closed() {
        let mut tp = ThemePickerComponent::new();
        assert!(tp.collect_overlays().is_empty());
    }

    #[test]
    fn test_collect_overlays_returns_one_when_open() {
        let mut tp = ThemePickerComponent::new();
        tp.set_viewport(Rect::new(0, 0, 100, 24));
        tp.open(ThemeName::Dracula);

        let overlays = tp.collect_overlays();
        assert_eq!(overlays.len(), 1);
    }

    #[test]
    fn test_click_inside_overlay() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Dracula);

        // Click on the second theme item (row 7 = inner_top(6) + 1)
        let result = tp.click_at(85, 7, Some(Rect::new(80, 5, 18, 12)));
        assert!(matches!(result, Some(ThemePickerAction::PreviewTheme(_))));
        assert!(!tp.is_open()); // closed after click
    }

    #[test]
    fn test_click_outside_overlay_cancels() {
        let mut tp = ThemePickerComponent::new();
        tp.open(ThemeName::Nord);

        let result = tp.click_at(5, 5, Some(Rect::new(80, 5, 18, 12)));
        assert_eq!(result, Some(ThemePickerAction::Cancelled(ThemeName::Nord)));
        assert!(!tp.is_open());
    }

    #[test]
    fn test_hover_highlight_on_theme_item() {
        use crate::theme::UiColors;
        use ratatui::buffer::Buffer;

        let mut tp = ThemePickerComponent::new();
        tp.set_viewport(Rect::new(0, 0, 100, 24));
        tp.open(ThemeName::Dracula);

        // Set mouse position over the second item row.
        // Overlay area: x=80, y=10, inner_top = 11, so row 12 = item index 1.
        let overlay_area = Rect::new(80, 10, 18, 12);
        let hovered_row = 12u16;
        tp.set_mouse_position(Some((85, hovered_row)));

        let overlays = tp.collect_overlays();
        assert_eq!(overlays.len(), 1);

        let palette = ratatui_themes::Theme::default().palette();
        let colors = UiColors::from_palette(&palette);
        let mut buf = Buffer::empty(overlay_area);
        overlays[0].content.render(overlay_area, &mut buf, &colors);

        // The hovered row should have a non-reset background
        let cell = buf.cell((81, hovered_row)).unwrap();
        assert_ne!(
            cell.bg,
            ratatui::style::Color::Reset,
            "Hovered theme item row should have a background highlight"
        );

        // The non-hovered, non-selected row below should not have a hover background
        let non_hover_row = hovered_row + 1;
        let cell_below = buf.cell((81, non_hover_row)).unwrap();
        assert_eq!(
            cell_below.bg,
            ratatui::style::Color::Reset,
            "Non-hovered theme item should not have a background highlight"
        );
    }
}
