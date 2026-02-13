# Requirements

TuiSage is a terminal UI for interactively building CLI commands from [usage](https://usage.jdx.dev/) specs.

## Purpose

CLI tools with many subcommands, flags, and arguments are difficult to use from memory. Usage specs (`.usage.kdl` files) formally describe a CLI tool's interface, but reading a spec file isn't a practical way to build commands. TuiSage bridges this gap by turning a usage spec into an interactive, explorable interface that produces ready-to-run command strings.

## Target Users

- Developers who work with complex CLI tools and want to discover or recall available options.
- Teams that define CLI interfaces with usage specs and want a quick way to construct commands.
- Users who prefer interactive exploration over reading `--help` output or documentation.

## Core Goals

1. **Parse usage specs** — Read `.usage.kdl` files (or stdin) and understand the full command tree: subcommands, flags, arguments, choices, defaults, and aliases.
2. **Interactive exploration** — Present the command hierarchy in a navigable TUI so users can browse subcommands, see available flags, and understand argument requirements.
3. **Command assembly** — As the user makes selections, continuously build and display the resulting command string in real time.
4. **Long-running interface** — Remain open after building a command, allowing users to modify it and build again, or execute commands directly within the TUI.

## Functional Requirements

### Input

- Accept a usage spec via `--spec-cmd`, which runs a command and parses its stdout as a usage spec (e.g., `--spec-cmd "mise tasks ls --usage"`).
- Accept a usage spec via `--spec-file`, which reads a `.usage.kdl` file from disk.
- Exactly one of `--spec-cmd` or `--spec-file` must be provided.
- Accept an optional `--cmd` flag to override the base command being built (e.g., `--cmd "mise run"`), replacing the spec's binary name.
- Support `--usage` to output TuiSage's own usage spec in `.usage.kdl` format (via `clap_usage`).

### Command Navigation

- Display the full tree of subcommands for the loaded spec as a flat indented list (all commands always visible with depth-based indentation).
- Allow navigating into nested subcommands (multi-level depth) via arrow keys (←/→ or h/l) and Enter.
- Allow navigating back up the command tree via Left arrow, Esc, or parent selection.
- The selected command in the always-visible tree, combined with the live command preview, provides constant visibility of the user's position in the command hierarchy.

### Flag Interaction

- Display all flags available for the currently selected command, including inherited global flags.
- Support boolean flags (toggle on/off).
- Support count flags (increment/decrement, e.g., `-vvv`).
- Support flags that take string values (free-text input).
- Support flags with predefined choices (cycle through options).
- Display default values clearly when a flag has one.
- Distinguish between global and command-specific flags.

### Argument Interaction

- Display positional arguments for the currently selected command.
- Support free-text input for arguments.
- Support arguments with predefined choices (cycle through options).
- Indicate which arguments are required vs. optional.

### Filtering

- Provide fuzzy filtering to highlight matching commands and flags by name.
- Activate filtering with a keyboard shortcut (`/`).
- **Keep all items visible** — non-matching items are subdued using color (dimmed/grayed) rather than hidden, so users can see the full context and all available options without layout shift.
- **Matching items stand out** — items that match the filter pattern are displayed in normal color with matching characters highlighted/emphasized.
- **Separate name/help matching** — the command/flag name and help text are treated as independent fields for matching. Each field is scored separately; highlighting only appears in the field(s) that independently match. This prevents confusing partial highlights where a few characters match in the name and the rest match in the help text.
- **Help text highlighting** — when the help text matches the filter, matching characters in the help text are highlighted using the same style as name highlights.
- **Filtered navigation** — when a filter is active, `↑`/`↓` skip non-matching items and move directly to the previous/next matching item, making it fast to cycle through matches.
- **Auto-select matching items** — if the currently selected item doesn't match the filter, automatically move the selection to the next matching item.
- **Filter mode visual cues** — when filter mode is activated, the panel title immediately shows the `/` prompt (e.g., `Commands (/)`) even before any text is typed, and the panel border changes to the active color to clearly indicate filter mode is in progress.
- **Clear filter when changing panels** — switching focus via Tab or mouse click clears the active filter to prevent confusion.
- Use scored matching (powered by `nucleo-matcher`) to rank results by relevance.

### Command Preview

- Show a live preview of the assembled command, updated on every interaction.
- Global flags toggled from any subcommand level are correctly included in the built command.
- On startup, the correct flags and arguments for the initially selected command are displayed immediately (no key press required).
- Allow the user to accept and output the command.

### Command Execution & Output

- Execute the built command directly from within the TUI using an embedded pseudo-terminal (PTY).
- Commands are executed with separate process arguments (not shell-stringified), using `portable-pty` for full TTY support (colors, ANSI sequences, interactive I/O).
- During execution, the UI switches to an execution view: the command is displayed at the top, terminal output fills the main area, and a status bar shows running/exited state.
- Keyboard input is forwarded to the running process (including Ctrl-C for SIGINT, arrow keys, etc.).
- When the process exits, the terminal output remains visible. The user presses Esc/Enter/q to close the execution view and return to the command builder.
- Remain open after execution to allow building and running additional commands.
- Provide a print-only mode: when the command preview is focused, a dedicated keybinding (`p`) outputs the command to stdout and exits, enabling piping or integration with external tools.
- Exit cleanly with no output when the user quits the application.
- Support `--usage` flag to output TuiSage's own usage spec and exit, enabling self-describing CLI integration.

## Non-Functional Requirements

### Input Methods

- **Keyboard-first**: All functionality must be accessible via keyboard with vim-style navigation.
- **Mouse-supported**: Click to select, scroll to navigate, and click-to-activate as a secondary input method.

### Performance

- The UI should feel instant — no perceptible lag on navigation or typing.
- Support usage specs of any practical size without degradation.

### Visual Design

- Use terminal colors effectively to distinguish commands, flags, arguments, and values.
- Support multiple color themes, switchable at runtime with `]` (next) and `[` (previous).
- Use clear, accessible symbols that render well across terminal emulators.
- Scroll long lists automatically to keep the selection visible.

### Compatibility

- Run on macOS, Linux, and Windows terminals.
- Work with common terminal emulators (iTerm2, Terminal.app, Alacritty, Windows Terminal, etc.).

## Future Considerations

- Copy command to clipboard directly from the TUI.
- Continuous integration pipeline for automated testing and quality checks.
- Support script files with embedded `USAGE` heredoc blocks via `--spec-file`.
- PTY resize support: dynamically resize the embedded terminal when the TUI window is resized during execution.
- Send stdin input to the running process via a dedicated input bar.
