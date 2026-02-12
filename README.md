# TuiSage

A terminal UI for interactively building CLI commands from [usage](https://usage.jdx.dev/) specs.

Point TuiSage at a `.usage.kdl` file and it presents an interactive interface for browsing subcommands, toggling flags, filling in arguments, and producing a complete command string.

## Features

- **Browse subcommands** — explore the full command hierarchy as an expandable/collapsible tree
- **Toggle flags** — boolean flags, count flags (`-vvv`), and flags with values
- **Fill arguments** — positional args with free-text input or choice cycling
- **Live preview** — see the assembled command update in real time
- **Fuzzy filter** — press `/` to filter commands or flags by name
- **Mouse support** — click to select, scroll wheel to navigate, click-to-activate
- **Scrolling** — long lists scroll automatically to keep the selection visible
- **Default indicators** — flags with default values are clearly marked
- **Color themes** — multiple built-in themes, switchable at runtime with `[` and `]`
- **Stdin support** — pipe a usage spec directly into TuiSage

## Installation

```sh
cargo install --path .
```

Or build from source:

```sh
cargo build --release
```

## Usage

### From a file

```sh
tuisage path/to/cli.usage.kdl
```

### From stdin

```sh
cat cli.usage.kdl | tuisage
# or
tuisage - < cli.usage.kdl
```

### From a script with embedded USAGE

If your script contains a `USAGE` heredoc block, usage-lib will parse it:

```sh
tuisage ./my-script.sh
```

### Output

When you press **Enter** on the command preview, TuiSage prints the assembled command to stdout and exits. This makes it composable with other tools:

```sh
# Copy the built command to your clipboard (macOS)
tuisage spec.kdl | pbcopy

# Execute the built command directly
eval "$(tuisage spec.kdl)"
```

## Keyboard Shortcuts

| Key | Action |
|---|---|
| `↑` / `↓` or `k` / `j` | Navigate within a panel |
| `←` / `h` | Collapse tree node (or move to parent) |
| `→` / `l` | Expand tree node (or move to first child) |
| `Tab` / `Shift-Tab` | Cycle focus between panels |
| `Enter` | Toggle expand/collapse / toggle flag / edit arg / accept command |
| `Space` | Toggle boolean flag / increment count flag |
| `Backspace` | Decrement count flag (floor at 0) |
| `/` | Start fuzzy filter (works in Commands and Flags panels) |
| `Esc` | Cancel filter / stop editing / go back / quit |
| `]` / `[` | Next / previous color theme |
| `q` | Quit |
| `Ctrl-C` | Quit |

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
# Run all tests (111 tests: unit + rendering + snapshots)
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
| [usage-lib](https://crates.io/crates/usage-lib) | Parse usage specs (KDL format) |
| [ratatui](https://crates.io/crates/ratatui) | TUI framework |
| [crossterm](https://crates.io/crates/crossterm) | Terminal backend & events |
| [ratatui-interact](https://crates.io/crates/ratatui-interact) | UI components (TreeView, breadcrumb, input, focus management) |
| [ratatui-themes](https://crates.io/crates/ratatui-themes) | Color theming |
| [color-eyre](https://crates.io/crates/color-eyre) | Error reporting |
| [insta](https://crates.io/crates/insta) | Snapshot testing (dev) |

## License

MIT