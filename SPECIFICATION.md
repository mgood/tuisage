# Specification

This document defines the detailed behavior of TuiSage — how features work, how the UI is structured, and how user interactions are handled. It bridges the high-level goals in REQUIREMENTS.md and the implementation details in IMPLEMENTATION.md.

## Library Preferences

These are the preferred libraries for implementing TuiSage features, as specified during initial development:

| Feature | Preferred Library | Notes |
|---|---|---|
| TUI framework | `ratatui` | Terminal UI rendering and layout |
| Terminal backend | `crossterm` | Cross-platform terminal control and events |
| UI components | `ratatui-interact` | Breadcrumb, Input, FocusManager, ListPickerState, TreeView |
| Color theming | `ratatui-themes` | Consistent theme palettes |
| Usage spec parsing | `usage-lib` | Parse `.usage.kdl` files (no default features) |
| Fuzzy matching | `nucleo-matcher` | Pattern-based fzf-style scoring with multi-word and special character support |
| Error reporting | `color-eyre` | Pretty error messages with context |
| Snapshot testing | `insta` | Terminal output comparison (dev dependency) |

## Input Handling

### CLI Arguments

TuiSage accepts a single positional argument specifying the source of the usage spec:

| Invocation | Behavior |
|---|---|
| `tuisage <path>` | Parse the file at `<path>` as a usage spec (`.usage.kdl` or a script with embedded `USAGE` block) |
| `tuisage -` | Read the usage spec from stdin (explicit) |
| `tuisage` (stdin piped) | Detect non-TTY stdin and read the spec from it |
| `tuisage` (no stdin) | Print usage help to stderr and exit with code 1 |

Parsing errors produce a descriptive error message via `color-eyre` and exit non-zero.

### Spec Parsing

The usage spec is parsed via `usage-lib` into a `Spec` struct that provides:

- Binary name
- Top-level flags and arguments
- A tree of subcommands, each with their own flags and arguments
- Flag metadata: long/short names, whether they take values, choices, defaults, aliases, count mode
- Argument metadata: name, required/optional, choices

## UI Layout

The terminal is divided into the following regions, rendered top-to-bottom:

```
┌──────────────────────────────────────────────────┐
│ Breadcrumb Bar                                   │
├─────────────────┬────────────────┬───────────────┤
│                 │                │               │
│   Commands      │     Flags      │   Arguments   │
│   Panel         │     Panel      │   Panel       │
│                 │                │               │
├─────────────────┴────────────────┴───────────────┤
│ Command Preview                                  │
├──────────────────────────────────────────────────┤
│ Help / Status Bar                                │
└──────────────────────────────────────────────────┘
```

### Breadcrumb Bar

- Displays the current navigation path through the command tree.
- Format: `binary > subcommand > nested-subcommand`
- Always shows at least the binary name from the spec.
- Uses the `Breadcrumb` component from `ratatui-interact`.

### Commands Panel

- Displays the full command hierarchy as a flat list with depth-based indentation (2 spaces per level).
- All commands and subcommands are always visible — there is no expand/collapse.
- Each item shows the command name and its `help` text (if available).
- Aliases are shown alongside the command name (e.g., `remove (rm)`).
- Left arrow (←/h) moves selection to the parent command; Right arrow (→/l) moves to the first child.
- The selected command determines which flags and arguments are displayed in the other panels.
- When filtering is active, all commands remain visible:
  - **Non-matching commands** are displayed in a dimmed/subdued color
  - **Matching commands** are displayed normally, with matching characters highlighted (bold+underlined on unselected items, inverted colors on the selected item)
  - **Matching includes the full ancestor path** so that queries like "cfgset" match "config set"
  - **Selected command** is always shown with bold styling and cursor indicator
- The panel title shows just "Commands" (no counts), or "Commands (/query)" when filtering is active.

### Flags Panel

- Lists all flags available for the currently selected command, including global (inherited) flags.
- Each flag shows:
  - A checkbox indicator: `✓` (enabled) or `○` (disabled) for boolean flags
  - The flag name (long form preferred, short form shown alongside)
  - Current value for value-bearing flags
  - `(default: X)` indicator when a default value exists
  - `[G]` indicator for inherited global flags
  - Count value for count flags (e.g., `[3]`)
- Flags with choices show the current selection.
- The panel title shows just "Flags" (no counts), or "Flags (/query)" when filtering is active.

### Arguments Panel

- The panel title shows just "Arguments" (no counts).
- Lists positional arguments for the currently selected command.
- Each argument shows:
  - The argument name
  - `(required)` indicator for required arguments
  - Current value or placeholder
  - Available choices if defined
- When editing, the argument field becomes an active text input.

### Command Preview

- Shows the fully assembled command string as it would be output.
- Updates in real time as the user toggles flags, fills values, and navigates.
- When focused: displays a `▶ RUN` indicator to signal that Enter will accept the command.
- When unfocused: displays a `$` prompt prefix.
- The command is colorized: binary name, subcommands, flags, and values each get distinct colors.

### Help / Status Bar

- Shows contextual help text for the currently hovered item.
- Displays available keyboard shortcuts.
- Shows the current theme name.

## Focus System

The UI has four focusable panels, cycled with Tab/Shift-Tab:

1. **Commands** — subcommand list
2. **Flags** — flag list
3. **Args** — argument list
4. **Preview** — command preview

Focus determines which panel receives keyboard input. The focused panel has an active border style; unfocused panels have a dimmer border.

Focus is managed via `ratatui-interact`'s `FocusManager`. The focus order is rebuilt when navigating to a new command (panels with no items are skipped).

### Focus Rules

- If a command has no subcommands, the Commands panel is skipped in the focus order.
- If a command has no flags, the Flags panel is skipped.
- If a command has no arguments, the Args panel is skipped.
- Preview is always focusable.
- On navigation (entering/leaving a subcommand), focus resets to the first available panel.

## State Management

### Command Selection

The currently selected command in the tree view determines which flags and arguments are displayed. The tree view maintains its own state for:

- Which node is currently selected (cursor position)
- Which nodes are expanded (showing their children)
- Scroll offset for long trees

The selected command's full path (e.g., `["config", "set"]`) is computed from the tree structure to look up flag and argument values.

### Flag Values

Flag values are stored in a `HashMap` keyed by a command-path string (e.g., `"deploy"` or `"plugin>install"`). Each command's flags are stored as a `HashMap<String, FlagValue>` where:

- `FlagValue::Bool(bool)` — for boolean/toggle flags
- `FlagValue::String(String)` — for flags that take a value
- `FlagValue::Count(u32)` — for count flags (e.g., `-vvv`)

### Argument Values

Argument values are stored in a `Vec<ArgValue>` per command path, preserving positional order. Each `ArgValue` contains:

- `name` — argument name
- `value` — current value (empty string if unset)
- `required` — whether the argument is required
- `choices` — available choices (empty vec if free-text)

### State Synchronization

When the user navigates to a new command, the state is synchronized:

- Flag values are initialized from defaults (or preserved if previously set).
- Argument values are initialized (or preserved if previously set).
- List indices are reset to 0.
- Scroll offsets are reset.
- The focus manager is rebuilt based on available panels.

## Keyboard Interactions

### Global Keys

| Key | Action |
|---|---|
| `Ctrl-C` | Quit immediately (no output) |
| `q` | Quit (when not editing or filtering) |
| `Esc` | Context-dependent: cancel filter → cancel edit → move to parent command → quit |

### Navigation Keys

| Key | Action |
|---|---|
| `↑` / `k` | Move selection up in the focused panel |
| `↓` / `j` | Move selection down in the focused panel |
| `←` / `h` | Collapse the selected tree node (or move to parent if already collapsed) |
| `→` / `l` | Expand the selected tree node (or move to first child if already expanded) |
| `Tab` | Cycle focus to the next panel |
| `Shift-Tab` | Cycle focus to the previous panel |

### Action Keys

| Key | Context | Action |
|---|---|---|
| `Enter` | Commands panel | Toggle expand/collapse of the selected tree node |
| `Enter` | Flags panel (boolean) | Toggle the flag |
| `Enter` | Flags panel (value) | Start editing the flag value |
| `Enter` | Flags panel (choices) | Cycle to the next choice |
| `Enter` | Args panel | Start editing the argument / cycle choice |
| `Enter` | Preview panel | Execute the built command |
| `Space` | Flags panel (boolean) | Toggle the flag |
| `Space` | Flags panel (count) | Increment the count |
| `Backspace` | Flags panel (count) | Decrement the count (floor at 0) |
| `/` | Commands or Flags panel | Activate fuzzy filter mode |

### Editing Mode Keys

When editing a text value (flag value or argument):

| Key | Action |
|---|---|
| Any character | Append to the input |
| `Backspace` | Delete the last character |
| `Enter` | Confirm the value and exit editing mode |
| `Esc` | Cancel editing and exit editing mode |

### Filter Mode Keys

When the fuzzy filter is active:

| Key | Action |
|---|---|
| Any character | Append to the filter query; auto-select next matching item if current item doesn't match |
| `Backspace` | Delete the last character from the query; auto-select next matching item if current item doesn't match |
| `Esc` | Clear the filter and exit filter mode |
| `↑` / `↓` | Navigate within all results (matching items displayed normally, non-matching items subdued) |
| `Enter` | Apply the filter and exit filter mode |
| `Tab` | Switch focus to the other panel and clear the filter |

### Theme Keys

| Key | Action |
|---|---|
| `]` | Switch to the next theme |
| `[` | Switch to the previous theme |

## Mouse Interactions

Mouse support is enabled via crossterm's `EnableMouseCapture`.

| Action | Effect |
|---|---|
| Left click on a panel | Focus that panel and select the clicked item |
| Left click on an already-selected item | Activate it (toggle expand/collapse for tree nodes, same as Enter) |
| Right click on a command | Expand the tree node |
| Scroll wheel up | Move selection up in the panel under the cursor |
| Scroll wheel down | Move selection down in the panel under the cursor |

### Click Region Tracking

The UI registers click regions during rendering so that mouse coordinates can be mapped back to the correct panel and item index. Regions are stored as `(Rect, Focus)` tuples and cleared/rebuilt on each render.

### Edit Finishing on Mouse

If the user is currently editing a value and clicks on a different item or panel, the edit is finished (committed) before processing the new click. This prevents the edit input text from "bleeding" into a different field.

## Fuzzy Filtering

Filtering uses scored fuzzy-matching (powered by `nucleo-matcher::Pattern`):

1. The user presses `/` to activate filter mode in the Commands or Flags panel.
2. The panel title shows the filter query (e.g., `Commands (/query)` or `Flags (/roll)`). No counts are shown.
3. As the user types, items are scored against the filter pattern using `Pattern::parse()`:
   - **Pattern matching** supports multi-word patterns (whitespace-separated) and fzf-style special characters (^, $, !, ')
   - **Matching items** (score > 0) are displayed in normal color with matching characters highlighted (bold+underlined on unselected items, inverted colors on the selected item)
   - **Non-matching items** (score = 0) are displayed in a dimmed/subdued color without shifting the layout
   - Match indices are sorted and deduplicated for accurate character-level highlighting
4. **Full-path matching**: In the Commands panel, subcommands are matched against their full ancestor path (e.g. "config set") so that queries like "cfgset" match subcommands via their parent chain.
5. All commands remain visible for context — the flat list structure is preserved.
6. **Auto-selection**: If the currently selected item doesn't match the filter, the selection automatically moves to the first matching item.
7. Navigation keys (`↑`/`↓`) operate on all items, but matching items stand out visually.
8. `Enter` applies the filter and exits filter mode.
9. `Esc` clears the filter and returns to the normal view.
10. **Changing panels** (via `Tab` or mouse click) clears the filter and resets the filter state.

## Command Building

The `build_command()` method assembles the final command string:

1. Start with the binary name from the spec.
2. Append each subcommand in the current command path.
3. Append all active flags for the current command and its ancestors:
   - Boolean flags: `--flag-name`
   - Value flags: `--flag-name value` (or `--flag-name=value` for certain formats)
   - Count flags: repeated short flag (e.g., `-vvv` for count 3)
   - Flags with choices: `--flag-name selected-choice`
4. Append all non-empty argument values in positional order.
5. Include flags from parent commands (global flags) if they have been set.

### Flag Formatting Rules

- Long flags are preferred (`--verbose` over `-v`) except for count flags which use the short form repeated.
- Flags with both long and short forms use the long form in the output.
- Boolean flags that are `false` (off) are omitted.
- Count flags with count 0 are omitted.
- String flags with empty values are omitted.

## Scrolling

When a list is longer than the visible area, scrolling is handled automatically:

- The selected item is always kept visible within the viewport.
- `ensure_visible()` adjusts the scroll offset so the selected index is within the visible range.
- Scroll state is tracked per panel (`command_scroll`, `flag_scroll`, `arg_scroll`).
- Scroll offsets reset to 0 when navigating to a new command.

## Theming

TuiSage uses `ratatui-themes` for color theming. The theme provides a `ThemePalette` from which semantic UI colors are derived:

| UI Element | Palette Mapping |
|---|---|
| Command names | `palette.blue` |
| Flag names | `palette.yellow` |
| Argument names | `palette.green` |
| Values | `palette.cyan` |
| Required indicators | `palette.red` |
| Help text | `palette.overlay0` or dim |
| Active border | `palette.blue` |
| Inactive border | `palette.surface1` |
| Selection background | `palette.surface1` |
| Editing background | `palette.surface2` |
| Filter text | `palette.mauve` |
| Choice indicators | `palette.peach` |
| Default value text | `palette.overlay0` |
| Count indicators | `palette.peach` |

Themes can be cycled at runtime with `]` and `[` keys. The current theme name is displayed in the status bar.

## Terminal Lifecycle

1. **Startup**: Parse CLI args → load spec → enable mouse capture → initialize terminal → create `App` state → enter event loop.
2. **Event loop**: Draw frame → wait for event → handle key/mouse/resize → repeat. The application remains running indefinitely until the user quits.
3. **Execute**: User presses Enter on preview → execute the built command → show output/status → return to the TUI for building the next command.
4. **Quit**: User presses `q`/`Ctrl-C`/`Esc` at root → restore terminal → disable mouse capture → exit 0 (no output).
5. **Error**: Parsing or terminal errors → report error via `color-eyre` → exit non-zero.

The TUI is **long-running** by design, allowing repeated command building and execution within a single session. An optional print-only mode may be added to output commands to stdout for integration with shell tools.