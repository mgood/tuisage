# Implementation

This document describes how TuiSage is built — its architecture, code structure, and key design decisions. It complements the behavioral specification in SPECIFICATION.md with concrete implementation details.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   main.rs                       │
│  - CLI arg parsing (clap derive)                │
│  - --usage output (clap_usage)                  │
│  - Spec loading (trailing args or --spec-file) │
│  - Base command override (--cmd)                │
│  - Terminal setup (ratatui + mouse capture)      │
│  - Event loop (key, mouse, resize)              │
│  - Action dispatch to App                        │
│  - Terminal teardown and command output          │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│             App (thin coordinator)               │
│  14 fields: spec, mode, execution, theme,       │
│  command_path, panels, flag/arg values,          │
│  focus_manager, layout snapshot, mouse_position │
│                                                 │
│  Dispatches events to focused component          │
│  Applies cross-cutting effects (sync_state,     │
│  focus changes, global flag propagation)         │
│  No editing/filtering state (owned by panels)   │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                   ui.rs                         │
│  Layout computation and panel arrangement       │
│  Thin coordinator: sets focus + mouse_position  │
│  on panels, calls render, collects overlays     │
│  Overlay pipeline (clamp + render last)         │
│  Click region registration                      │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│             src/components/                      │
│  mod.rs          — Component trait, EventResult, │
│                    OverlayContent, panel helpers │
│  filterable      — FilterableComponent wrapper  │
│  list_panel_base — ListPanelBase shared logic   │
│  command_panel   — CommandPanelComponent         │
│  flag_panel      — FlagPanelComponent            │
│  arg_panel       — ArgPanelComponent             │
│  choice_select   — ChoiceSelectComponent         │
│  theme_picker    — ThemePickerComponent           │
│  execution       — ExecutionComponent            │
│  preview         — CommandPreview Widget          │
│  help_bar        — HelpBar + Keybind Widget      │
│  select_list     — SelectList StatefulWidget     │
│                                                 │
│  command_builder.rs — Command string assembly   │
│  theme.rs    — UiColors semantic color palette  │
└─────────────────────────────────────────────────┘
```

## Source Files

### `src/main.rs`

Entry point. Parses CLI arguments (clap derive), handles `--usage` output via `clap_usage`, loads the usage spec (from trailing arguments via `sh -c` / `cmd /C`, or `--spec-file`), applies `--cmd` override, initializes the ratatui terminal with mouse capture, and runs the event loop.

The event loop has two modes:
- **Builder mode**: Blocking event read — delegates to `app.handle_key()` or `app.handle_mouse()`, which return an `Action` enum (`None`, `Quit`, or `Execute`).
- **Execution mode**: Polling event read (16ms interval) — forwards keyboard input to the PTY, continuously redraws to show live terminal output.

When execution starts, `main.rs` just asks `App` to enter execution mode for the current terminal size. `App` builds the command parts and delegates process creation to `ExecutionComponent::spawn()`, which owns PTY creation, parser setup, background threads, and cleanup wiring.

### `src/app.rs`

Core state and logic (~5500 lines, ~3500 of which are tests). Owns all mutable application state and coordinates between components.

#### Key Types

- **`Action`** — event handler return value: `None`, `Quit`, `Execute`.
- **`AppMode`** — `Builder` or `Executing`.
- **`Focus`** — focusable panels: `Commands`, `Flags`, `Args`, `Preview`.
- **`FlagValue`** — `Bool(bool)`, `NegBool(Option<bool>)` (None=omitted, Some(true)=on, Some(false)=off), `String(String)`, `Count(u32)`.
- **`ArgValue`** — name, value, required, choices, help.
- **`App`** — main application state struct.

#### `App` Struct Fields

| Field | Type | Purpose |
|---|---|---|
| `spec` | `usage::Spec` | The parsed usage specification |
| `mode` | `AppMode` | Current app mode (Builder or Executing) |
| `execution` | `Option<ExecutionComponent>` | Execution component when a command is running |
| `theme_name` | `ThemeName` | Current color theme name |
| `theme_picker` | `ThemePickerComponent` | Theme picker overlay component (manages own open/close lifecycle) |
| `command_path` | `Vec<String>` | Current position in the command tree (derived from tree selection) |
| `command_panel` | `FilterableComponent<CommandPanelComponent>` | Command tree panel wrapped in FilterableComponent (owns tree nodes, tree state, navigation) |
| `flag_values` | `HashMap<String, Vec<(String, FlagValue)>>` | Flag state keyed by command path |
| `flag_panel` | `FilterableComponent<FlagPanelComponent>` | Flag panel wrapped in FilterableComponent (owns ListPanelBase, negate cols, choice select, inline editing) |
| `arg_values_by_path` | `HashMap<String, Vec<ArgValue>>` | Persisted arg state keyed by command path |
| `arg_values` | `Vec<ArgValue>` | Current-path arg values cached for rendering and editing |
| `arg_panel` | `FilterableComponent<ArgPanelComponent>` | Arg panel wrapped in FilterableComponent (owns ListPanelBase, choice select, inline editing) |
| `focus_manager` | `FocusManager<Focus>` | Focus cycling logic (from ratatui-interact) |
| `layout` | `UiLayout` | Latest frame layout snapshot for click regions and overlay hit-testing |
| `mouse_position` | `Option<(u16, u16)>` | Current mouse cursor position for hover highlighting |

#### Key Responsibilities

- **Event dispatch** — `handle_key()` gives the focused `FilterableComponent` first crack at each key, then handles global shortcuts (theme, quit, execute, focus cycling). `handle_mouse()` maps clicks to focus changes and delegates to the focused panel, passing the panel’s render area as a parameter.
- **State synchronization** — `sync_state()` / `sync_command_path_from_tree()` initialize or restore flag/arg values when navigating to a new command, rebuild the focus manager, and refresh panels’ cached filterable items. Arg values are persisted per command path and rehydrated on revisit; global flag values are shared across all command levels, with deepest-level wins.
- **Focus management** — `set_focus()` notifies the losing and gaining components via focus hooks, which clean up applied filters, inline edits, and open overlays without ad hoc teardown in `App`.
- **Dynamic completions** — `find_completion()` looks up `complete` directives; `run_completion()` executes the shell command synchronously via `sh -c`. The focused panel emits a typed Enter request → `App` runs the completion → panel opens the choice select overlay. On failure, falls back to free-text editing. Results are not cached — the command re-runs each time the select box opens.
- **Command building** — thin wrappers delegating to `command_builder::build_command()` (display string) and `build_command_parts()` (process args).

#### Command Tree

Built at startup into a `TreeNode<CmdData>` hierarchy (hidden commands filtered). `flatten_command_tree()` converts this into a `Vec<FlatCommand>` with `depth`, `full_path`, and `id` fields. The flat list is the single source of truth for navigation, scoring, and rendering — all commands are always visible.

### `src/command_builder.rs`

Pure functions for assembling CLI command strings from application state. `build_command()` produces the display string; `build_command_parts()` produces a `Vec<String>` for process execution. `LiveArgPreview` captures in-progress edit state so the command preview reflects ongoing edits in real time.

### `src/ui.rs`

Rendering coordinator (~330 lines of layout + delegation, plus ~1150 lines of tests). Computes layout (preview at top, main content, help bar at bottom), sets focus and mouse position on panels, calls component render methods, collects overlays from components, and renders them via an overlay pipeline (viewport-clamped, last = topmost z-order). Click regions are registered during render and stored in `UiLayout` for later mouse hit-testing.

### `src/components/`

The component module hierarchy containing all reusable Widget implementations and shared helpers.

#### `src/components/mod.rs` — Component Traits & Shared Panel Helpers

Defines the shared component traits and panel rendering helpers.

**`Component` trait** — Interaction contract for stateful UI modules. Each component defines its own `Action` type via an associated type. Methods: `handle_focus_gained()`, `handle_focus_lost()`, `handle_key()`, `handle_mouse(event, area)`, `collect_overlays()`. Returns `EventResult<Self::Action>` from event handlers.

**`RenderableComponent` trait** — Rendering contract layered on top of `Component`. Only components that actually render their own primary content implement this trait.

**`EventResult<A>`** — Generic enum: `Consumed`, `NotHandled`, `Action(A)`. Combinators: `map()` and `and_then()` transform the action type.

**Overlay support** — `OverlayContent` trait for rendering overlay content. `OverlayRequest` describes a pending overlay. `clamp_overlay()` computes viewport-clamped position. Components return `Vec<OverlayRequest>` from `collect_overlays()`.

**Shared panel helpers** — `PanelState`, `ItemContext`, `panel_title()`, `panel_block()`, `push_selection_cursor()`, `push_highlighted_name()`, `build_help_line()`, `render_help_overlays()`, `push_edit_cursor()`, `selection_bg()`, `item_match_state()`, `build_highlighted_text()`. Also provides `find_adjacent_match()` and `find_first_match()` for filter-aware navigation shared by all panels.

#### `src/components/filterable.rs` — FilterableComponent Wrapper

Compositional wrapper (~280 lines) that adds filter interaction to any inner component implementing `Filterable`. Owns filter typing state independently of the inner component. Handles focus-loss cleanup. Uses `Deref`/`DerefMut` to transparently delegate non-filter operations to the inner component.

**`Filterable` trait** — defines `apply_filter()`, `clear_filter()`, `set_filter_active()`, `has_active_filter()`, `filter_text()`.

**`FilterAction<A>`** — wraps inner component actions: `Inner(A)`, `FocusNext`, `FocusPrev`.

#### `src/components/list_panel_base.rs` — ListPanelBase

Shared state and logic (~450 lines) embedded by both `FlagPanelComponent` and `ArgPanelComponent`. Provides list navigation, filter scoring, inline editing lifecycle, focus lifecycle teardown, choice select delegation, and hover/click handling. Uses internal event types (`EditEvent`, `ChoiceEvent`, `FocusLostEvent`) to decouple from panel-specific action types.

#### `src/components/command_panel.rs` — CommandPanelComponent

Owns command tree state and renders the command tree as a selectable flat indented list with depth-based indentation, aliases, fuzzy-match highlighting, scrollbar, and help overlays (~440 lines). Wrapped in `FilterableComponent`. Also provides `render_panel_scrollbar()` used by all panels.

#### `src/components/flag_panel.rs` — FlagPanelComponent

Embeds `ListPanelBase` and adds flag-specific behavior (~430 lines). Owns `negate_cols` for tristate toggle rendering and click regions. Renders via `render_with_data(FlagRenderData)` since flag values live in `App`. Emits typed Enter requests for choice/completion overlay activation.

#### `src/components/arg_panel.rs` — ArgPanelComponent

Embeds `ListPanelBase` and adds arg-specific behavior (~380 lines). Renders via `render_with_data(ArgRenderData)` since arg values are supplied by `App`. Emits typed Enter requests for choice/completion overlay activation.

#### `src/components/execution.rs` — ExecutionComponent

Owns the execution state (PTY, parser, exit status) and encapsulates execution startup, rendering, key handling, and PTY I/O (~225 lines). Key types: `ExecutionState` (command, parser, PTY writer/master, exit status), `ExecutionAction` (`Close`).

#### `src/components/choice_select.rs` — ChoiceSelectComponent

Self-contained filtered choice selection overlay (~615 lines, 22 unit tests). Manages open/close lifecycle, filter state, and overlay rendering. Used by FlagPanel and ArgPanel for flags/args with predefined choices or dynamic completions.

#### `src/components/theme_picker.rs` — ThemePickerComponent

Self-contained theme picker overlay (~380 lines, 12 unit tests). Manages open/close lifecycle, theme preview during navigation, and overlay rendering. Key types: `ThemePickerAction` (`PreviewTheme`, `Confirmed`, `Cancelled`).

#### `src/components/preview.rs` — CommandPreview Widget

Renders the assembled command string with syntax-aware token coloring (binary name, subcommands, flags, positional arguments) (~130 lines).

#### `src/components/help_bar.rs` — HelpBar + Keybind Widget

Renders the context-sensitive help/status bar with keyboard shortcuts and a right-aligned theme indicator (~100 lines).

#### `src/components/select_list.rs` — SelectList StatefulWidget

Renders a bordered selectable list overlay used by both the choice select dropdown and theme picker (~230 lines). Supports empty state, scroll offset, optional cursor prefix, configurable colors, per-item descriptions, and hover highlighting.

### `src/theme.rs`

Semantic color palette (~80 lines). `UiColors` is derived from the active `ThemePalette` and maps abstract roles (command, flag, arg, value, active border, etc.) to concrete `Color` values.

## Dependencies

| Crate | Version | Purpose | Notes |
|---|---|---|---|
| `clap` | 4 | CLI argument parsing | `derive` feature for struct-based arg definitions |
| `clap_usage` | 2.0 | Usage spec generation | Generates `.usage.kdl` output from clap `Command` |
| `usage-lib` | 2.16 | Parse `.usage.kdl` specs | `default-features = false` (skip docs/tera/roff) |
| `ratatui` | 0.30 | TUI framework | Provides `Frame`, `Terminal`, widgets, layout |
| `crossterm` | 0.29 | Terminal backend + events | `event-stream` feature enabled |
| `ratatui-interact` | 0.4 | UI components | `TreeView`, `TreeNode`, `FocusManager`, `ListPickerState` |
| `ratatui-themes` | 0.1 | Color theming | Provides `ThemePalette` with named themes |
| `nucleo-matcher` | 0.3 | Fuzzy matching | Pattern-based scored matching with smart case, normalization, and multi-word support |
| `portable-pty` | 0.9 | Pseudo-terminal management | Cross-platform PTY creation and process spawning |
| `tui-term` | 0.3 | Terminal widget | `PseudoTerminal` widget for rendering PTY output in ratatui |
| `vt100` | 0.16 | Terminal emulation | VT100 parser for processing terminal control sequences |
| `color-eyre` | 0.6 | Error reporting | Pretty error messages with backtraces |
| `insta` | 1 | Snapshot testing (dev) | Full terminal output comparison |
| `pretty_assertions` | 1 | Test diffs (dev) | Better assertion failure output |

### Why `usage-lib` With No Default Features

The `usage-lib` crate includes optional features for generating documentation, man pages (roff), and templates (tera). TuiSage only needs spec parsing, so these are disabled to minimize compile time and binary size.

## Test Infrastructure


### Test Fixtures

The primary test fixture is `fixtures/sample.usage.kdl`, which covers:

- Nested subcommands (2–3 levels deep: `plugin > install/remove`)
- Boolean flags (`--force`, `--verbose`, `--rollback`)
- Flags with values (`--tag <tag>`, `--output <file>`)
- Required and optional positional arguments
- Flags with choices (`environment: dev|staging|prod`)
- Dynamic completions (`complete` directives on plugin install/uninstall/update)
- Global flags (`--verbose`, `--quiet`)
- Aliases (`set`/`add`, `list`/`ls`, `remove`/`rm`)
- Count flags (`--verbose` with `count=true`)

Tests construct specs inline via `usage::Spec::parse_file()` on the fixture or by parsing spec strings directly.

### Snapshot Testing

Snapshot tests use `insta` to capture the full terminal output of a rendered frame at a fixed size (typically 80×24 or 60×20). This catches unintended visual regressions.

Snapshots are stored in `src/snapshots/` and should be reviewed with `cargo insta review` when they change.

Snapshot tests cover: root view, subcommand views, flag toggling, argument editing, filter mode, preview focus, deep navigation, theme variations, and edge cases (no subcommands, narrow terminal).

## Key Design Decisions

| Decision | Rationale |
|---|---|
| Shared panel helpers (`components/mod.rs`) | Extracts common panel rendering patterns (borders, selection cursors, text highlighting, filter scoring) into reusable helpers used by all three list panel components. Eliminates duplication and ensures consistent behavior. |
| Custom `Widget` trait implementations | `CommandPreview`, `HelpBar`, and `SelectList` implement ratatui's `Widget` trait for self-contained, composable rendering. Positioning/layout logic stays in `ui.rs`; rendering logic lives in the widget. |
| Core modules (`app.rs`, `ui.rs`) as thin coordinators | `app.rs` owns state and dispatches events; `ui.rs` computes layout and delegates to components. Shared UI patterns live in `components/mod.rs`; navigation helpers live in `components/filterable.rs` and `components/list_panel_base.rs`. |
| `Vec<String>` for command path | Simple push/pop navigation through the command tree. Joined with `>` for hash map keys. |
| State preserved across navigation | Flag/arg values are stored per command-path key, so navigating away and back retains previous selections. |
| Startup sync via `sync_command_path_from_tree()` | On construction, the tree selection and command path are synchronized so the correct flags/args are displayed on the first render without requiring a key press. |
| Global flags collected from all levels | `build_command()` scans all command-path levels for global flag values (deepest wins), emitting them before subcommands. This ensures global flags toggled from any subcommand are included. |
| Rendering to stderr, output to stdout | Enables composability: `eval "$(tuisage spec.kdl)"` works because TUI output doesn't mix with the command string. |
| Crossterm backend | Best cross-platform terminal support (macOS, Linux, Windows). |
| Theme palette mapping | Instead of hardcoding colors, derive semantic colors from the active theme. This makes all themes work automatically. |
| `TreeViewState` + `TreeNode` from ratatui-interact | Provides the command hierarchy data model. Used as a flat indented list (all nodes always visible) with selection and scroll offset tracking. |
| `ListPickerState` from ratatui-interact | Provides selection index + scroll offset tracking for flags and args panels. |
| `FocusManager` from ratatui-interact | Handles Tab/Shift-Tab cycling with dynamic panel availability, reducing boilerplate. |
| Finishing edits on focus change | Prevents a class of bugs where the edit input text leaks into the wrong field when clicking elsewhere. |
| Synchronous completion execution | Completion commands are run synchronously via `sh -c` each time the select box is opened. Avoids async complexity; commands are expected to be fast. Fresh results on every open. |
| Completion fallback to free-text | When a completion command fails or the user presses Esc, the typed text is kept as the value. The select overlay is non-blocking. |
| Unified choice select + text input | Opening a choice select also enters editing mode. The text input serves as both the value and the filter. Typing clears any active selection, so users can seamlessly switch between browsing choices and entering custom text. |
| Consistent selection highlighting | All three panels (Commands, Flags, Args) use the same `selection_bg` background color on the selected item, plus the `▶` caret. Help text is rendered as a right-aligned overlay after the List widget, with automatic skip when it would overlap item content. |
| Commands list always visible | The flat indented list shows the entire command hierarchy, not just subcommands of the current selection. It remains visible even when navigating to leaf commands with no children, providing constant wayfinding context. |
| Arguments panel visibility based on spec | The arguments panel is shown whenever the current command defines arguments in the spec, ensuring consistent visibility regardless of whether arg values are populated. |

## Remaining Work

- **Clipboard copy** — copy the built command to the system clipboard from within the TUI
- **Embedded USAGE blocks** — verify and test support for script files with heredoc USAGE blocks via `--spec-file`
- **Further module splitting** — enter/completion lookup and value-mutation orchestration still live in App; these could move into dedicated services or richer panel-side actions to further reduce coordination responsibilities
- **CI pipeline** — GitHub Actions for `cargo test`, `cargo clippy`, and `insta` snapshot checks
