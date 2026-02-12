# Implementation

This document describes how TuiSage is built — its architecture, code structure, key design decisions, and current development state. It complements the behavioral specification in SPECIFICATION.md with concrete implementation details.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   main.rs                       │
│  - CLI arg parsing (file path or stdin)         │
│  - Parse usage spec via usage-lib               │
│  - Terminal setup (ratatui + mouse capture)      │
│  - Event loop (key, mouse, resize)              │
│  - Terminal teardown and command output          │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                   app.rs                        │
│  - App struct (holds Spec + all runtime state)  │
│  - Command path navigation (Vec<String>)        │
│  - Flag/arg value storage (HashMaps)            │
│  - Focus management (via ratatui-interact)      │
│  - Key event handling → Action dispatch         │
│  - Mouse event handling → Action dispatch       │
│  - Command string builder                       │
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
│  - Click region registration                    │
└─────────────────────────────────────────────────┘
```

## Source Files

### `src/main.rs`

The entry point. Responsibilities:

- Parse CLI arguments to determine the spec source (file path, stdin, or piped input).
- Use `usage::Spec::parse_file()` or `String::parse::<usage::Spec>()` to load the spec.
- Enable mouse capture via `crossterm::event::EnableMouseCapture`.
- Initialize the ratatui `DefaultTerminal`.
- Run the event loop (`run_event_loop`), which draws frames and dispatches events.
- On exit, restore the terminal, disable mouse capture, and optionally print the accepted command to stdout.

The event loop is intentionally thin — it reads crossterm events and delegates to `app.handle_key()` or `app.handle_mouse()`, which return an `Action` enum (`None`, `Quit`, or `Accept`).

### `src/app.rs`

The core state and logic module (~2050 lines including ~960 lines of tests).

#### Key Types

- **`Action`** — return value from event handlers: `None`, `Quit`, `Accept`.
- **`Focus`** — enum of focusable panels: `Commands`, `Flags`, `Args`, `Preview`.
- **`FlagValue`** — discriminated union: `Bool(bool)`, `String(String)`, `Count(u32)`.
- **`ArgValue`** — struct with `name`, `value`, `required`, `choices` fields.
- **`CmdData`** — data stored in each tree node: `name`, `help`, `aliases`.
- **`App`** — the main application state struct.

#### `App` Struct Fields

| Field | Type | Purpose |
|---|---|---|
| `spec` | `usage::Spec` | The parsed usage specification |
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
| `click_regions` | `Vec<(Rect, Focus)>` | Registered click targets for mouse mapping |

#### Key Methods

- **`current_command()`** — resolves the `command_path` to the current `SpecCommand` in the spec tree.
- **`visible_subcommands()`** — returns subcommands at the current level, filtered if filter mode is active.
- **`visible_flags()`** — returns flags for the current command plus inherited global flags, filtered if active.
- **`sync_state()`** — initializes or restores flag/arg values when navigating to a new command. Handles defaults.
- **`sync_command_path_from_tree()`** — derives `command_path` from the currently selected tree node and calls `sync_state()`.
- **`handle_key()`** — top-level key dispatcher. Routes to `handle_editing_key()`, `handle_filter_key()`, or direct navigation/action based on current mode.
- **`handle_mouse()`** — maps mouse events to panel focus, item selection, scroll, and activation. Finishes any active edit before switching targets.
- **`build_command()`** — assembles the complete command string from the current state.
- **`fuzzy_match_score()`** — scored fuzzy matching using `nucleo-matcher`, returns match quality score.
- **`filter_tree_nodes()`** — recursively filters tree nodes preserving ancestors, sorted by relevance score.
- **`tree_toggle_selected()`** — toggles expand/collapse of the selected tree node (Enter key on Commands).
- **`tree_expand_or_enter()`** — expands a collapsed node or moves to its first child (Right/l key).
- **`tree_collapse_or_parent()`** — collapses an expanded node or moves to its parent (Left/h key).
- **`navigate_to_command()`** — expands ancestors and selects a specific command by path (used in tests).
- **`navigate_into_selected()`** / **`navigate_up()`** — backward-compatible wrappers for tree expand+select-child / select-parent.
- **`ensure_visible()`** — adjusts scroll offset so the selected item is within the visible viewport.

#### Tree Building

The command hierarchy is built at startup by `build_command_tree()`:

1. The root node (ID `""`) represents the binary name and the root command's flags/args.
2. Each subcommand becomes a child `TreeNode<CmdData>` with ID set to the space-joined command path (e.g., `"config set"`).
3. Hidden commands (`hide=true`) are filtered out.
4. Non-root nodes with children are initially collapsed, so the tree starts showing only the top-level subcommands.

Helper functions `flatten_visible_ids()`, `flatten_visible_nodes()`, and `count_visible()` traverse the tree respecting collapsed state.

#### State Synchronization Flow

When `sync_command_path_from_tree()` is called (on tree selection change):

1. Derive `command_path` from the selected tree node's ID (split by space).
2. Call `sync_state()` which:
   a. Looks up the current command in the spec.
   b. For each flag: if a stored value exists, preserves it; otherwise initializes from the flag's default (or `false`/empty/`0`).
   c. For each argument: if a stored value exists, preserves it; otherwise initializes with an empty value and metadata.
   d. Rebuilds the focus manager, skipping panels that have no items.

### `src/ui.rs`

The rendering module (~1955 lines including ~1065 lines of tests).

#### `UiColors` Struct

A semantic color palette derived from the active `ThemePalette`. Maps abstract roles (command, flag, arg, value, active border, etc.) to concrete `Color` values. Constructed via `UiColors::from_palette()`.

#### Rendering Functions

- **`render()`** — top-level entry point called by the event loop. Computes layout, derives colors, and delegates to sub-renderers.
- **`render_main_content()`** — splits the area into up to 3 columns (commands, flags, args). Commands tree is always visible. Hides flags/args panels with no content and redistributes space.
- **`render_command_list()`** — renders the command tree using ratatui-interact's `TreeView` widget with themed `TreeStyle`, showing expand/collapse icons, tree connectors, and aliases.
- **`render_flag_list()`** — renders flags with checkbox indicators (✓/○), values, defaults, global tags, and count badges.
- **`render_arg_list()`** — renders arguments with required indicators, current values, choices, and inline editing.
- **`render_preview()`** — renders the colorized command preview with `▶ RUN` or `$` prefix based on focus.
- **`render_help_bar()`** — renders contextual help and key hints at the bottom.
- **`colorize_command()`** — parses the built command string and applies per-token coloring.
- **`flag_display_string()`** — formats a single flag's display text for the list.

#### Click Region Registration

During rendering, each panel's `Rect` is stored in `app.click_regions` as a `(Rect, Focus)` tuple. The mouse handler uses these to determine which panel was clicked and translates the click coordinates to an item index.

## Dependencies

| Crate | Version | Purpose | Notes |
|---|---|---|---|
| `usage-lib` | 2.16 | Parse `.usage.kdl` specs | `default-features = false` (skip docs/tera/roff) |
| `ratatui` | 0.30 | TUI framework | Provides `Frame`, `Terminal`, widgets, layout |
| `crossterm` | 0.29 | Terminal backend + events | `event-stream` feature enabled |
| `ratatui-interact` | 0.4 | UI components | `TreeView`, `TreeNode`, `FocusManager`, `ListPickerState` |
| `ratatui-themes` | 0.1 | Color theming | Provides `ThemePalette` with named themes |
| `nucleo-matcher` | 0.3 | Fuzzy matching | Provides scored matching with smart case and normalization |
| `color-eyre` | 0.6 | Error reporting | Pretty error messages with backtraces |
| `insta` | 1 | Snapshot testing (dev) | Full terminal output comparison |
| `pretty_assertions` | 1 | Test diffs (dev) | Better assertion failure output |

### Why `usage-lib` With No Default Features

The `usage-lib` crate includes optional features for generating documentation, man pages (roff), and templates (tera). TuiSage only needs spec parsing, so these are disabled to minimize compile time and binary size.

## Test Infrastructure

### Test Count

111 tests total:
- **59 tests** in `app.rs` — state logic, command building, tree navigation, key handling, mouse handling, filtering, editing, tree expand/collapse
- **52 tests** in `ui.rs` — 19 snapshot tests + 33 assertion-based rendering tests

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
| `TreeViewState` + `TreeNode` from ratatui-interact | Provides the full command hierarchy as a collapsible tree with selection, expand/collapse state, and scroll offset. |
| `ListPickerState` from ratatui-interact | Provides selection index + scroll offset tracking for flags and args panels. |
| `FocusManager` from ratatui-interact | Handles Tab/Shift-Tab cycling with dynamic panel availability, reducing boilerplate. |
| Finishing edits on focus change | Prevents a class of bugs where the edit input text leaks into the wrong field when clicking elsewhere. |
| Commands tree always visible | The tree shows the entire command hierarchy, not just subcommands of the current selection. It remains visible even when navigating to leaf commands with no children. |
| Arguments panel visibility based on spec | The arguments panel is shown whenever the current command defines arguments in the spec, ensuring consistent visibility regardless of whether arg values are populated. |

## Development Status

### Completed

- Project setup and spec parsing
- Core app state: navigation, flag values, arg values, command building
- Full TUI rendering: 3-panel layout, preview, help bar
- Keyboard navigation: vim keys, Tab cycling, Enter/Space/Backspace actions
- Fuzzy filtering for commands and flags with scored ranking (nucleo-matcher)
- Mouse support: click, scroll, right-click, click-to-activate
- Visual polish: theming, scrolling, default indicators, accessible symbols
- Stdin support for piped specs
- **TreeView display** — full command hierarchy shown as an expandable/collapsible tree using `ratatui-interact`'s `TreeView` widget, with tree connectors, expand/collapse icons, and keyboard/mouse navigation (Left/Right to collapse/expand, Enter to toggle). Top-level commands are direct tree nodes (no root wrapper), maximizing screen space and simplifying navigation.
- Comprehensive test suite (111 tests)
- Zero clippy warnings

### Remaining Work

- **Inline aliases** — show command aliases with dimmed styling in the command list
- **Command execution** — execute built commands directly within the TUI, remain open for building next command
- **Print-only mode** — optional keybinding to output command to stdout for shell integration
- **Clipboard copy** — copy the built command to the system clipboard from within the TUI
- **Embedded USAGE blocks** — verify and test support for script files with heredoc USAGE blocks
- **Module splitting** — break `app.rs` into `state`, `input`, `builder` sub-modules; break `ui.rs` into widget modules
- **CI pipeline** — GitHub Actions for `cargo test`, `cargo clippy`, and `insta` snapshot checks