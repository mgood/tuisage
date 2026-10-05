# Presage: optional enrichment for TuiSage

Presage augments a Usage-driven TuiSage form with reusable choices, dynamic choice
providers, initial values, locked values, additional validation, execution mode
and appearance settings. Those settings live in a separate KDL companion document.
The command continues to describe its grammar through ordinary Usage metadata.

This guide describes the Presage-capable implementation. Presage is an extension;
an unmodified TuiSage installation does not recognise its documents or
options. Check `tuisage --help` for `--presage` and `--selector`.

## What changes, and what stays in Usage

Vanilla TuiSage reads Usage metadata to construct a form for a command. Usage
supplies subcommands, arguments, flags, help, declared choices and completion
metadata. Presage attaches settings to those existing fields without adding new
Usage syntax or making the command implement a form-specific CLI.

| Concern | Usage-driven form | Optional Presage addition |
|---|---|---|
| Grammar | Commands, flags, arguments and requiredness | References existing fields; defines no new grammar |
| Choices | Existing Usage choices and completions | Named fixed lists, executable providers and discovered selector IDs |
| Initial state | Defaults available from the command description | Editable initial values or explicitly locked values |
| Validation | Grammar and declared constraints | A command-specific validation executable |
| Submission | Normal command execution | A document-selected execute or compose mode |
| Appearance | TuiSage appearance options | Reusable theme and light/dark selections |

A companion is optional. Without a matching document, the normal Usage path
applies. An invalid selected document is an error, rather than a reason
to silently discard validation or locks. Presage does not weaken Usage constraints:
a suggested value still has to satisfy the command's grammar.

## A first companion

Suppose `demo --usage` emits this Usage document:

```kdl
name "demo"
arg "[region]"
```

Install the following as `demo.tuisage.kdl` beside the resolved `demo` executable:

```kdl
version 1
default "root/args/region" json="\"Sydney\""
list "regions" {
    choice "Sydney" description="Australia"
    choice "Zürich" description="Switzerland"
}
field "root/args/region" list="regions"
```

Open the form normally:

```sh
tuisage demo --usage
```

The region field starts with Sydney and offers the named list. Descriptions are
presentation text; the selected value is passed as an argument unchanged. The
list is a suggestion source, not a closed enum. Use a Usage choice constraint or
additional validation when other values must be rejected.

## Document identity and discovery

The default filename is `<command>.tuisage.kdl`. A selector changes it to
`<command>.<selector>.tuisage.kdl`:

```sh
tuisage --selector staging demo --usage
```

This selects `demo.staging.tuisage.kdl`. Selectors accept letters, digits, dots,
underscores and hyphens. They are TuiSage invocation settings, not arguments added
to the command. A selector lookup never substitutes the unqualified document.

An explicit document can be selected instead:

```sh
tuisage --presage /opt/demo/forms/custom.tuisage.kdl demo --usage
```

An explicit file cannot be combined with `--selector`. A directory can:

```sh
tuisage --presage /opt/demo/forms --selector staging demo --usage
```

TuiSage searches that directory before the automatic tiers below, using the normal
command and selector filename. Selector enumeration includes that directory with
the same precedence. Missing documents continue through the automatic cascade;
a missing selector never falls back to an unqualified filename. A nonexistent
explicit path is an error, so create a directory before supplying it.

Automatic discovery checks these tiers in order and selects the first existing document:

| Tier | macOS | Linux | Windows |
|---|---|---|---|
| User | `$HOME/Library/Application Support/TuiSage/commands` | `$XDG_DATA_HOME/tuisage/commands`, defaulting to `$HOME/.local/share/tuisage/commands` | `%LOCALAPPDATA%\TuiSage\commands` |
| Executable | Beside the resolved command executable | Same | Same |
| Shared | `/Library/Application Support/TuiSage/commands` | Each `$XDG_DATA_DIRS` directory with `/tuisage/commands` appended; defaults are `/usr/local/share` and `/usr/share` | `%PROGRAMDATA%\TuiSage\commands` |

Documents are not merged. A higher-priority document replaces the lower-priority
one in full. Executable symlinks are resolved before sidecar lookup. PATH lookup
uses absolute directory entries. There is no implicit current-directory search.

Application-data roots must be absolute. An absent, empty or relative HOME omits
the HOME-based macOS user tier. On Linux an absent, empty or relative
`XDG_DATA_HOME` falls back to an absolute HOME plus `.local/share`. Relative
`XDG_DATA_DIRS` entries are ignored; absent or empty `XDG_DATA_DIRS` uses the
defaults above. On Windows an absent, empty or relative `LOCALAPPDATA` or
`PROGRAMDATA` omits that tier. Windows uses the Local AppData and ProgramData
known folders.

## Field identifiers

Bindings and defaults use identifiers for existing Usage fields:

| Field | Identifier |
|---|---|
| Global flag | `global/flags/profile` |
| Root-local flag | `root/flags/format` |
| Root argument | `root/args/region` |
| Subcommand flag | `commands/deploy/flags/format` |
| Nested subcommand argument | `commands/project/create/args/name` |

Within a path segment, `~` is escaped as `~0` and `/` as `~1`. A `field` binding
must name an existing field and an existing list. Unknown fields, unknown lists,
duplicate bindings and unsupported nodes are document errors.

## Fixed, dynamic and selector lists

A fixed list contains `choice` nodes, as in the first example. An executable list
names an executable followed by literal arguments:

```kdl
list "accounts" {
    provider "demo_choices.py" "accounts"
}
field "root/args/account" list="accounts"
```

This example requires an `account` argument in the Usage document. Presage starts
the executable directly. It does not interpret its argv as a shell command.
Relative provider and validator paths resolve beside the selected companion,
not through PATH. Absolute paths remain absolute. A script must be executable
and its interpreter must be available in the inherited environment.

Providers receive a version-1 JSON request on standard input. For example:

```json
{
  "version": 1,
  "executable": "demo",
  "argv": ["--profile", "staging"],
  "command": [],
  "field": "root/args/account",
  "fields": {
    "global/flags/profile": "staging",
    "root/args/account": null
  }
}
```

`command` is the selected subcommand path. `fields` contains current values keyed
by canonical field identifier. Values can be strings, booleans, counts, arrays
or null according to field type and whether a value is supplied. Providers should
use this context when another field controls the available choices.

A successful choice response has this shape:

```json
{
  "version": 1,
  "choices": [
    {"value": "research", "description": "Research account"},
    {"value": "personal"}
  ]
}
```

Write protocol JSON to stdout and diagnostics to stderr. Choice values are kept
separate from descriptions and passed as exact argument values, including spaces
and Unicode. Empty strings can be represented explicitly.

Fixed lists are available immediately. Executable lists run asynchronously and
show `Loading...` while pending. Context changes refresh an open completion;
responses from an old request or old context are discarded. Empty results and
provider failures leave manual entry available. The provider deadline is five
seconds, including input/output collection. Timeout kills the direct child;
cleanup of descendants is not guaranteed.

A selector list is another fixed source:

```kdl
list "profiles" { selectors }
field "global/flags/profile" list="profiles"
```

TuiSage enumerates matching `<command>.<selector>.tuisage.kdl` files across the same
cascade, retaining the first path for each selector. Their IDs become the choices.
Enumeration does not merge their contents. Choosing such a value in a command
field does not reload another companion; `--selector` chooses the document at
startup. A list must use exactly one kind of source: choices, a provider or selectors.

## Defaults, locks and validation

Defaults carry a JSON value inside the KDL `json` property:

```kdl
default "global/flags/profile" json="\"staging\"" locked=#false
default "root/flags/dry-run" json="true" locked=#true
```

Values must match the Usage field's type. The omitted `locked` property defaults
to false. Locked values cannot be changed through editing, completion or reset.
An invocation cannot replace a lock with a conflicting value. Repeated fields
can take JSON arrays; an empty array omits the value, while `[""]` supplies an
explicit empty string.

For additional command-specific checks, name a validator:

```kdl
validate "demo_validate.py"
```

It receives the same versioned context on stdin at submission. Success is:

```json
{"version":1,"errors":{}}
```

To reject a submission, return messages keyed by field identifier:

```json
{"version":1,"errors":{"root/args/region":"Choose a supported region."}}
```

A rejection leaves the form open without running or returning the command.
Nonzero exit, malformed output, an unsupported response version or timeout also
blocks submission. Unlike a choice-provider failure, validation failure cannot
be ignored to continue. Validators should check values without executing the
operational command or causing other side effects.

## Mode, appearance and invocation precedence

A presage companion file can contain:

```kdl
mode "compose"
theme "auto" light="catppuccin-latte" dark="tokyo-night"
```

`execute` runs the command through TuiSage's normal execution view. `compose`
returns a JSON object containing `executable` and `argv`, then exits without
running it. The argv array excludes the executable. The command preview is for
display and copying; consumers should use the structured values directly.

Explicit `--compose` or `--execute` overrides document mode. Explicit `--theme`,
`--theme-light`, `--theme-dark` and `--validate` override their corresponding
settings. `--defaults` adds or replaces unlocked document defaults; conflicting
locks are errors. Grammar requirements still apply to every submission.

The Usage source and executable identity remain invocation choices through the
trailing Usage command, `--spec-file` and `--cmd`. A companion does not redefine
the command grammar or its executable identity.

## Inspect the selected document

Bare `--presage` queries discovery and validates the selected document without
opening a terminal form:

```sh
tuisage --presage -- demo --usage
tuisage --presage --selector staging -- demo --usage
```

To inspect an explicit directory without opening the form, include a bare query
occurrence as well as the path occurrence:

```sh
tuisage --presage /opt/demo/forms --presage --selector staging -- demo --usage
```

The same query syntax works with an explicit file, without `--selector`. At most
one explicit path is accepted. A bare occurrence requests JSON rather than a form.

Use `--` before a trailing Usage command so its executable is not consumed as an
explicit document path. A query returns:

```json
{
  "version": 1,
  "path": "/opt/demo/bin/demo.staging.tuisage.kdl",
  "selectors": {
    "staging": "/opt/demo/bin/demo.staging.tuisage.kdl"
  }
}
```

`path` is null when no document matches. Other selectors are listed by filename
without parsing each document. A valid query does not prove that a provider or
validator can execute: those executables run later when needed. `--presage PATH` alone selects a file or directory for a form; include the bare
occurrence when querying it.

## Packaging, trust and troubleshooting

Ship companion documents separately from Usage metadata. Keep relative providers
and validators beside their document, or use deliberate absolute executable paths.
Moving a document into the user override tier changes both its precedence and the
base directory for relative executable paths. Runtime KDL needs no authoring tool.

Presage discovers and reads files; it does not generate, install, update, migrate
or remove them. The application or package installer owns those operations,
including preservation across executable replacement. Test the actual installed
paths after an upgrade instead of assuming a source-tree check covers packaging.

A trusted document can launch executable providers and validators. Inspect both
before installation. Restrict write access to locations used for automatic lookup.
Providers inherit the launching environment, which may differ between a terminal
and a desktop launcher. Do not embed credentials in companion files or diagnostics.

| Symptom | Check |
|---|---|
| Query returns null | Resolved executable, filename, selector and absolute data roots |
| Unexpected document wins | User override tier and the query's selected path |
| Explicit selection fails | Path exists, the selected document is valid, and selectors are used with directories rather than explicit files |
| List fails or stays empty | Executable path relative to the selected document, permissions, interpreter, context and response JSON |
| Submission is blocked | Usage constraints, locks and validator diagnostics |
| Works in a shell only | Desktop launch PATH and other required environment values |
| Values become stale | Provider uses current field context and does not rely on stale external state |

The absence of a companion is a supported Usage-only case. Invalid selected
configuration is reported explicitly so its constraints are never silently lost.
