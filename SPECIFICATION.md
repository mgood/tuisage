# Specification

This document defines the detailed behavior of TuiSage — how features work, how the UI is structured, and how user interactions are handled. It bridges the high-level goals in REQUIREMENTS.md and the implementation details in IMPLEMENTATION.md.

## Library Preferences

These are the preferred libraries for implementing TuiSage features, as specified during initial development:

| Feature | Preferred Library | Notes |
|---|---|---|
| TUI framework | `ratatui` | Terminal UI rendering and layout |
| Terminal backend | `crossterm` | Cross-platform terminal control and events |
| UI components | `ratatui-interact` | Input, FocusManager, ListPickerState, TreeView |
| Color theming | `ratatui-themes` | Consistent theme palettes |
| Usage spec parsing | `usage-lib` | Parse `.usage.kdl` files (no default features) |
| Fuzzy matching | `nucleo-matcher` | Pattern-based fzf-style scoring with multi-word and special character support |
| Error reporting | `color-eyre` | Pretty error messages with context |
| Snapshot testing | `insta` | Terminal output comparison (dev dependency) |
| Embedded terminal | `tui-term` | PseudoTerminal widget for rendering PTY output in the TUI |
| Terminal emulation | `vt100` | VT100 parser for processing terminal control sequences |
| PTY management | `portable-pty` | Cross-platform pseudo-terminal creation and process spawning |

## Input Handling

### CLI Arguments

TuiSage uses `clap` (derive mode) for argument parsing and `clap_usage` for generating its own usage spec. The CLI accepts the following flags:

| Flag | Description |
|---|---|
| `--spec-cmd <CMD>` | Run a shell command and parse its stdout as a usage spec (e.g., `--spec-cmd "mise tasks ls --usage"`) |
| `--spec-file <FILE>` | Read a usage spec from a file path (`.usage.kdl` or a script with embedded `USAGE` block) |
| `--cmd <CMD>` | Override the base command being built (e.g., `--cmd "mise run"`), replacing the spec's binary name |
| `--usage` | Output TuiSage's own usage spec (in `.usage.kdl` format via `clap_usage`) and exit |
| `-h, --help` | Print help (provided by clap) |
| `-V, --version` | Print version (provided by clap) |

**Rules:**
- Exactly one of `--spec-cmd` or `--spec-file` must be provided (mutually exclusive, both required without the other).
- `--cmd` is optional; when omitted the spec's `bin` field is used as the base command.
- `--usage` short-circuits before any spec loading and prints the usage spec to stdout.

Parsing errors and spec command failures produce descriptive error messages via `color-eyre` and exit non-zero. When `--spec-cmd` is used, the command is executed via `sh -c` (or `cmd /C` on Windows) and its stdout is parsed as a usage spec; a non-zero exit status from the command is reported as an error.

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
│ Command Preview                                  │
├─────────────────┬────────────────────────────────┤
│                 │                                │
│   Commands      │     Flags                      │
│   Panel         │     Panel                      │
│   (40%)         │     (60% top)                  │
│                 │                                │
│                 ├────────────────────────────────┤
│                 │                                │
│                 │     Arguments                  │
│                 │     Panel                      │
│                 │     (60% bottom)               │
│                 │                                │
├─────────────────┴────────────────────────────────┤
│ 💡 Help text for current item                    │
│ Keybinding hints                      [Theme]    │
└──────────────────────────────────────────────────┘
```

The layout uses a 2-column design: Commands on the left (40% width), and Flags + Arguments stacked vertically on the right (60% width, with Flags taking 60% and Args 40% of that column). The Command Preview (3 rows) is fixed at the top, and the help bar (2 rows) is fixed at the bottom. When there are no subcommands, the Commands panel is hidden and Flags + Arguments fill the full width.

The Command Preview is at the top so it remains in the same position when switching to execution mode (where the command is also displayed at the top), providing visual stability.

The selected command in the always-visible tree, combined with the live command preview at the bottom, provides constant visibility of the user's position in the command hierarchy.

### Commands Panel

- Displays the full command hierarchy as a flat list with depth-based indentation (2 spaces per level).
- All commands and subcommands are always visible — the entire tree is shown at all times.
- Each item shows the command name and its `help` text (right-aligned, if available).
- Aliases are shown alongside the command name (e.g., `remove (rm)`).
- Left arrow (←/h) moves selection to the parent command; Right arrow (→/l) moves to the first child.
- Enter navigates into the selected command (moves to first child), same as Right arrow.
- The selected command determines which flags and arguments are displayed in the other panels.
- On startup, the tree selection and command state are synchronized so the correct flags and arguments are displayed immediately (no key press required).
- When filtering is active, all commands remain visible:
  - **Non-matching commands** are displayed in a dimmed/subdued color
  - **Matching commands** are displayed normally, with matching characters highlighted independently in the name and help text (bold+underlined on unselected items, inverted colors on the selected item)
  - **Matching includes the full ancestor path** so that queries like "cfgset" match "config set"
  - **Selected command** is always shown with bold styling and cursor indicator
  - **Help text highlighting** — if the help text matches the filter, matching characters are highlighted with the same style as name matches
- The panel title shows just "Commands" (no counts), or "Commands 🔍" when filter mode is first activated, or "Commands 🔍 query" as the user types. The panel border changes to the active color during filter mode.

### Flags Panel

- Lists all flags available for the currently selected command, including global (inherited) flags.
- Each flag shows:
  - A checkbox indicator: `✓` (enabled) or `○` (disabled) for boolean flags
  - The flag name (long form preferred, short form shown alongside)
  - Current value for value-bearing flags
  - `(default: X)` indicator when a default value exists
  - `[G]` indicator for inherited global flags
  - Count value for count flags (e.g., `[3]`)
- Flags with choices show the current selection via an inline select box (see [Inline Choice Select Box](#inline-choice-select-box)).
- Global flags toggled from any subcommand level are correctly included in the built command.
- When filtering is active, name and help text are matched independently — highlights only appear in the field that matched.
- The panel title shows just "Flags" (no counts), or "Flags 🔍" when filter mode is first activated, or "Flags 🔍 query" as the user types. The panel border changes to the active color during filter mode.

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

- Shows the fully assembled command string as it would be output, fixed at the **top** of the screen.
- Updates in real time as the user toggles flags, fills values, and navigates.
- Remains in the same position when switching to execution mode, providing visual stability.
- When focused: displays a `▶` prefix to signal that Enter will execute the command.
- When unfocused: displays a `$` prompt prefix.
- The command is colorized: binary name, subcommands, flags, and values each get distinct colors.
- When focused, pressing `p` prints the command to stdout and exits (print-only mode for piping).

### Help / Status Bar

- Shows contextual help text for the currently hovered item.
- Displays available keyboard shortcuts.
- Shows the current theme name.

### Execution View (during command execution)

When a command is executed, the UI switches to a dedicated execution layout:

```
┌──────────────────────────────────────────────────┐
│  Command: $ mycli deploy --tag v1.0 prod         │
├──────────────────────────────────────────────────┤
│                                                  │
│  Pseudo-terminal output                          │
│  (rendered via tui-term PseudoTerminal widget)   │
│                                                  │
│                                                  │
├──────────────────────────────────────────────────┤
│  Running… / Exited (0) — press Esc to close      │
└──────────────────────────────────────────────────┘
```

- **Command pane** (top, 3 rows): Shows the executed command string with a `$` prefix, styled with bold text and the active border color.
- **Terminal pane** (middle, fills remaining space): Renders the PTY output via `tui-term::PseudoTerminal`. While running, the border is active-colored and titled "Output (running…)". After exit, the border is inactive-colored and titled "Output (finished)".
- **Status bar** (bottom, 1 row): Shows "Running… (input is forwarded to the process)" while active, or "Exited (CODE) — press Esc/Enter/q to close" after the process finishes.

## Focus System

The UI has four focusable panels, cycled with Tab/Shift-Tab:

1. **Commands** — command tree list
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

The currently selected command in the flat tree list determines which flags and arguments are displayed. The list maintains its own state for:

- Which node is currently selected (cursor position)
- Scroll offset for long lists

The selected command's full path (e.g., `["config", "set"]`) is computed from the flat list structure to look up flag and argument values. On startup, the tree selection is synchronized with the command path so that the correct flags and arguments are displayed immediately without requiring any user input.

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
| `Enter` | Commands panel | Navigate into the selected command (move to first child, same as →/l) |
| `Enter` | Flags panel (boolean) | Toggle the flag |
| `Enter` | Flags panel (value) | Start editing the flag value |
| `Enter` | Flags panel (choices) | Cycle to the next choice |
| `Enter` | Args panel | Start editing the argument / cycle choice |
| `Enter` | Preview panel | Execute the built command in an embedded PTY |
| `Space` | Flags panel (boolean) | Toggle the flag |
| `Space` | Flags panel (count) | Increment the count |
| `Backspace` | Flags panel (count) | Decrement the count (floor at 0) |
| `p` | Preview panel | Print the command to stdout and exit (print-only mode) |
| `/` | Commands, Flags, or Args panel | Activate fuzzy filter mode (no effect in Preview panel) |
| `Ctrl+R` | Any panel | Execute the built command in an embedded PTY |

### Editing Mode Keys

When editing a text value (flag value or argument):

| Key | Action |
|---|---|
| Any character | Append to the input |
| `Backspace` | Delete the last character |
| `Enter` | Confirm the value and exit editing mode |
| `Esc` | Cancel editing and exit editing mode |

### Filter Mode Keys

Filtering has two phases: **typing mode** (actively entering a query) and **applied filter** (query locked in, keys navigate instead of typing).

When in **typing mode** (after pressing `/`):

| Key | Action |
|---|---|
| Any character | Append to the filter query; auto-select next matching item if current item doesn't match |
| `Backspace` | Delete the last character from the query; auto-select next matching item if current item doesn't match |
| `Esc` | Clear the filter and exit typing mode |
| `↑` / `↓` | Navigate to the previous/next **matching** item (skips non-matching items) |
| `Enter` | Apply the filter and exit typing mode (filter remains active; navigation keys like `j`/`k` move between matches instead of appending to the query) |
| `Tab` | Switch focus to the other panel and clear the filter |

When a filter is **applied** (after pressing `Enter`):

| Key | Action |
|---|---|
| `↑` / `↓` / `j` / `k` | Navigate to the previous/next **matching** item (skips non-matching items) |
| `Esc` | Clear the applied filter |
| `Tab` / `Shift-Tab` | Switch focus to another panel and clear the filter |
| `/` | Start a new filter (clears the previous one and enters typing mode) |

### Theme Keys

| Key | Action |
|---|---|
| `]` | Switch to the next theme |
| `[` | Switch to the previous theme |
| `T` | Switch to the next theme (alias) |

### Execution Mode Keys

When a command is running in the embedded terminal:

| Key | Action |
|---|---|
| Any character | Forwarded to the running process as stdin |
| `Enter` | Forwarded as carriage return (`\r`) |
| `Backspace` | Forwarded as DEL (`\x7f`) |
| `Tab` | Forwarded as tab (`\t`) |
| `Esc` | Forwarded as escape (`\x1b`) while running |
| `↑` / `↓` / `←` / `→` | Forwarded as ANSI arrow key sequences |
| `Home` / `End` / `Delete` | Forwarded as ANSI escape sequences |
| `Ctrl-C` | Sends SIGINT (`\x03`) to the running process |
| `Ctrl-D` | Sends EOF (`\x04`) to the running process |

When the command has exited:

| Key | Action |
|---|---|
| `Esc` | Close the execution view and return to the command builder |
| `Enter` | Close the execution view and return to the command builder |
| `q` | Close the execution view and return to the command builder |

## Mouse Interactions

Mouse support is enabled via crossterm's `EnableMouseCapture`.

| Action | Effect |
|---|---|
| Left click on a panel | Focus that panel and select the clicked item |
| Left click on an already-selected item | Activate it (same as Enter for the focused panel) |
| Left click on already-focused Preview | Print the command to stdout and exit (Accept) |
| Scroll wheel up | Move selection up in the panel under the cursor |
| Scroll wheel down | Move selection down in the panel under the cursor |

### Click Region Tracking

The UI registers click regions during rendering so that mouse coordinates can be mapped back to the correct panel and item index. Regions are stored as `(Rect, Focus)` tuples and cleared/rebuilt on each render.

### Edit Finishing on Mouse

If the user is currently editing a value and clicks on a different item or panel, the edit is finished (committed) before processing the new click. This prevents the edit input text from "bleeding" into a different field.

## Fuzzy Filtering

Filtering uses scored fuzzy-matching (powered by `nucleo-matcher::Pattern`):

1. The user presses `/` to activate filter mode in the Commands, Flags, or Args panel. Pressing `/` in the Preview panel has no effect.
2. **Filter mode visual cues**:
   - The panel title immediately shows the 🔍 emoji (e.g., `Commands 🔍` when the query is empty, `Commands 🔍 query` as the user types). The 🔍 remains visible as long as a filter is applied (including after Enter exits typing mode). No counts are shown.
   - The panel border color changes to the active border color during typing mode to clearly indicate filter input is active.
3. As the user types, items are scored against the filter pattern using `Pattern::parse()`:
   - **Pattern matching** supports multi-word patterns (whitespace-separated) and fzf-style special characters (^, $, !, ')
   - **Matching items** (score > 0) are displayed in normal color with matching characters highlighted (bold+underlined on unselected items, inverted colors on the selected item)
   - **Non-matching items** (score = 0) are displayed in a dimmed/subdued color without shifting the layout
   - Match indices are sorted and deduplicated for accurate character-level highlighting
4. **Separate name/help matching**: The command/flag name and the help text are treated as independent fields for matching. Each field is scored separately against the filter pattern. An item matches if *either* field scores > 0, but **highlighting is only shown in the field(s) that independently match**. This prevents confusing partial highlights in the name when the item only matched via its help text (or vice versa).
5. **Help text highlighting**: When the help text matches the filter pattern, matching characters in the help text are highlighted using the same bold+underlined style (or inverted style on selected items) used for name highlighting.
6. **Full-path matching**: In the Commands panel, subcommands are matched against their full ancestor path (e.g. "config set") so that queries like "cfgset" match subcommands via their parent chain.
7. All commands remain visible for context — the flat list structure is preserved.
8. **Auto-selection**: If the currently selected item doesn't match the filter, the selection automatically moves to the first matching item.
9. **Filtered navigation**: When a filter is applied (during typing mode or after Enter), `↑`/`↓` skip non-matching items and move directly to the previous/next matching item. This makes it fast to cycle through matches without manually scrolling past dimmed non-matches.
10. `Enter` exits typing mode but keeps the filter applied. Highlighting, dimming, filtered navigation, and the 🔍 indicator all remain active. Navigation keys (`j`/`k`, `↑`/`↓`) move between matches instead of appending to the filter query.
11. `Esc` clears the filter (including any applied filter from Enter) and returns to the normal view.
12. **Changing panels** (via `Tab`, `Shift-Tab`, or mouse click) clears the filter and resets the filter state.

## Inline Choice Select Box

When a flag or argument has predefined choices (e.g., `--template` with choices `basic`, `full`, `minimal`), activating it (via Enter) opens an **inline select box** instead of cycling through choices or opening a text editor.

### Behavior

1. **Activation**: Pressing Enter on a flag or argument that has choices opens the select box. The select box appears as an overlay rendered inline, directly below the item's position in the list.
2. **Appearance**: The select box shows all available choices as a vertical list with a border, styled using the panel's active border color. The currently selected choice is highlighted. Choices that don't match the filter are hidden (not dimmed).
3. **Fuzzy filtering**: As the user types, the choices are filtered using the same `nucleo-matcher` fuzzy matching algorithm. Non-matching choices are **hidden** (removed from the visible list), unlike the main panel filter which dims non-matches. The select box title shows the typed filter text.
4. **Navigation**: `↑`/`↓`/`j`/`k` navigate between visible (matching) choices. Typing narrows the list.
5. **Selection**: `Enter` confirms the highlighted choice, closes the select box, and sets the flag/argument value.
6. **Cancellation**: `Esc` closes the select box without changing the value.
7. **Auto-select**: If only one choice matches the filter, it is auto-highlighted. If the filter narrows to zero matches, the select box shows an empty state.
8. **Sizing**: The select box is as wide as needed to fit the longest choice, plus the filter text, and as tall as the number of visible choices (up to a maximum of 10 rows). It is positioned to stay within the terminal bounds.

### Rendering

The select box is rendered as an overlay on top of the normal panel content, anchored below the activated item. It uses:
- Active border color for the border
- Selection background color for the highlighted choice
- Normal text for other choices
- The title shows the filter query (if any)

## Command Building

The `build_command()` method assembles the final command string (for display):

1. Start with the binary name from the spec.
2. Gather global flag values from **all** command levels (root and every subcommand in the current path). When a global flag is set at multiple levels, the deepest level's value is used. Global flags are emitted immediately after the binary name, before any subcommand names.
3. Append each subcommand in the current command path.
4. Append all active non-global flags for each subcommand level:
   - Boolean flags: `--flag-name`
   - Value flags: `--flag-name value` (or `--flag-name=value` for certain formats)
   - Count flags: repeated short flag (e.g., `-vvv` for count 3)
   - Flags with choices: `--flag-name selected-choice`
5. Append all non-empty argument values in positional order.

### Command Parts for Execution

The `build_command_parts()` method produces a `Vec<String>` of separate arguments for process execution:

- The binary name is split on whitespace (e.g., `"mise run"` becomes `["mise", "run"]`).
- Flag names and their values are separate elements (e.g., `["--tag", "v1.0"]` not `["--tag v1.0"]`).
- Argument values are NOT quoted — each is a separate process argument, which avoids shell injection issues.
- Count flags use the same format as display (e.g., `"-vvv"` as a single element).

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

TuiSage uses `ratatui-themes` for color theming. The theme provides a `ThemePalette` from which semantic UI colors are derived via `UiColors::from_palette()`:

| UI Element | Palette Mapping |
|---|---|
| Command names | `palette.info` |
| Flag names | `palette.warning` |
| Argument names | `palette.success` |
| Values | `palette.accent` |
| Required indicators | `palette.error` |
| Help text | `palette.muted` |
| Active border | `palette.accent` |
| Inactive border | `palette.muted` |
| Selection background | `palette.selection` |
| Editing background | Derived from `palette.selection` (warm-tinted) |
| Choice indicators | `palette.info` |
| Default value text | `palette.muted` |
| Count indicators | `palette.secondary` |
| Bar background | Derived from `palette.bg` (slightly lighter) |

Themes can be cycled at runtime with `]`/`[` keys (or `T` for next). The current theme name is displayed in the status bar.

## Terminal Lifecycle

1. **Startup**: Parse CLI args (clap) → handle `--usage` if present → load spec (from `--spec-cmd` or `--spec-file`) → apply `--cmd` override if present → enable mouse capture → initialize terminal → create `App` state → enter event loop.
2. **Event loop (builder mode)**: Draw frame → wait for event (blocking) → handle key/mouse/resize → repeat. The application remains running indefinitely until the user quits.
3. **Execute**: User presses Enter on preview → spawn the command in a PTY via `portable-pty` → switch to execution mode → display embedded terminal output via `tui-term`.
4. **Event loop (execution mode)**: Draw frame → poll for events (16ms interval for live terminal refresh) → forward keyboard input to PTY → repeat until user closes the execution view.
5. **Process exit**: Background thread detects child process exit → sets `exited` flag and records exit status → UI updates to show "Exited" status → user presses Esc/Enter/q to close.
6. **Return to builder**: Execution state is dropped (PTY writer and master cleaned up) → app mode switches back to `Builder` → normal event loop resumes.
7. **Quit**: User presses `q`/`Ctrl-C`/`Esc` at root → restore terminal → disable mouse capture → exit 0 (no output).
8. **Error**: Parsing or terminal errors → report error via `color-eyre` → exit non-zero.

The TUI is **long-running** by design, allowing repeated command building and execution within a single session. An optional print-only mode may be added to output commands to stdout for integration with shell tools.

## Command Execution Architecture

Command execution uses a multi-threaded PTY-based approach:

1. **PTY creation**: `portable-pty::NativePtySystem` creates a master/slave PTY pair sized to fit the terminal area (minus UI chrome).
2. **Process spawning**: The command is spawned on the slave side using `CommandBuilder` with separate arguments (not shell-stringified). The slave is dropped immediately after spawning.
3. **Output reading**: A background thread reads from the PTY master's reader in 8KB chunks and feeds the data into a `vt100::Parser`, which maintains the terminal screen state.
4. **Input forwarding**: Keyboard events in execution mode are converted to byte sequences and written to the PTY master's writer.
5. **Exit detection**: A background thread calls `child.wait()` and sets an `AtomicBool` flag plus the exit status string when the process finishes.
6. **Cleanup**: After exit, a background thread drops the PTY writer (allowing the reader thread to see EOF) and drops the master to release system resources.
7. **Rendering**: The `tui-term::PseudoTerminal` widget renders the `vt100::Parser`'s screen into the ratatui frame each tick.

### State Model

```
AppMode::Builder  ──(Enter on Preview)──►  AppMode::Executing
                                                │
                                           [process runs]
                                                │
                                           [process exits]
                                                │
                                     (Esc/Enter/q) ──►  AppMode::Builder
```

The `ExecutionState` struct holds:
- `command_display: String` — the command string shown at the top
- `parser: Arc<RwLock<vt100::Parser>>` — shared terminal state
- `pty_writer: Arc<Mutex<Option<Box<dyn Write + Send>>>>` — input channel to the process
- `exited: Arc<AtomicBool>` — whether the child has finished
- `exit_status: Arc<Mutex<Option<String>>>` — the exit code/signal description