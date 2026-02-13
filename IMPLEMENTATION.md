# Implementation

This document describes how TuiSage is built — its architecture, code structure, key design decisions, and current development state. It complements the behavioral specification in SPECIFICATION.md with concrete implementation details.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   main.rs                       │
│  - CLI arg parsing (clap derive)                │
│  - --usage output (clap_usage)                  │
│  - Spec loading (--spec-cmd or --spec-file)     │
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
│  - Semantic color derivation from theme palette │
│  - Component rendering (commands, flags, args)  │
│  - Command preview colorization                 │
│  - Execution view (PseudoTerminal + status)     │
│  - Click region registration                    │
└─────────────────────────────────────────────────┘
```

## Source Files

### `src/main.rs`

The entry point. Responsibilities:

- Parse CLI arguments via `clap` (derive mode) into an `Args` struct with `--spec-cmd`, `--spec-file`, `--cmd`, and `--usage` flags.
- Handle `--usage` by generating TuiSage's own usage spec via `clap_usage::generate()` and exiting.
- Validate that exactly one of `--spec-cmd` or `--spec-file` is provided.
- Load the usage spec: run a shell command (`sh -c` / `cmd /C`) for `--spec-cmd`, or read a file for `--spec-file`.
- Override the spec's `bin` field if `--cmd` is provided.
- Enable mouse capture via `crossterm::event::EnableMouseCapture`.
- Initialize the ratatui `DefaultTerminal`.
- Run the event loop (`run_event_loop`), which draws frames and dispatches events.
- Spawn commands in a PTY via `spawn_command()` when the `Execute` action is triggered.
- On exit, restore the terminal, disable mouse capture, and optionally print the accepted command to stdout.

The event loop has two modes:
- **Builder mode**: Blocking event read — delegates to `app.handle_key()` or `app.handle_mouse()`, which return an `Action` enum (`None`, `Quit`, `Accept`, or `Execute`).
- **Execution mode**: Polling event read (16ms interval) — forwards keyboard input to the PTY, continuously redraws to show live terminal output.

The `spawn_command()` function:
1. Calls `app.build_command_parts()` to get separate process arguments.
2. Creates a PTY pair via `portable-pty::NativePtySystem`, sized to fit the terminal area.
3. Spawns the command on the PTY slave using `CommandBuilder` with individual args (not shell-stringified).
4. Starts background threads for: reading PTY output into `vt100::Parser`, waiting for child exit, and cleanup.
5. Sets up the `ExecutionState` and switches the app to `AppMode::Executing`.

### `src/app.rs`

The core state and logic module (~2900 lines including ~1200 lines of tests).

#### Key Types

- **`Action`** — return value from event handlers: `None`, `Quit`, `Accept`, `Execute`.
- **`AppMode`** — enum: `Builder` (normal command-building UI) or `Executing` (embedded terminal running).
- **`ExecutionState`** — struct holding PTY state: `command_display`, `parser` (vt100), `pty_writer`, `exited` flag, `exit_status`.
- **`Focus`** — enum of focusable panels: `Commands`, `Flags`, `Args`, `Preview`.
- **`FlagValue`** — discriminated union: `Bool(bool)`, `String(String)`, `Count(u32)`.
- **`ArgValue`** — struct with `name`, `value`, `required`, `choices` fields.
- **`CmdData`** — data stored in each tree node: `name`, `help`, `aliases`.
- **`App`** — the main application state struct.

#### `App` Struct Fields

| Field | Type | Purpose |
|---|---|---|
| `spec` | `usage::Spec` | The parsed usage specification |
| `mode` | `AppMode` | Current app mode (Builder or Executing) |
| `execution` | `Option<ExecutionState>` | Execution state when a command is running |
| `theme_name` | `String` | Current color theme name |
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
| `edit_input` | `InputState` | State for the value editing text input |
| `click_regions` | `ClickRegionRegistry<Focus>` | Registered click targets for mouse mapping (from ratatui-interact) |

#### Key Methods

- **`current_command()`** — resolves the `command_path` to the current `SpecCommand` in the spec tree.
- **`visible_subcommands()`** — returns subcommands at the current level (test-only, used for backward compatibility).
- **`visible_flags()`** — returns all flags for the current command plus inherited global flags (no filtering; rendering applies styling).
- **`sync_state()`** — initializes or restores flag/arg values when navigating to a new command. Handles defaults.
- **`sync_command_path_from_tree()`** — derives `command_path` from the currently selected command in the flat list and calls `sync_state()`.
- **`handle_key()`** — top-level key dispatcher. Routes to `handle_execution_key()` (when executing), `handle_editing_key()`, `handle_filter_key()`, or direct navigation/action based on current mode.
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
- **`auto_select_next_match()`** — automatically moves selection to the first matching item when current selection doesn't match filter. Operates on the full flat command list.
- **`tree_expand_or_enter()`** — moves selection to first child of the selected command (Right/l/Enter key).
- **`tree_collapse_or_parent()`** — moves selection to the parent command (Left/h key).
- **`navigate_to_command()`** — selects a specific command by path in the flat list (used in tests).
- **`navigate_into_selected()`** / **`navigate_up()`** — select first child / select parent in the flat list.
- **`flatten_command_tree()`** — flattens the `TreeNode` hierarchy into a `Vec<FlatCommand>` with depth and full path information. All commands are always visible.
- **`ensure_visible()`** — adjusts scroll offset so the selected item is within the visible viewport.

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

The rendering module (~2070 lines including ~1065 lines of tests).

#### `UiColors` Struct

A semantic color palette derived from the active `ThemePalette`. Maps abstract roles (command, flag, arg, value, active border, etc.) to concrete `Color` values. Constructed via `UiColors::from_palette()`.

#### Rendering Functions

- **`render()`** — top-level entry point called by the event loop. Computes layout, derives colors, and delegates to sub-renderers. When in execution mode, delegates entirely to `render_execution_view()`.
- **`render_execution_view()`** — renders the execution UI: command display at top (3 rows), `PseudoTerminal` widget in the middle (fills remaining space), and status bar at bottom (1 row). Uses `tui-term::PseudoTerminal` to render the `vt100::Parser`'s screen. Shows running/finished state with appropriate border colors and status text.
- **`render_main_content()`** — splits the area into a 2-column layout: commands on the left (40%) and flags + args stacked vertically on the right (60%). When there are no subcommands, the commands panel is hidden and flags + args fill the full width.
- **`render_command_list()`** — renders the command list as a flat `List` widget with depth-based indentation (2 spaces per level). Supports per-item styling: selected items get bold text, non-matching items are dimmed during filtering, and matching characters are highlighted with inverted colors on selected items or bold+underlined on unselected items.
- **`render_flag_list()`** — renders flags with checkbox indicators (✓/○), values, defaults, global tags, and count badges.
- **`render_arg_list()`** — renders arguments with required indicators, current values, choices, and inline editing.
- **`render_preview()`** — renders the colorized command preview with `▶ RUN` or `$` prefix based on focus.
- **`render_help_bar()`** — renders contextual help and key hints at the bottom.
- **`colorize_command()`** — parses the built command string and applies per-token coloring.
- **`flag_display_string()`** — formats a single flag's display text for the list.

#### Click Region Registration

During rendering, each panel's `Rect` is stored in `app.click_regions` as a `(Rect, Focus)` tuple. The mouse handler uses these to determine which panel was clicked and translates the click coordinates to an item index.

#### Filtering and Match Highlighting

When filtering is active (`app.filtering == true`):

1. **Match Score Computation** — `compute_tree_match_scores()` and `compute_flag_match_scores()` use `nucleo-matcher::Pattern` to score all items against the filter pattern. Pattern-based matching properly handles multi-word patterns and fzf-style special characters.
2. **Full-Path Matching** — Commands are scored against their full ancestor path (e.g. "config set") in addition to the command name, aliases, and help text. This means queries like "cfgset" will match "config set" where "set" is a subcommand of "config".
3. **Visual Styling** — Items with score = 0 (non-matches) are rendered with `Modifier::DIM` to subdue them without shifting the layout. This applies to both commands and flags.
4. **Character Highlighting** — `fuzzy_match_indices()` returns the positions of matched characters, and `build_highlighted_text()` creates styled spans. On unselected items, matches are bold+underlined. On selected items, matches use inverted colors (foreground↔background) for visibility against the selection background.
5. **Auto-Selection** — When the filter changes, `auto_select_next_match()` moves the cursor to the first matching item if the current selection doesn't match. This operates on the full flat command list for commands, and the full flag list for flags.
6. **Panel Switching** — `set_focus()` clears the filter when changing panels (via Tab or mouse) to prevent confusion.
7. **Clean Headers** — Panel titles show just the panel name (e.g. "Commands", "Flags", "Arguments") without counts. When filtering is active, the filter query is shown (e.g. "Flags (/roll)").

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

127 tests total:
- **70 tests** in `app.rs` — state logic, command building, command parts, tree navigation, key handling, mouse handling, filtering, editing, tree expand/collapse, execution state management, execution key handling
- **57 tests** in `ui.rs` — rendering assertions, snapshot tests, theming, click regions, filter display

### Test Fixtures

The primary test fixture is `fixtures/sample.usage.kdl`, which covers:

- Nested subcommands (2–3 levels deep: `plugin > install/remove`)
- Boolean flags (`--force`, `--verbose`, `--rollback`)
- Flags with values (`--tag <tag>`, `--output <file>`)
- Required and optional positional arguments
- Flags with choices (`environment: dev|staging|prod`)
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
| Single-file modules (`app.rs`, `ui.rs`) | Keeps things simple while the codebase is small. Split when complexity warrants it. |
| `Vec<String>` for command path | Simple push/pop navigation through the command tree. Joined with `>` for hash map keys. |
| State preserved across navigation | Flag/arg values are stored per command-path key, so navigating away and back retains previous selections. |
| Rendering to stderr, output to stdout | Enables composability: `eval "$(tuisage spec.kdl)"` works because TUI output doesn't mix with the command string. |
| Crossterm backend | Best cross-platform terminal support (macOS, Linux, Windows). |
| Theme palette mapping | Instead of hardcoding colors, derive semantic colors from the active theme. This makes all themes work automatically. |
| `TreeViewState` + `TreeNode` from ratatui-interact | Provides the command hierarchy data model. Used as a flat indented list (all nodes always visible) with selection and scroll offset tracking. |
| `ListPickerState` from ratatui-interact | Provides selection index + scroll offset tracking for flags and args panels. |
| `FocusManager` from ratatui-interact | Handles Tab/Shift-Tab cycling with dynamic panel availability, reducing boilerplate. |
| Finishing edits on focus change | Prevents a class of bugs where the edit input text leaks into the wrong field when clicking elsewhere. |
| Commands list always visible | The flat indented list shows the entire command hierarchy, not just subcommands of the current selection. It remains visible even when navigating to leaf commands with no children, providing constant wayfinding context. |
| Arguments panel visibility based on spec | The arguments panel is shown whenever the current command defines arguments in the spec, ensuring consistent visibility regardless of whether arg values are populated. |

## Development Status

### Completed

- Project setup and spec parsing
- CLI argument parsing via `clap` (derive mode) with `--spec-cmd`, `--spec-file`, `--cmd`, and `--usage` flags
- Usage spec output via `clap_usage` (`--usage` flag)
- Spec loading from shell commands (`--spec-cmd`) and files (`--spec-file`)
- Base command override (`--cmd`) to set the built command prefix independently of the spec
- Core app state: navigation, flag values, arg values, command building
- Full TUI rendering: 2-column layout (commands left, flags+args stacked right), preview, help bar
- Keyboard navigation: vim keys, Tab cycling, Enter/Space/Backspace actions
- Fuzzy filtering with scored ranking, subdued non-matches, character-level match highlighting, and full-path subcommand matching (nucleo-matcher Pattern API)
- Mouse support: click, scroll, click-to-activate
- Visual polish: theming, scrolling, default indicators, accessible symbols
- **Command tree display** — full command hierarchy shown as a flat indented list with depth-based indentation (2 spaces per level). All commands are always visible. Left/Right (h/l) navigate to parent/first-child. Enter navigates into the selected command (same as Right). The selected command determines which flags and arguments are displayed.
- **Theme cycling** — `]`/`[` keys cycle through themes forward/backward, `T` also cycles forward. Current theme name shown in status bar.
- **Print-only mode** — press `p` when the Preview panel is focused to output the command to stdout and exit, enabling piping and shell integration.
- **Command execution** — execute built commands in an embedded PTY terminal directly within the TUI. Commands are spawned with separate process arguments (not shell-stringified) via `portable-pty`. Terminal output is rendered in real-time using `tui-term::PseudoTerminal`. Keyboard input is forwarded to the running process. The execution view shows the command at the top, terminal output in the middle, and a status bar at the bottom. After the process exits, the user closes the view to return to the command builder for building and running additional commands.
- Comprehensive test suite (127+ tests)
- Zero clippy warnings

### Remaining Work

- **PTY resize** — dynamically resize the embedded terminal when the TUI window is resized during execution
- **Clipboard copy** — copy the built command to the system clipboard from within the TUI
- **Embedded USAGE blocks** — verify and test support for script files with heredoc USAGE blocks via `--spec-file`
- **Module splitting** — break `app.rs` into `state`, `input`, `builder` sub-modules; break `ui.rs` into widget modules
- **CI pipeline** — GitHub Actions for `cargo test`, `cargo clippy`, and `insta` snapshot checks
