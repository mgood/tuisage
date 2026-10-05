# TuiSage

A terminal UI for running [mise tasks](https://mise.jdx.dev/tasks/) or other commands supporting [`--usage`](https://usage.jdx.dev/) specs.

![Animation showing the interface](https://vhs.charm.sh/vhs-2rTonR9L8NblHYy3DQQi2Y.gif)

## Features

- **Execute commands** — Runs in an embedded terminal so you can see the output, then return to the UI.
- **Fuzzy filter** — Press `/` to activate search mode, or start typing in a "select" box. Uses [nucleo](https://crates.io/crates/nucleo-matcher) for fzf-style matching.
- **Dynamic completions** — Supports running a custom command to generate completion values. See the spec for the ["complete" statement](https://usage.jdx.dev/spec/reference/complete).
- **Mouse support** — Click to select, or mouse wheel to scroll up and down.
- **Themes** — Press "T" or click the name to open the theme selector. Uses [ratatui-themes](https://crates.io/crates/ratatui-themes).

## Installation

### Binaries

Binary downloads are available from the [releases](https://github.com/mgood/tuisage/releases).

They can also be fetched and installed with [mise](https://mise.jdx.dev):

```sh
mise u github:mgood/tuisage
```

### Building from source

Automatically build from crates.io:

```sh
cargo install tuisage
```

From a local clone:

```sh
cargo install --path .
```

Or build inside the clone:

```sh
cargo build --release
./target/release/tuisage
```

## Usage

### Mise Tasks

This is designed to work well with `mise`. Running `mise tasks ls --usage` prints the full usage spec the tasks, though you need to specify `--cmd "mise run"` as the command prefix to run the tasks:

```sh
tuisage --cmd "mise run" mise tasks ls --usage
```

For convenience, you can add it to your mise config file to run as `mise tui`:

```toml
[tasks.tui]
description = "TUI to run mise tasks"
run = 'tuisage --cmd "mise run" mise tasks ls --usage'
```

### Native `--usage` support

For tools supporting `--usage` you can run them like:

```sh
tuisage mytool --usage
```

To add `--usage` to an existing project, see the [list of integrations](https://usage.jdx.dev/spec/integrations) which supports common argument parsers for a variety of languages.

### From a file

You can also provide your own `--usage` spec from a file:

```sh
tuisage --spec-file path/to/cli.usage.kdl
```

### Other combinations

You can combine these as well:

```sh
tuisage --cmd "docker compose" --spec-file docker-compose.usage.kdl
```

## CLI Reference

| Flag | Description |
|---|---|
| `[SPEC_CMD]...` | Command to run to get the usage spec (e.g., `tuisage mycli --usage`) |
| `--spec-file <FILE>` | Read usage spec from a file |
| `--cmd <CMD>` | Base command to build (overrides the spec's binary name) |
| `--usage` | Generate usage spec for TuiSage itself |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

Provide either trailing arguments (spec command) or `--spec-file` (but not both).

## Keyboard Shortcuts

| Key | Action |
|---|---|
| `↑` / `↓` or `k` / `j` | Navigate within a panel or select box |
| `Tab` / `Shift-Tab` | Cycle focus between panels |
| `Enter` | Activate the selected input |
| `Space` | Toggle or increment a flag |
| `Backspace` | Remove/clear: decrement or clear a value |
| `Ctrl+U` | Reset flags and arguments for the current command |
| `/` | Enter search mode |
| `Esc` | Cancel filter / stop editing |
| `Ctrl+R` | Execute command |
| `]` / `[` | Cycle through themes |
| `T` | Open theme picker |
| `q` or `Ctrl+C` | Quit |

## Mouse

Left click to activate most elements. Mouse wheel scrolls selection up and down.

Click a shortcut in the bottom help row to perform the same action as its keyboard key. The theme name remains clickable to open the theme picker.

### Keymaps

TuiSage keeps its built-in shortcuts and accepts individual overrides in an optional TOML file at `$XDG_CONFIG_HOME/tuisage/keymap.toml`. If `XDG_CONFIG_HOME` is unset or empty, it checks `~/.config/tuisage/keymap.toml`. Use `--keymap PATH` to select a file explicitly. A missing default file is ignored. A missing or invalid explicit file is reported before the terminal UI starts.

The sample at [`examples/keymap.toml`](examples/keymap.toml) demonstrates optional plain Enter and keypad Enter submit bindings while keeping Ctrl+R. The built-in mappings remain unchanged when no keymap is configured. Supported named actions include `submit`, `cancel`, `next-field`, and `previous-field`. Map a key to `unbound` to disable that individual built-in binding.

Key names accept `ctrl`, `alt`, `shift`, and `cmd` or `super` modifiers. Use names such as `enter`, `keypad-enter`, `tab`, `backtab`, `escape`, or a single character.

Keypad Enter is detected only when the terminal reports it separately from Return. TuiSage requests the supported keyboard enhancement protocol on Unix terminals and restores the keyboard and mouse modes when it exits. Terminals that collapse these events cannot distinguish them for the application.

## Compatibility

This has been mostly tested in [ghostty](https://ghostty.org), though I have also tried it with the Mac built-in Terminal.app, and the Zed and VSCode embedded terminals. Some seem to trouble aligning the box-drawing characters, but are otherwise functional.

## Security Considerations

Only run this with *trusted* tools. Since it automatically runs custom commands provided by the usage spec, it could run an unintended command.

## Testing

```sh
# Run all tests
cargo test

# Review snapshot changes interactively
cargo insta review

# Run with acceptance of new snapshots
cargo insta test --accept
```

## Roadmap

Some features I would like to implement:

- **History / favorites** – revisit commands from the current session, save them for fast use in the future.
- **Saving preferences** – persist theme selection, or maybe other options.
- **Filename and other completions** – Recognize inputs for file paths to provide a file navigator, present a calendar picker for date fields, etc.

## Documentation

This README presents the main documentation intended for users. Other documents are primarily designed to help building and maintaining the app with AI agents, but may provide insight into the development.

| Document | Purpose |
|---|---|
| **AGENTS.md** | Development guidelines for AI agents and contributors |
| **REQUIREMENTS.md** | High-level goals, features, and user stories |
| **SPECIFICATION.md** | Detailed behavioral specification (UI, interactions, data flow) |
| **IMPLEMENTATION.md** | Architecture, code structure, and development state |

## Dependencies

| Crate | Purpose |
|---|---|
| [clap](https://crates.io/crates/clap) | CLI argument parsing (derive) |
| [clap_usage](https://crates.io/crates/clap_usage) | Generate usage specs from clap definitions |
| [usage-lib](https://crates.io/crates/usage-lib) | Parse usage specs (KDL format) |
| [ratatui](https://crates.io/crates/ratatui) | TUI framework |
| [crossterm](https://crates.io/crates/crossterm) | Terminal backend & events |
| [serde](https://crates.io/crates/serde) and [toml](https://crates.io/crates/toml) | Keymap configuration parsing |
| [unicode-width](https://crates.io/crates/unicode-width) | Help-row display widths and mouse hit areas |
| [ratatui-interact](https://crates.io/crates/ratatui-interact) | UI components (TreeView, input, focus management) |
| [ratatui-themes](https://crates.io/crates/ratatui-themes) | Color theming |
| [nucleo-matcher](https://crates.io/crates/nucleo-matcher) | Fuzzy matching |
| [portable-pty](https://crates.io/crates/portable-pty) | Cross-platform pseudo-terminal for command execution |
| [tui-term](https://crates.io/crates/tui-term) | Pseudo-terminal widget for embedded terminal output |
| [vt100](https://crates.io/crates/vt100) | Terminal emulation (VT100 parser) |
| [color-eyre](https://crates.io/crates/color-eyre) | Error reporting |
| [insta](https://crates.io/crates/insta) | Snapshot testing (dev) |

## License

MIT
