# TuiSage

A terminal UI for interactively building CLI commands from [usage](https://usage.jdx.dev/) specs.

Point TuiSage at a usage spec — from a file or by running a command — and it presents an interactive interface for browsing subcommands, toggling flags, filling in arguments, and producing a complete command string.

## Features

- **Browse subcommands** — explore the full command hierarchy as an expandable/collapsible tree
- **Toggle flags** — boolean flags, count flags (`-vvv`), and flags with values
- **Fill arguments** — positional args with free-text input or choice cycling
- **Live preview** — see the assembled command update in real time
- **Execute commands** — run the built command directly within the TUI in an embedded terminal (PTY)
- **Fuzzy filter** — press `/` to filter commands or flags with scored, ranked matching (powered by `nucleo-matcher`)
- **Mouse support** — click to select, scroll wheel to navigate, click-to-activate
- **Scrolling** — long lists scroll automatically to keep the selection visible
- **Default indicators** — flags with default values are clearly marked
- **Color themes** — multiple built-in themes, switchable at runtime with `[` and `]`
- **Usage spec output** — generate TuiSage's own usage spec with `--usage`

## Installation

```sh
cargo install --path .
```

Or build from source:

```sh
cargo build --release
```

## Usage

### From a command (standard use case)

Run a command that outputs a usage spec, and optionally specify the base command to build:

```sh
# Build a "mise run" command using the spec from "mise tasks ls --usage"
tuisage --cmd "mise run" --spec-cmd "mise tasks ls --usage"

# Just explore a tool's usage spec
tuisage --spec-cmd "mytool --usage"
```

### From a file

```sh
tuisage --spec-file path/to/cli.usage.kdl
```

### With a base command override

The `--cmd` flag overrides the binary name from the spec, so the built command starts with whatever you provide:

```sh
tuisage --cmd "docker compose" --spec-file docker-compose.usage.kdl
```

### Generate TuiSage's own usage spec

```sh
tuisage --usage
```

### Command Execution

When you press **Enter** on the command preview, TuiSage executes the command in an embedded terminal directly within the TUI. The command is run with separate process arguments (not shell-stringified) for safety.

During execution:
- The command is displayed at the top of the screen
- Terminal output is shown in real time in a pseudo-terminal pane
- Keyboard input is forwarded to the running process (including Ctrl-C)
- After the process exits, press Esc/Enter/q to close and return to the command builder

This allows you to build, run, tweak, and re-run commands iteratively without leaving the TUI.

## CLI Reference

| Flag | Description |
|---|---|
| `--spec-cmd <CMD>` | Run a command to get the usage spec (e.g., `"mise tasks ls --usage"`) |
| `--spec-file <FILE>` | Read usage spec from a file |
| `--cmd <CMD>` | Base command to build (overrides the spec's binary name) |
| `--usage` | Generate usage spec for TuiSage itself |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

One of `--spec-cmd` or `--spec-file` is required (but not both).

## Keyboard Shortcuts

| Key | Action |
|---|---|
| `↑` / `↓` or `k` / `j` | Navigate within a panel |
| `←` / `h` | Collapse tree node (or move to parent) |
| `→` / `l` | Expand tree node (or move to first child) |
| `Tab` / `Shift-Tab` | Cycle focus between panels |
| `Enter` | Toggle expand/collapse / toggle flag / edit arg / execute command |
| `Space` | Toggle boolean flag / increment count flag |
| `Backspace` | Decrement count flag (floor at 0) |
| `/` | Start fuzzy filter (works in Commands and Flags panels) |
| `Esc` | Cancel filter / stop editing / go back / quit |
| `]` / `[` | Next / previous color theme |
| `q` | Quit |
| `Ctrl-C` | Quit |

### During Command Execution

| Key | Action |
|---|---|
| Any key | Forwarded to the running process |
| `Ctrl-C` | Send SIGINT to the running process |
| `Ctrl-D` | Send EOF to the running process |

### After Command Exits

| Key | Action |
|---|---|
| `Esc` / `Enter` / `q` | Close execution view, return to builder |

## Mouse

| Action | Effect |
|---|---|
| Left click | Focus panel and select item |
| Click on selected item | Activate (toggle expand/collapse / toggle / edit) |
| Right click (commands) | Expand tree node |
| Scroll wheel | Move selection up/down |

## Example Spec

Here's a minimal `.usage.kdl` spec that TuiSage can read:

```kdl
bin "mytool"
about "A sample tool"

flag "-v --verbose" help="Verbose output"
flag "-f --force" help="Force operation"

cmd "init" help="Initialize a project" {
    arg "<name>" help="Project name"
    flag "-t --template <tpl>" help="Template" {
        arg "<tpl>" {
            choices "basic" "full" "minimal"
        }
    }
}

cmd "deploy" help="Deploy the app" {
    arg "<env>" help="Target environment" {
        choices "dev" "staging" "prod"
    }
    flag "--tag <tag>" help="Image tag"
    flag "--dry-run" help="Preview only"
}
```

See `fixtures/sample.usage.kdl` for a more comprehensive example.

## Testing

```sh
# Run all tests (127 tests: unit + rendering + snapshots)
cargo test

# Review snapshot changes interactively
cargo insta review

# Run with acceptance of new snapshots
cargo insta test --accept
```

## Documentation

| Document | Purpose |
|---|---|
| **README.md** | User guide (this file) |
| **REQUIREMENTS.md** | High-level goals, features, and user stories |
| **SPECIFICATION.md** | Detailed behavioral specification (UI, interactions, data flow) |
| **IMPLEMENTATION.md** | Architecture, code structure, and development state |
| **AGENTS.md** | Development guidelines for contributors and AI agents |

## Dependencies

| Crate | Purpose |
|---|---|
| [clap](https://crates.io/crates/clap) | CLI argument parsing (derive) |
| [clap_usage](https://crates.io/crates/clap_usage) | Generate usage specs from clap definitions |
| [usage-lib](https://crates.io/crates/usage-lib) | Parse usage specs (KDL format) |
| [ratatui](https://crates.io/crates/ratatui) | TUI framework |
| [crossterm](https://crates.io/crates/crossterm) | Terminal backend & events |
| [ratatui-interact](https://crates.io/crates/ratatui-interact) | UI components (TreeView, breadcrumb, input, focus management) |
| [ratatui-themes](https://crates.io/crates/ratatui-themes) | Color theming |
| [nucleo-matcher](https://crates.io/crates/nucleo-matcher) | Fuzzy matching |
| [portable-pty](https://crates.io/crates/portable-pty) | Cross-platform pseudo-terminal for command execution |
| [tui-term](https://crates.io/crates/tui-term) | Pseudo-terminal widget for embedded terminal output |
| [vt100](https://crates.io/crates/vt100) | Terminal emulation (VT100 parser) |
| [color-eyre](https://crates.io/crates/color-eyre) | Error reporting |
| [insta](https://crates.io/crates/insta) | Snapshot testing (dev) |

## License

MIT