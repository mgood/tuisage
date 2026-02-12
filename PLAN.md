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
          │
          ▼
┌─────────────────────────────────────────────────┐
│               widgets/ (components)             │
│  - command_list.rs  (subcommand browser)        │
│  - flag_list.rs     (flag toggle/value entry)   │
│  - arg_input.rs     (positional arg input)      │
│  - command_preview.rs (live command preview)     │
│  - filter_input.rs  (fzf-style filter bar)      │
└─────────────────────────────────────────────────┘
```

## Development Steps

### Phase 1: Project Setup & Spec Parsing
- [x] Create PLAN.md
- [ ] Initialize Cargo project with dependencies
- [ ] Create sample `.usage.kdl` test fixtures
- [ ] Write tests: parse a usage spec and verify structure
- [ ] Implement basic `App` struct that loads a `Spec`

### Phase 2: Core App State & Command Building
- [ ] Implement navigation state (command path breadcrumbs)
- [ ] Implement flag state tracking (toggled booleans, flag values)
- [ ] Implement arg state tracking (positional arg values)
- [ ] Implement command string builder from current state
- [ ] Write tests: command building from various flag/arg combinations

### Phase 3: Basic TUI Rendering
- [ ] Set up ratatui terminal initialization/cleanup
- [ ] Implement basic layout (3-pane: commands, options, preview)
- [ ] Render subcommand list for current command
- [ ] Render flags list with toggle state
- [ ] Render positional args with input fields
- [ ] Render live command preview at bottom
- [ ] Write snapshot tests for rendering

### Phase 4: Keyboard Navigation
- [ ] Implement focus cycling between panels (Tab/Shift-Tab)
- [ ] Arrow key navigation within lists
- [ ] Enter to select subcommand / toggle flag
- [ ] Text input for flag values and arg values
- [ ] Escape to go back up command tree
- [ ] Ctrl-C / q to quit
- [ ] Enter on preview to accept and print command
- [ ] Write tests for keyboard event handling

### Phase 5: FZF-Style Filtering
- [ ] Implement fuzzy matcher for command/flag lists
- [ ] Add filter input bar that activates on typing
- [ ] Filter subcommand list as user types
- [ ] Filter flag list as user types
- [ ] Write tests for fuzzy matching

### Phase 6: Visual Polish
- [ ] Color scheme: distinct colors for commands, flags, args, values
- [ ] Highlight focused/selected items
- [ ] Show help text for hovered items
- [ ] Status bar with keybinding hints
- [ ] Scrolling for long lists
- [ ] Border styling

### Phase 7: Mouse Support
- [ ] Click to select items in lists
- [ ] Click to focus panels
- [ ] Scroll wheel support

## Dependencies

- `usage-lib` (no default features, skip docs/tera/roff) - parse usage specs
- `ratatui` - TUI framework
- `crossterm` - terminal backend
- `insta` - snapshot testing

## Test Strategy

Tests are structured in layers:
1. **Unit tests**: Individual widget rendering with known usage spec input
2. **Integration tests**: Full app state + rendering for representative specs
3. **Snapshot tests**: Using `insta` to capture and verify terminal output

### Representative Test Fixtures

A sample spec covering:
- Nested subcommands (2-3 levels deep)
- Boolean flags (--force, --verbose)
- Flags with values (--user <user>, --output <file>)
- Required and optional positional args
- Flags with choices
- Global flags
- Aliases

## Key Design Decisions

- **No docs feature**: We only need spec parsing from `usage-lib`, not doc generation
- **Crossterm backend**: Best cross-platform terminal support
- **State machine for focus**: Clean handling of which panel/input is active
- **Command path as Vec<String>**: Navigate subcommand tree by pushing/popping
- **Immediate command preview**: Always show what the current command would be