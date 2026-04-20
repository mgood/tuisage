//! Composable filter wrapper for panel components.
//!
//! `FilterableComponent<C>` wraps an inner component that implements the
//! `Filterable` trait, adding interactive filter-typing behavior (keystroke
//! handling, filter input state) on top.  The inner component handles
//! rendering and score computation; the wrapper manages the interaction.
//!
//! This follows the compositional pattern: each layer builds on top of
//! the next without knowing internals.

use std::ops::{Deref, DerefMut};

use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::{buffer::Buffer, layout::Rect};

use nucleo_matcher::{Config, Matcher};
use ratatui_interact::components::InputState;

use crate::app::{fuzzy_match_score, MatchScores};
use crate::theme::UiColors;

use super::{Component, EventResult, OverlayRequest, RenderableComponent};

// ── FilterableItem ──────────────────────────────────────────────────

/// Cached item data for generic filter score computation.
///
/// Panels populate this during `sync_state` so that the filter wrapper
/// can compute match scores without needing external data.
pub struct FilterableItem {
    /// Lookup key (e.g. flag name, arg name, command id).
    pub key: String,
    /// All searchable name variations (flag name + --long + -short, arg name, etc.).
    pub name_texts: Vec<String>,
    /// Optional help text for matching.
    pub help: Option<String>,
}

/// Compute match scores for a set of filterable items against a pattern.
pub fn compute_match_scores(
    items: &[FilterableItem],
    pattern: &str,
) -> std::collections::HashMap<String, MatchScores> {
    let mut scores = std::collections::HashMap::new();
    let mut matcher = Matcher::new(Config::DEFAULT);

    for item in items {
        let name_score = item
            .name_texts
            .iter()
            .map(|t| fuzzy_match_score(t, pattern, &mut matcher))
            .max()
            .unwrap_or(0);

        let help_score = item
            .help
            .as_ref()
            .map(|h| fuzzy_match_score(h, pattern, &mut matcher))
            .unwrap_or(0);

        scores.insert(item.key.clone(), MatchScores { name_score, help_score });
    }
    scores
}

// ── Filterable trait ────────────────────────────────────────────────

/// Trait for components that support text filtering.
///
/// The wrapper calls these methods to apply/clear filter state.
/// Each panel implements them using its own data structures.
pub trait Filterable: Component {
    /// Apply a filter pattern. Compute scores and auto-select.
    /// Returns an action if the parent needs to react (e.g. path changed).
    fn apply_filter(&mut self, text: &str) -> Option<<Self as Component>::Action>;

    /// Clear all filter state (text, scores).
    /// Returns an action if the parent needs to react.
    fn clear_filter(&mut self) -> Option<<Self as Component>::Action>;

    /// Set whether the filter input is actively being typed.
    /// Used for border color rendering (active = highlighted border).
    fn set_filter_active(&mut self, active: bool);

    /// Whether a filter is applied (text is non-empty).
    fn has_active_filter(&self) -> bool;

    /// Current filter text.
    #[allow(dead_code)] // called through concrete impls and FilterableComponent delegation
    fn filter_text(&self) -> &str;
}

// ── FilterAction ────────────────────────────────────────────────────

/// Actions emitted by the FilterableComponent wrapper.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterAction<A> {
    /// Pass-through action from the inner component.
    Inner(A),
    /// User requested focus switch forward (Tab during filter).
    FocusNext,
    /// User requested focus switch backward (BackTab during filter).
    FocusPrev,
}

// ── FilterableComponent ─────────────────────────────────────────────

/// Composable wrapper that adds filter interaction to an inner component.
///
/// Owns the filter typing mode and input state. Delegates score computation,
/// navigation, and any optional rendering support to the inner component via
/// the `Filterable` trait.
///
/// Access inner component methods transparently via `Deref`/`DerefMut`.
pub struct FilterableComponent<C> {
    inner: C,
    /// Whether the user is actively typing a filter.
    filtering: bool,
    /// Filter input state for interactive typing.
    filter_input: InputState,
}

impl<C: Filterable> FilterableComponent<C> {
    pub fn new(inner: C) -> Self {
        Self {
            inner,
            filtering: false,
            filter_input: InputState::empty(),
        }
    }

    /// Whether the user is currently in interactive filter typing mode.
    pub fn is_filtering(&self) -> bool {
        self.filtering
    }

    /// Whether any filter is visually active (typing or text applied).
    pub fn has_active_filter(&self) -> bool {
        self.inner.has_active_filter()
    }

    /// Current filter text.
    pub fn filter_text(&self) -> &str {
        self.inner.filter_text()
    }

    /// Start interactive filter mode.
    pub fn start_filtering(&mut self) {
        self.filtering = true;
        self.filter_input.clear();
        self.inner.set_filter_active(true);
    }

    /// Set filter text programmatically (for tests).
    /// Applies the filter to the inner component.
    #[allow(dead_code)]
    pub fn set_filter_text(&mut self, text: &str) {
        self.filter_input.set_text(text.to_string());
        self.inner.apply_filter(text);
    }

    /// Get the filter input state (for tests).
    #[allow(dead_code)]
    pub fn filter_input(&self) -> &InputState {
        &self.filter_input
    }

    /// Get mutable filter input state (for tests).
    #[allow(dead_code)]
    pub fn filter_input_mut(&mut self) -> &mut InputState {
        &mut self.filter_input
    }

    /// Apply the current filter_input text to the inner component.
    /// Returns an EventResult wrapping any action from the inner.
    fn sync_filter(&mut self) -> EventResult<FilterAction<<C as Component>::Action>> {
        let text = self.filter_input.text().to_string();
        match self.inner.apply_filter(&text) {
            Some(action) => EventResult::Action(FilterAction::Inner(action)),
            None => EventResult::Consumed,
        }
    }

    fn stop_and_clear(&mut self) {
        self.filtering = false;
        self.filter_input.clear();
        self.inner.clear_filter();
        self.inner.set_filter_active(false);
    }
}

// ── Deref to inner ──────────────────────────────────────────────────

impl<C: Filterable> Deref for FilterableComponent<C> {
    type Target = C;
    fn deref(&self) -> &C {
        &self.inner
    }
}

impl<C: Filterable> DerefMut for FilterableComponent<C> {
    fn deref_mut(&mut self) -> &mut C {
        &mut self.inner
    }
}

// ── Component impl ──────────────────────────────────────────────────

impl<C: Filterable> Component for FilterableComponent<C> {
    type Action = FilterAction<C::Action>;

    fn handle_focus_gained(&mut self) -> EventResult<Self::Action> {
        self.inner.handle_focus_gained().map(FilterAction::Inner)
    }

    fn handle_focus_lost(&mut self) -> EventResult<Self::Action> {
        let result = self.inner.handle_focus_lost().map(FilterAction::Inner);
        if self.filtering || self.inner.has_active_filter() {
            self.stop_and_clear();
            match result {
                EventResult::NotHandled => EventResult::Consumed,
                other => other,
            }
        } else {
            result
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> EventResult<Self::Action> {
        if self.filtering {
            match key.code {
                KeyCode::Esc => {
                    self.stop_and_clear();
                    EventResult::Consumed
                }
                KeyCode::Enter => {
                    // Stop typing mode but keep the filter text applied
                    self.filtering = false;
                    self.inner.set_filter_active(false);
                    EventResult::Consumed
                }
                KeyCode::Tab => {
                    self.stop_and_clear();
                    EventResult::Action(FilterAction::FocusNext)
                }
                KeyCode::BackTab => {
                    self.stop_and_clear();
                    EventResult::Action(FilterAction::FocusPrev)
                }
                KeyCode::Backspace => {
                    self.filter_input.delete_char_backward();
                    self.sync_filter()
                }
                KeyCode::Char(c) => {
                    self.filter_input.insert_char(c);
                    self.sync_filter()
                }
                // Navigation during filter: delegate to inner
                KeyCode::Up | KeyCode::Down => {
                    self.inner.handle_key(key).map(FilterAction::Inner)
                }
                _ => EventResult::Consumed,
            }
        } else {
            // Not filtering — delegate to inner
            match self.inner.handle_key(key) {
                EventResult::NotHandled
                    if key.code == KeyCode::Esc && self.inner.has_active_filter() =>
                {
                    self.stop_and_clear();
                    EventResult::Consumed
                }
                EventResult::NotHandled if key.code == KeyCode::Char('/') => {
                    self.start_filtering();
                    EventResult::Consumed
                }
                other => other.map(FilterAction::Inner),
            }
        }
    }

    fn handle_mouse(&mut self, event: MouseEvent, area: Rect) -> EventResult<Self::Action> {
        self.inner.handle_mouse(event, area).map(FilterAction::Inner)
    }

    fn collect_overlays(&mut self) -> Vec<OverlayRequest> {
        self.inner.collect_overlays()
    }
}

impl<C: Filterable + RenderableComponent> RenderableComponent for FilterableComponent<C> {
    fn render(&mut self, area: Rect, buf: &mut Buffer, colors: &UiColors) {
        self.inner.render(area, buf, colors);
    }
}
