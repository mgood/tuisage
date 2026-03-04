# Implementation

This document describes how TuiSage is built — its architecture, code structure, key design decisions, and current development state. It complements the behavioral specification in SPECIFICATION.md with concrete implementation details.

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
│  - Command execution (PTY spawn + I/O threads)  │
│  - Terminal teardown and command output          │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                   app.rs                        │
│  - App struct (holds Spec + all runtime state)  │
│  - AppMode (Builder / Executing) + transitions  │
│  - ExecutionState (PTY parser, writer, status)  │
│  - Command path navigation (Vec<String>)        │
│  - Flag/arg value storage (HashMaps)            │
│  - Focus management (via ratatui-interact)      │
│  - Key event handling → Action dispatch         │
│  - Mouse event handling → Action dispatch       │
│  - Execution mode key forwarding to PTY         │
│  - Command string builder + command parts       │
│  - Fuzzy matching (nucleo-matcher)              │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                   ui.rs                         │
│  - Layout computation (panels, preview, help    │
│    bar)                                         │
│  - Component rendering (commands, flags, args)  │
│  - Command preview colorization                 │
│  - Execution view (PseudoTerminal + status)     │
│  - Click region registration                    │
│  - Delegates to widgets.rs for shared patterns  │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                 widgets.rs                      │
│  - UiColors (semantic color palette)            │
│  - PanelState (shared panel rendering context)  │
│  - Panel chrome (title, block, borders)         │
│  - Selection cursor, highlighted names          │
│  - Help text, edit cursor, selection bg         │
│  - Fuzzy match highlighting (build_highlighted_ │
│    text)                                        │
└─────────────────────────────────────────────────┘
```

## Source Files

### `src/main.rs`

The entry point. Responsibilities:

- Parse CLI arguments via `clap` (derive mode) into an `Args` struct with trailing arguments (spec command), `--spec-file`, `--cmd`, and `--usage` flags.
- Handle `--usage` by generating TuiSage's own usage spec via `clap_usage::generate()` and exiting.
- Validate that either trailing arguments or `--spec-file` is provided (but not both).
- Load the usage spec: run a shell command (`sh -c` / `cmd /C`) for trailing arguments, or read a file for `--spec-file`.
- Override the spec's `bin` field if `--cmd` is provided.
- Enable mouse capture via `crossterm::event::EnableMouseCapture`.
- Initialize the ratatui `DefaultTerminal`.
- Run the event loop (`run_event_loop`), which draws frames and dispatches events.
- Spawn commands in a PTY via `spawn_command()` when the `Execute` action is triggered.
- On exit, restore the terminal, disable mouse capture, and exit.

The event loop has two modes:
- **Builder mode**: Blocking event read — delegates to `app.handle_key()` or `app.handle_mouse()`, which return an `Action` enum (`None`, `Quit`, or `Execute`).
- **Execution mode**: Polling event read (16ms interval) — forwards keyboard input to the PTY, continuously redraws to show live terminal output.

The `spawn_command()` function:
1. Calls `app.build_command_parts()` to get separate process arguments.
2. Creates a PTY pair via `portable-pty::NativePtySystem`, sized to fit the terminal area.
3. Spawns the command on the PTY slave using `CommandBuilder` with individual args (not shell-stringified).
4. Starts background threads for: reading PTY output into `vt100::Parser`, waiting for child exit, and cleanup.
5. Sets up the `ExecutionState` and switches the app to `AppMode::Executing`.

### `src/app.rs`

The core state and logic module (~5600 lines including ~1400 lines of tests).

#### Key Types

- **`Action`** — return value from event handlers: `None`, `Quit`, `Execute`.
- **`AppMode`** — enum: `Builder` (normal command-building UI) or `Executing` (embedded terminal running).
- **`ExecutionState`** — struct holding PTY state: `command_display`, `parser` (vt100), `pty_writer`, `pty_master`, `exited` flag, `exit_status`.
- **`Focus`** — enum of focusable panels: `Commands`, `Flags`, `Args`, `Preview`.
- **`FlagValue`** — discriminated union: `Bool(bool)`, `NegBool(Option<bool>)` (for negatable flags: None=omitted, Some(true)=on, Some(false)=off), `String(String)`, `Count(u32)`.
- **`ArgValue`** — struct with `name`, `value`, `required`, `choices`, `help` fields.
- **`CmdData`** — data stored in each tree node: `name`, `help`, `aliases`.
- **`ChoiceSelectState`** — state for the inline choice select box: `choices`, `descriptions` (optional per-choice descriptions), `selected_index` (`Option<usize>`, None = no selection), `source_panel`, `source_index`, `filter_active` (starts false so all choices are visible on open; activates on first keystroke). When open, `editing` is also true and `edit_input` serves as both the text value and the filter.
- **`App`** — the main application state struct.

#### `App` Struct Fields

| Field | Type | Purpose |
|---|---|---|
| `spec` | `usage::Spec` | The parsed usage specification |
| `mode` | `AppMode` | Current app mode (Builder or Executing) |
| `execution` | `Option<ExecutionState>` | Execution state when a command is running |
| `theme_name` | `ThemeName` | Current color theme name |
| `theme_picker` | `Option<ThemePickerState>` | State for the theme picker overlay (None when closed) |
| `theme_indicator_rect` | `Option<Rect>` | Area of theme indicator in help bar (for mouse clicks) |
| `command_path` | `Vec<String>` | Current position in the command tree (derived from tree selection) |
| `flag_values` | `HashMap<String, Vec<(String, FlagValue)>>` | Flag state keyed by command path |
| `arg_values` | `Vec<ArgValue>` | Arg values for the current command |
| `focus_manager` | `FocusManager<Focus>` | Focus cycling logic (from ratatui-interact) |
| `editing` | `bool` | Whether a text input is currently active |
| `filter_input` | `InputState` | State for the fuzzy filter text input |
| `filtering` | `bool` | Whether filter mode is active |
| `command_tree_nodes` | `Vec<TreeNode<CmdData>>` | Full command hierarchy as tree nodes (built from spec) |
| `command_tree_state` | `TreeViewState` | Tree view state: selection index, collapsed set, scroll |
| `flag_list_state` | `ListPickerState` | Selection + scroll state for flags |
| `arg_list_state` | `ListPickerState` | Selection + scroll state for args |
| `choice_select` | `Option<ChoiceSelectState>` | State for inline choice select box (None when closed); holds completion results while open |
| `edit_input` | `InputState` | State for the value editing text input |
| `click_regions` | `ClickRegionRegistry<Focus>` | Registered click targets for mouse mapping (from ratatui-interact) |

#### Key Methods

- **`current_command()`** — resolves the `command_path` to the current `SpecCommand` in the spec tree.
- **`visible_subcommands()`** — returns subcommands at the current level (test-only, used for backward compatibility).
- **`visible_flags()`** — returns all flags for the current command plus inherited global flags (no filtering; rendering applies styling).
- **`sync_state()`** — initializes or restores flag/arg values when navigating to a new command. Handles defaults. Flags with a `negate` field are initialized as `NegBool(None)` (omitted).
- **`sync_command_path_from_tree()`** — derives `command_path` from the currently selected command in the flat list and calls `sync_state()`.
- **`handle_key()`** — top-level key dispatcher. Routes to `handle_execution_key()` (when executing), `handle_editing_key()`, `handle_filter_key()`, or direct navigation/action based on current mode. The `/` key only activates filter mode for Commands, Flags, and Args panels (not Preview).
- **`handle_execution_key()`** — handles keys during command execution: forwards input to the PTY while running, closes execution view on Esc/Enter/q when the process has exited.
- **`handle_mouse()`** — maps mouse events to panel focus, item selection, scroll, and activation. Finishes any active edit before switching targets.
- **`build_command()`** — assembles the complete command string from the current state (for display).
- **`build_command_parts()`** — assembles the command as a `Vec<String>` of separate arguments for process execution. Splits the binary name on whitespace, keeps flag names and values as separate elements, and does not quote argument values.
- **`start_execution()`** / **`close_execution()`** — transition between Builder and Executing modes.
- **`write_to_pty()`** — sends byte data to the PTY writer (for forwarding keyboard input during execution).
- **`fuzzy_match_score()`** — scored fuzzy matching using `nucleo-matcher::Pattern`, returns match quality score (0 for no match). Uses `Pattern::parse()` for proper multi-word and special character support (^, $, !, ').
- **`fuzzy_match_indices()`** — returns both score and match indices for character-level highlighting. Indices are sorted and deduplicated as recommended by nucleo-matcher docs.
- **`compute_tree_match_scores()`** — computes match scores for all commands in the flat list when filtering is active; matches against the command name, aliases, help text, and the full ancestor path (e.g. "config set") so queries like "cfgset" work. Returns map of node ID → score.
- **`compute_flag_match_scores()`** — computes match scores for all flags when filtering is active; returns map of flag name → score.
- **`auto_select_next_match()`** — automatically moves selection to the first matching item when current selection doesn’t match filter. Uses the generic `find_first_match()` helper for consistent behavior across all panels.
- **`find_adjacent_match()`** — generic helper for `move_to_next_match()` / `move_to_prev_match()`. Searches forward or backward through a scored item list, wrapping around.
- **`find_first_match()`** — generic helper for `auto_select_next_match()`. Returns the current index if it already matches, or the first matching index.
- **`tree_expand_or_enter()`** — moves selection to first child of the selected command (Right/l/Enter key).
- **`tree_collapse_or_parent()`** — moves selection to the parent command (Left/h key).
- **`navigate_to_command()`** — selects a specific command by path in the flat list (used in tests).
- **`navigate_into_selected()`** / **`navigate_up()`** — select first child / select parent in the flat list.
- **`flatten_command_tree()`** — flattens the `TreeNode` hierarchy into a `Vec<FlatCommand>` with depth and full path information. All commands are always visible.
- **`ensure_visible()`** — adjusts scroll offset so the selected item is within the visible viewport.

#### Dynamic Completions

Support for the usage spec `complete` directive. When a flag or argument has no static `choices` but the current command has a matching `complete` entry, the completion command is run and results populate the select overlay.

- **`find_completion(arg_name)`** — searches the current command's `complete` map for a `SpecComplete` matching the given arg/flag name. Returns `Option<&SpecComplete>`.
- **`run_completion(cmd, descriptions)`** — executes a shell command via `sh -c`, parses output lines into `(Vec<String>, Vec<Option<String>>)`. When `descriptions` is true, lines are parsed as `value:description` pairs (with `\:` escaping). Returns `None` on failure or empty output.
- **`run_and_open_completion(arg_name, current_value)`** — runs the completion command and opens the select box with results. Re-executes the command each time for fresh results.
- **`open_completion_select(choices, descriptions)`** — opens the choice select overlay populated with completion results and their descriptions.
- **`parse_completion_line(line)`** — parses a single `value:description` line, handling `\:` escaping within values. Returns `Some((value, description))` or `None` for lines without a description separator.
- **`choice_description(idx)`** — returns the description for a choice at the given index, if present.

**Completion flow**: `handle_enter()` → no static choices? → `find_completion()` → `run_and_open_completion()` → `open_completion_select()`. On failure, falls back to `start_editing()`.

**No caching**: Completion commands are re-run each time the select box is opened to get fresh results. Results are held in memory only while the select box is open.

#### Command Tree and Flat List

The command hierarchy is built at startup by `build_command_tree()`:

1. Each subcommand becomes a child `TreeNode<CmdData>` with ID set to the space-joined command path (e.g., `"config set"`).
2. Hidden commands (`hide=true`) are filtered out.
3. `flatten_command_tree()` converts the tree into a flat `Vec<FlatCommand>` for display and navigation. Each `FlatCommand` includes:
   - `depth` — nesting level (0 for top-level commands)
   - `full_path` — space-joined ancestor names (e.g. "config set") used for fuzzy matching
   - `id` — same as the tree node ID, used for score lookups

All commands are always visible — the entire hierarchy is shown as a flat indented list. The flat list is the single source of truth for navigation, matching, and rendering.

#### State Synchronization Flow

When `sync_command_path_from_tree()` is called (on tree selection change):

1. Derive `command_path` from the selected tree node's ID (split by space).
2. Call `sync_state()` which:
   a. Looks up the current command in the spec.
   b. For each flag: if a stored value exists, preserves it; otherwise initializes from the flag's default (or `false`/empty/`0`).
   c. For each argument: if a stored value exists, preserves it; otherwise initializes with an empty value and metadata.
   d. Rebuilds the focus manager, skipping panels that have no items.

### `src/ui.rs`

The rendering module (~1950 lines including ~1150 lines of tests). Uses helpers and custom widgets from `widgets.rs` for consistent rendering.

#### Rendering Functions

- **`render()`** — top-level entry point called by the event loop. Computes layout (preview at top, main content in middle, help bar at bottom), derives colors, and delegates to sub-renderers. When in execution mode, delegates entirely to `render_execution_view()`.
- **`render_execution_view()`** — renders the execution UI: command display at top (3 rows), `PseudoTerminal` widget in the middle (fills remaining space), and status bar at bottom (1 row). Uses `tui-term::PseudoTerminal` to render the `vt100::Parser`'s screen. Shows running/finished state with appropriate border colors and status text.
- **`render_main_content()`** — splits the area into a 2-column layout: commands on the left (40%) and flags + args stacked vertically on the right (60%). When there are no subcommands, the commands panel is hidden and flags + args fill the full width.
- **`render_command_list()`** — renders the command list as a flat `List` widget with depth-based indentation (2 spaces per level). Delegates panel chrome, selection cursors, and text highlighting to `widgets.rs` helpers.
- **`render_flag_list()`** — renders flags with checkbox indicators (✓/○ for boolean, ○/✓/✗ tristate for negatable), values, defaults, global tags, and count badges. Negatable flags also display the negate string after the flag name (e.g., `--color / --no-color`).
- **`render_arg_list()`** — renders arguments with required indicators, current values, choices, and inline editing.
- **`render_preview()`** — delegates to the `CommandPreview` widget for colorized command rendering.
- **`render_help_bar()`** — delegates to the `HelpBar` widget for keybinding hints and theme indicator.
- **`render_choice_select()`** — computes overlay positioning, then delegates to `SelectList` widget.
- **`render_theme_picker()`** — computes overlay positioning, then delegates to `SelectList` widget.
- **`flag_display_string()`** — formats a single flag's display text for the list.

### `src/widgets.rs`

Reusable UI widget helpers and custom widgets (~700 lines) that eliminate duplication across renderers. Contains both helper functions used by `ui.rs` panel renderers and self-contained `Widget` trait implementations for composable UI sections.

#### Custom Widgets

These implement the ratatui `Widget` trait for self-contained, testable rendering:

- **`CommandPreview`** — renders the assembled command string in a bordered block with syntax-aware token coloring (binary name, subcommands, flags, positional arguments). Handles focus-dependent styling (▶ prefix when focused, $ when not).
- **`HelpBar`** — renders the context-sensitive help/status bar with keyboard shortcuts on the left and a right-aligned theme indicator. Exposes `theme_indicator_rect()` for mouse hit-testing.
- **`SelectList`** — renders a bordered selectable list overlay with Clear + block + items. Used by both the choice select dropdown and theme picker. Supports empty state ("no matches"), scroll offset for long lists, `Option<usize>` selection (None = no highlight), optional `▶` cursor prefix (via `with_cursor()`), configurable item/selected colors, and optional per-item descriptions (shown in help color for dynamic completions).

#### `UiColors` Struct

A semantic color palette derived from the active `ThemePalette`. Maps abstract roles (command, flag, arg, value, active border, etc.) to concrete `Color` values. Constructed via `UiColors::from_palette()`. Used by both `ui.rs` and `widgets.rs`.

#### `PanelState` Struct

Captures the shared rendering context for any list panel: focus state, scroll offset, selected index, filter text, match scores, colors, and panel area. Constructed from `App` state via `PanelState::from_app()`.

#### `ItemContext` Struct

Per-item rendering context bundling: `is_selected`, `is_match`, `name_matches` (character indices), and `help_matches` (character indices). Avoids function parameter bloat.

#### Helper Functions

- **`panel_title()`** — generates the panel title `Line`, including the 🔍 emoji and filter query when filtering is active.
- **`panel_block()`** — creates a styled `Block` with focus-aware border color and title. All panels use consistent styling without horizontal padding.
- **`push_selection_cursor()`** — appends `▶ ` or `  ` prefix spans consistently across all panels.
- **`push_highlighted_name()`** — appends item name spans with filter-aware styling (dimmed, bold+underlined, or inverted).
- **`build_help_line()`** — builds a styled `Line` for help text with filter-aware styling, for use with right-aligned overlay rendering.
- **`render_help_overlays()`** — renders right-aligned help text directly to the frame buffer after the List widget renders. Positions a precisely-sized `Paragraph` at the right edge of each row, with a 1-char leading gap. Scans the buffer to detect existing content and skips rendering when help text would overlap item content.
- **`push_edit_cursor()`** — appends inline edit cursor spans (before_cursor + ▎ + after_cursor).
- **`selection_bg()`** — returns the appropriate background color for selected/editing items.
- **`item_match_state()`** — computes whether an item matches the filter and retrieves match indices for both name and help fields.
- **`build_highlighted_text()`** — creates styled `Span` vectors from text with highlighted match character positions.

#### Click Region Registration

During rendering, each panel's `Rect` is stored in `app.click_regions` as a `(Rect, Focus)` tuple. The mouse handler uses these to determine which panel was clicked and translates the click coordinates to an item index.

#### Filtering and Match Highlighting

When filtering is active (`app.filtering == true`):

1. **Match Score Computation** — `compute_tree_match_scores()` and `compute_flag_match_scores()` use `nucleo-matcher::Pattern` to score all items against the filter pattern. Pattern-based matching properly handles multi-word patterns and fzf-style special characters. Each scoring function returns per-field scores (name score and help score separately) so that highlighting can be applied independently to each field.
2. **Separate Name/Help Matching** — The command/flag name and help text are treated as independent fields. Each is scored separately against the filter pattern. An item matches if *either* field scores > 0, but highlighting is only shown in the field(s) that independently match. This prevents confusing partial highlights in the name when an item only matched via its help text (or vice versa).
3. **Full-Path Matching** — Commands are scored against their full ancestor path (e.g. "config set") in addition to the command name, aliases, and help text. This means queries like "cfgset" will match "config set" where "set" is a subcommand of "config".
4. **Visual Styling** — Items with score = 0 (non-matches) are rendered with `Modifier::DIM` to subdue them without shifting the layout. This applies to both commands and flags.
5. **Character Highlighting** — `fuzzy_match_indices()` returns the positions of matched characters, and `build_highlighted_text()` creates styled spans. On unselected items, matches are bold+underlined. On selected items, matches use inverted colors (foreground↔background) for visibility against the selection background. Help text is also highlighted when it matches the filter, using the same styling approach.
6. **Auto-Selection** — When the filter changes, `auto_select_next_match()` moves the cursor to the first matching item if the current selection doesn’t match. This operates on the full flat command list for commands, the full flag list for flags, and the arg list for arguments.
7. **Filtered Navigation** — When a filter is active (non-empty filter text), `↑`/`↓` keys skip non-matching items and move directly to the previous/next matching item. `move_to_prev_match()` and `move_to_next_match()` scan in the appropriate direction and wrap around if needed.
8. **Panel Switching** — `set_focus()` clears the filter when changing panels (via Tab or mouse) to prevent confusion.
9. **Filter Mode Visual Cues** — When filter mode is activated, the panel title immediately shows the 🔍 emoji (e.g. "Commands 🔍" even before any text is typed). The panel border color changes to the active border color during filter mode, making it visually obvious that filtering is in progress. Panel titles show the filter query as it's typed (e.g. "Flags 🔍 roll").

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

### Test Count

199 tests total:
- **129 tests** in `app.rs` — state logic, command building, command parts, tree navigation, key handling, mouse handling, filtering, editing, tree expand/collapse, execution state management, execution key handling, global flag sync, filtered navigation, startup sync, separate name/help matching, choice select box, theme picker, backspace clear/remove, dynamic completions (parsing, execution, caching, UI flow, unified input/selection)
- **68 tests** in `ui.rs` — rendering assertions, snapshot tests, theming, click regions, filter display, filter mode visual cues, choice select rendering, theme picker rendering

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
| Shared widget helpers (`widgets.rs`) | Extracts common panel rendering patterns (borders, selection cursors, text highlighting) into reusable helpers. Eliminates ~100 lines of duplication across the three list renderers and ensures consistent behavior. |
| Custom `Widget` trait implementations | `CommandPreview`, `HelpBar`, and `SelectList` implement ratatui's `Widget` trait for self-contained, composable rendering. Positioning/layout logic stays in `ui.rs`; rendering logic lives in the widget. |
| Core modules (`app.rs`, `ui.rs`) with helpers | Core state and rendering logic stays in focused modules. Shared UI patterns extracted to `widgets.rs`, navigation helpers extracted as free functions. |
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

## Development Status

### Completed

- Project setup and spec parsing
- CLI argument parsing via `clap` (derive mode) with trailing arguments (spec command), `--spec-file`, `--cmd`, and `--usage` flags
- Usage spec output via `clap_usage` (`--usage` flag)
- Spec loading from shell commands (trailing arguments) and files (`--spec-file`)
- Base command override (`--cmd`) to set the built command prefix independently of the spec
- Core app state: navigation, flag values, arg values, command building
- Full TUI rendering: 2-column layout (commands left, flags+args stacked right), preview, help bar
- Keyboard navigation: vim keys, Tab cycling, Enter/Space/Backspace actions
- Fuzzy filtering with scored ranking, subdued non-matches, character-level match highlighting, and full-path subcommand matching (nucleo-matcher Pattern API)
- **Separate name/help matching** — name and help text are scored independently; highlighting only appears in the field that matched, preventing confusing cross-field partial highlights
- **Help text highlighting** — filter matches in help text are highlighted with the same bold+underlined (or inverted) style used for name matches
- **Two-phase filtering** — typing mode (`filtering = true`) accepts query input; pressing Enter exits typing mode but keeps the filter applied. The `filter_active()` helper checks for non-empty filter text regardless of typing mode. Filtered navigation, highlighting, and dimming all remain active as long as `filter_active()` is true. Esc, Tab, and panel switching clear the filter entirely.
- **Filtered navigation** — `↑`/`↓`/`j`/`k` skip non-matching items whenever a filter is applied (both during typing and after Enter). This lets users type a query, press Enter, then navigate matches with `j`/`k` without appending to the query.
- **Filter mode visual cues** — panel title shows 🔍 emoji whenever a filter is applied (during typing mode and after Enter). Panel border changes to active color during typing mode.
- Mouse support: click, scroll, click-to-activate
- Visual polish: theming, scrolling, default indicators, accessible symbols
- **Startup sync** — tree selection and command path are synchronized on construction, so flags/args display correctly on the first render without requiring a key press
- **Global flags from any level** — global flags toggled from any subcommand are correctly included in the built command (deepest level's value wins)
- **Command tree display** — full command hierarchy shown as a flat indented list with depth-based indentation (2 spaces per level). All commands are always visible. Left/Right (h/l) navigate to parent/first-child. Enter navigates into the selected command (same as Right). The selected command determines which flags and arguments are displayed.
- **Theme picker** — `T` opens a theme picker overlay listing all 15 themes. Themes are previewed live as the user navigates with Up/Down/j/k. Enter confirms; Esc restores the original theme. Clicking the `[ThemeName]` indicator in the help bar also opens the picker. Mouse click on a theme confirms it; click outside cancels. The `]`/`[` keys still cycle themes directly without opening the picker.
- **Print-only mode removed** — the `p` key and `Accept` action were removed; all execution happens via `Ctrl+R` or Enter on Preview.
- **Command execution** — execute built commands in an embedded PTY terminal directly within the TUI. Commands are spawned with separate process arguments (not shell-stringified) via `portable-pty`. Terminal output is rendered in real-time using `tui-term::PseudoTerminal`. Keyboard input is forwarded to the running process. The execution view shows the command at the top, terminal output in the middle, and a status bar at the bottom. After the process exits, the user closes the view to return to the command builder for building and running additional commands.
- **PTY resize** — dynamically resize the embedded terminal when the TUI window is resized during execution. The PTY master is stored in `ExecutionState` and resized via `app.resize_pty()` when `Event::Resize` is received during execution mode. The vt100 parser screen is resized in place via `screen_mut().set_size()`, preserving content without flashing, while the child process receives SIGWINCH to redraw.
- **Command Preview at top** — the command preview pane is positioned at the top of the screen (matching the execution view layout), providing visual stability when switching between builder and execution modes.
- **Ctrl+R execute shortcut** — execute the built command from any panel via `Ctrl+R` (replaces `Ctrl+Enter` which could be intercepted by terminal emulators). Works regardless of edit or filter mode.
- **Args filter auto-select** — when filtering in the Arguments panel, the cursor automatically moves to the first matching argument if the current selection doesn't match the filter.
- **Inline choice select box** — flags and arguments with predefined choices open an inline select box overlay instead of cycling through options. The select box supports fuzzy filtering (non-matching choices are hidden), navigation with Up/Down, confirmation with Enter, and cancellation with Esc. The current value is pre-selected when the box opens.
- **Dynamic completions** — args and flags with `complete` directives in the usage spec run shell commands to populate the select overlay. Supports `descriptions=#true` for `value:description` pairs. Commands are re-run each time for fresh results. Esc falls back to free-text editing. Failed commands silently fall back to text editing.
- Comprehensive test suite (197 tests)
- Zero clippy warnings

### Remaining Work

- **Clipboard copy** — copy the built command to the system clipboard from within the TUI
- **Embedded USAGE blocks** — verify and test support for script files with heredoc USAGE blocks via `--spec-file`
- **Module splitting** — break `app.rs` into `state`, `input`, `builder` sub-modules; consider migrating remaining panel renderers (command list, flag list, arg list) to custom `Widget` implementations
- **CI pipeline** — GitHub Actions for `cargo test`, `cargo clippy`, and `insta` snapshot checks
