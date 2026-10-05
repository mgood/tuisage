# TuiSage

A terminal UI for running [mise tasks](https://mise.jdx.dev/tasks/) or other commands supporting [`--usage`](https://usage.jdx.dev/) specs.

![Animation showing the interface](https://vhs.charm.sh/vhs-2rTonR9L8NblHYy3DQQi2Y.gif)

## Features

- **Execute commands** — Runs in an embedded terminal so you can see the output, then return to the UI.
- **Fuzzy filter** — Press `/` to activate search mode, or start typing in a "select" box. Uses [nucleo](https://crates.io/crates/nucleo-matcher) for fzf-style matching.
- **Dynamic completions** — Supports running a custom command to generate completion values. See the spec for the ["complete" statement](https://usage.jdx.dev/spec/reference/complete).
- **Presage companions:** Optionally add named choices, providers, defaults, validation and presentation settings without changing the command's Usage grammar. See [PRESAGE.md](PRESAGE.md).
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

### Initial field values

Use `--defaults JSON` to initialize fields from an object, or `--defaults @PATH` to read that object from a file. Keys may use unqualified names when they identify one field, or qualified identifiers such as `commands/run/args/name`. Each entry has a `value` and an optional `locked` boolean. Locked values cannot be changed through editing, mouse selection, completion, or reset. Empty strings are retained as explicit values.

### Presage companion documents

Presage is optional metadata alongside Usage. It can supply named choice lists, executable providers, defaults, locks, validation, composition mode and theme settings. Documents use KDL and are named `<command>.tuisage.kdl`; selector documents use `<command>.<selector>.tuisage.kdl`. TuiSage selects one document by the documented cascade and never merges documents or searches the current directory. Usage remains the command grammar, and Usage-only commands keep their existing behavior.

Providers and validators run as separate executables with literal arguments and structured JSON context. Review companion documents and executable permissions before installing them. See [PRESAGE.md](PRESAGE.md) for discovery, selectors, document syntax and provider responses.

On Windows, automatic discovery uses `%LOCALAPPDATA%\TuiSage\commands` for user documents and `%PROGRAMDATA%\TuiSage\commands` for shared documents.

To query the selected document and available selectors without opening the interface, run `tuisage --presage -- mytool --usage`.

## CLI Reference

| Flag | Description |
|---|---|
| `[SPEC_CMD]...` | Command to run to get the usage spec (e.g., `tuisage mycli --usage`) |
| `--spec-file <FILE>` | Read usage spec from a file |
| `--cmd <CMD>` | Base command to build (overrides the spec's binary name) |
| `--usage` | Generate usage spec for TuiSage itself |
| `--presage [PATH]` | Query Presage, select one document file, or prepend a directory to discovery |
| `--selector <ID>` | Select the command's named companion document |
| `--execute` | Override a companion's default composition mode |
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
| **PRESAGE.md** | Optional KDL companion documents and provider contract |

## Dependencies

| Crate | Purpose |
|---|---|
| [clap](https://crates.io/crates/clap) | CLI argument parsing (derive) |
| [clap_usage](https://crates.io/crates/clap_usage) | Generate usage specs from clap definitions |
| [usage-lib](https://crates.io/crates/usage-lib) | Parse usage specs (KDL format) |
| [ratatui](https://crates.io/crates/ratatui) | TUI framework |
| [crossterm](https://crates.io/crates/crossterm) | Terminal backend & events |
| [ratatui-interact](https://crates.io/crates/ratatui-interact) | UI components (TreeView, input, focus management) |
| [ratatui-themes](https://crates.io/crates/ratatui-themes) | Color theming |
| [nucleo-matcher](https://crates.io/crates/nucleo-matcher) | Fuzzy matching |
| [portable-pty](https://crates.io/crates/portable-pty) | Cross-platform pseudo-terminal for command execution |
| [tui-term](https://crates.io/crates/tui-term) | Pseudo-terminal widget for embedded terminal output |
| [vt100](https://crates.io/crates/vt100) | Terminal emulation (VT100 parser) |
| [kdl](https://crates.io/crates/kdl) | Presage companion document parser |
| [color-eyre](https://crates.io/crates/color-eyre) | Error reporting |
| [insta](https://crates.io/crates/insta) | Snapshot testing (dev) |
| [tempfile](https://crates.io/crates/tempfile) | Temporary documents in tests (dev) |

## Native command composition

The `--compose` option returns one JSON object with the executable and ordered argv. It does not run the command or open the execution view. The TUI uses the controlling terminal, so redirected stdout contains only JSON. Cancellation returns no output with status 130. Explicit empty arguments are retained. Terminal modes are restored on completion or error. Run `python3 tests/terminal_composition.py` after building to check the PTY flow.

## Repeated options and arguments

Usage specs can declare repeatable positional arguments, repeated flag occurrences, and multiple values on one flag occurrence. In the TUI, `Ctrl+N` and `Ctrl+D` add or remove positional rows or values within a flag occurrence. For a flag with repeated occurrences, `Ctrl+Alt+N` and `Ctrl+Alt+D` add or remove an occurrence; `Alt+Left/Right` selects an occurrence and `Ctrl+Left/Right` selects one of its values. Blank rows are omitted until edited, while an explicitly entered empty string is passed as an empty argument. Submission checks the occurrence count and the value count separately.

Typed defaults use arrays for repeated fields. A repeated positional argument or repeated flag with one value per occurrence takes an array of strings. A flag with multiple values per occurrence takes an array of strings; when both the flag and its argument repeat, use an array of arrays so the occurrence groups remain distinct. For example: `{"group":{"value":[["one","two"],["three"]]}}`.

## License

MIT


### Submission validation

Every execute/compose submission checks required arguments/options, required subcommands, declared choices, and repeated-value minimum/maximum limits. Failure leaves the form open, displays the field identifier and correction in the status row, and neither executes nor emits a command. Explicit empty supplied strings are distinguished from omitted fields.

`--validate PATH` additionally invokes a provider executable directly, sending the version-1 JSON form context on stdin. The request includes executable, argv, command path, optional field, and a map of canonical field identifiers to values. It must return `{ "version": 1, "errors": {} }` for success, or errors keyed by field identifier. Nonzero exit, malformed response, unsupported version, or a five-second timeout blocks submission. The operational command is never invoked for validation. Usage 2.16.1 does not expose a general conditional-rule API; dependent command-specific constraints belong in this provider.

### Context-aware completion

Legacy `complete ... run="..."` providers retain line-based output. They additionally receive `TUISAGE_CONTEXT_VERSION=1` and `TUISAGE_CONTEXT`, a JSON context containing canonical field values, command path, current argv and requested field. Top-level completion declarations are a fallback when the command has no matching declaration.

A structured provider uses `complete "field" type="tuisage-json-v1:/path/to/provider"`. It receives the same JSON on stdin and returns `{ "version": 1, "choices": [{ "value": "...", "description": "optional" }] }`. This format can return explicit empty values. Providers run off the input thread. Each result is checked against its request generation and current form context; old responses are discarded. Changed context refreshes an open completion, while errors or empty responses retain manual input. No application discovery logic is built into TuiSage.

Completion results update suggestions in place, preserving manual text and cursor position while a provider is running.
### Named startup and automatic themes

Use `tuisage --theme catppuccin-latte mytool --usage` to select a named theme. Names and aliases use ratatui-themes' existing parser, including hyphen and underscore spelling. Omitted options retain Dracula.

Use `--theme auto --theme-light catppuccin-latte --theme-dark dracula` for automatic appearance. Both names must validate before terminal startup. macOS uses system AppleInterfaceStyle; Linux uses the desktop portal when available. Elsewhere, or when Linux supplies no preference, COLORFGBG is a terminal-background fallback, then the dark theme. System appearance has priority over terminal appearance. Polling occurs once a second outside the input thread. Manual cycling or confirming a theme disables automatic switching for the session; cancelling the picker retains automatic mode. Form values and focus are preserved on appearance changes.

The whole frame receives the palette foreground/background before widgets render. No terminal OSC palette mutation is used. The execution view receives the same base background while retaining child terminal colours.

Mouse selection of a theme disables automatic appearance changes. Shift+T opens the theme picker, including terminals that report it as lowercase t with the Shift modifier.
