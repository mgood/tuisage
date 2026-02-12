# TuiSage Development Plan

## Overview

TuiSage is a TUI application that reads a `usage` spec (KDL format) and presents an interactive interface for building CLI commands. Users can browse subcommands, toggle flags, fill in argument values, and produce a complete command string.

## Architecture

```
┌─────────────────────────────────────────────────┐
│                   main.rs                       │
│  - CLI arg parsing (file path)                  │
│  - Parse usage spec via usage-lib               │
│  - Initialize App state                         │
│  - Run TUI event loop                           │
│  - Print final command on exit                  │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                   app.rs                        │
│  - App struct (holds Spec, navigation state)    │
│  - Command path (breadcrumb of selected cmds)   │
│  - Flag values, arg values                      │
│  - Focus state (which panel/widget is active)   │
│  - Command builder (assembles final command)    │
└─────────────────────────────────────────────────┘
          │
          ▼
┌─────────────────────────────────────────────────┐
│                   ui.rs                         │
│  - Layout: command tree, options panel, preview │
│  - Renders current state to terminal frame      │
└─────────────────────────────────────────────────┘
```

## Development Steps

### Phase 1: Project Setup & Spec Parsing
- [x] Create PLAN.md
- [x] Initialize Cargo project with dependencies
- [x] Create sample `.usage.kdl` test fixtures
- [x] Write tests: parse a usage spec and verify structure
- [x] Implement basic `App` struct that loads a `Spec`

### Phase 2: Core App State & Command Building
- [x] Implement navigation state (command path breadcrumbs)
- [x] Implement flag state tracking (toggled booleans, flag values)
- [x] Implement arg state tracking (positional arg values)
- [x] Implement command string builder from current state
- [x] Write tests: command building from various flag/arg combinations

### Phase 3: Basic TUI Rendering
- [x] Set up ratatui terminal initialization/cleanup
- [x] Implement basic layout (3-pane: commands, options, preview)
- [x] Render subcommand list for current command
- [x] Render flags list with toggle state
- [x] Render positional args with input fields
- [x] Render live command preview at bottom
- [x] Write snapshot tests for rendering (13 insta snapshots)

### Phase 4: Keyboard Navigation
- [x] Implement focus cycling between panels (Tab/Shift-Tab)
- [x] Arrow key navigation within lists (+ hjkl vim keys)
- [x] Enter to select subcommand / toggle flag
- [x] Text input for flag values and arg values
- [x] Escape to go back up command tree
- [x] Ctrl-C / q to quit
- [x] Enter on preview to accept and print command
- [x] Write tests for keyboard event handling

### Phase 5: FZF-Style Filtering
- [x] Implement fuzzy matcher for command/flag lists (basic subsequence match)
- [x] Add filter input bar that activates on typing `/`
- [x] Filter subcommand list as user types
- [ ] Filter flag list as user types
- [x] Write tests for fuzzy matching
- [ ] Improve fuzzy matching with scoring/ranking (fzf-style)

### Phase 6: Visual Polish
- [x] Color scheme: distinct colors for commands, flags, args, values
- [x] Highlight focused/selected items
- [x] Show help text for hovered items
- [x] Status bar with keybinding hints
- [ ] Scrolling for long lists (scroll offsets exist but unused)
- [x] Border styling
- [ ] Show aliases inline in command list with dimmed style
- [ ] Show default values for flags

### Phase 7: Mouse Support
- [ ] Click to select items in lists
- [ ] Click to focus panels
- [ ] Scroll wheel support

### Phase 8: Additional Features
- [ ] Copy command to clipboard (in addition to print on exit)
- [ ] Execute command directly from TUI (optional mode)
- [ ] Support reading usage spec from stdin
- [ ] Support script-embedded USAGE blocks (usage-lib already supports this)

### Phase 9: Code Quality & CI
- [ ] Remove unused fields (command_scroll, flag_scroll) or implement scrolling
- [ ] Remove unused `breadcrumb()` method or use it
- [ ] Split app.rs into smaller modules (state, input, builder)
- [ ] Consider splitting ui.rs into widget modules
- [ ] Add CI (GitHub Actions: cargo test, cargo clippy, insta checks)
- [ ] Add README with usage examples and screenshots

## Dependencies

- `usage-lib` (no default features, skip docs/tera/roff) - parse usage specs
- `ratatui` - TUI framework
- `crossterm` - terminal backend
- `color-eyre` - error reporting
- `insta` - snapshot testing (dev)
- `pretty_assertions` - nicer test diffs (dev)

## Test Strategy

Tests are structured in layers:
1. **Unit tests**: App state logic, command building, fuzzy matching (19 tests in app.rs)
2. **Rendering tests**: Assertion-based checks for specific UI elements (11 tests in ui.rs)
3. **Snapshot tests**: Using `insta` to capture full terminal output (13 snapshots in ui.rs)

Total: 43 tests passing.

### Representative Test Fixtures

A sample spec (`fixtures/sample.usage.kdl`) covering:
- Nested subcommands (2-3 levels deep: plugin > install/remove)
- Boolean flags (--force, --verbose, --rollback)
- Flags with values (--tag <tag>, --output <file>)
- Required and optional positional args
- Flags with choices (environment: dev|staging|prod)
- Global flags (--verbose, --quiet)
- Aliases (set/add, list/ls, remove/rm)
- Count flags (--verbose with count=true)

## Key Design Decisions

- **No docs feature**: We only need spec parsing from `usage-lib`, not doc generation
- **Crossterm backend**: Best cross-platform terminal support
- **State machine for focus**: Clean handling of which panel/input is active
- **Command path as Vec<String>**: Navigate subcommand tree by pushing/popping
- **Immediate command preview**: Always show what the current command would be
- **Single-file modules**: app.rs and ui.rs kept as single files for now; split when they grow further