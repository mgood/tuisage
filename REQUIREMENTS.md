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
4. **Command output** — When the user confirms, print the assembled command to stdout so it can be piped, copied, or executed.

## Functional Requirements

### Input

- Accept a `.usage.kdl` file path as a CLI argument.
- Accept a usage spec piped via stdin (explicit `-` argument or automatic detection).
- Support script files with embedded `USAGE` heredoc blocks (via usage-lib).

### Command Navigation

- Display the full tree of subcommands for the loaded spec.
- Allow navigating into nested subcommands (multi-level depth).
- Allow navigating back up the command tree.
- Show the current command path as a breadcrumb trail.

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

- Provide fuzzy filtering to narrow down commands and flags by name.
- Activate filtering with a keyboard shortcut (`/`).
- Show filtered count vs. total count.

### Command Preview

- Show a live preview of the assembled command, updated on every interaction.
- Allow the user to accept and output the command.

### Output

- Print the final command string to stdout on acceptance.
- Exit cleanly with no output on cancellation.
- Composable with shell tools (`pbcopy`, `eval`, pipes).

## Non-Functional Requirements

### Input Methods

- **Keyboard-first**: All functionality must be accessible via keyboard with vim-style navigation.
- **Mouse-supported**: Click to select, scroll to navigate, and click-to-activate as a secondary input method.

### Performance

- The UI should feel instant — no perceptible lag on navigation or typing.
- Support usage specs of any practical size without degradation.

### Visual Design

- Use terminal colors effectively to distinguish commands, flags, arguments, and values.
- Support multiple color themes with a theme picker.
- Use clear, accessible symbols that render well across terminal emulators.
- Scroll long lists automatically to keep the selection visible.

### Compatibility

- Run on macOS, Linux, and Windows terminals.
- Work with common terminal emulators (iTerm2, Terminal.app, Alacritty, Windows Terminal, etc.).

## Future Considerations

- Copy command to clipboard directly from the TUI.
- Execute the built command directly from the TUI (optional mode).
- TreeView display for the command hierarchy (using ratatui-interact `TreeView`).
- Fuzzy matching with scoring/ranking (fzf-style, using `nucleo-matcher`).
- Show aliases inline in the command list with dimmed styling.
- Split large modules (`app.rs`, `ui.rs`) into focused sub-modules.
- CI pipeline (GitHub Actions: test, clippy, snapshot checks).