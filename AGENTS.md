# AGENTS.md — Development Guidelines

This file contains instructions for AI agents (and developers) working on TuiSage. It defines the development process, coding standards, testing philosophy, and workflow conventions.

## Project Context

TuiSage is a terminal UI for interactively building CLI commands from [usage](https://usage.jdx.dev/) specs. It reads `.usage.kdl` files and presents an interactive interface for browsing subcommands, toggling flags, filling in arguments, and producing a complete command string.

Refer to these documents for increasing levels of detail:

- **REQUIREMENTS.md** — what the application should do (goals, features, user stories)
- **SPECIFICATION.md** — how features should behave (UI layout, interactions, data flow)
- **IMPLEMENTATION.md** — how it's built (architecture, code structure, current state)
- **README.md** — user-facing guide (installation, usage, keyboard shortcuts)

## Development Process

- Work in a git branch. Use local commits to checkpoint progress and provide history if you get stuck.
- Before starting a new feature or fix, review the current state of the codebase and relevant documentation.
- After completing work, update the living documentation (REQUIREMENTS.md, SPECIFICATION.md, IMPLEMENTATION.md, README.md) to reflect changes.
- Start by producing or updating a plan before diving into implementation. Think through the approach, edge cases, and testing strategy.

## Testing Philosophy

Development follows a **testing-focused process**. Tests are an integral part of every change.

### Test-First When Possible

- When fixing a bug, write a test that reproduces the issue first, then update the code to pass the test.
- When adding a feature, write tests that define the expected behavior, then implement to satisfy them.
- Tests may also be used to inspect current behavior before making modifications.

### Test Layers

Tests are structured in layers of increasing scope:

1. **Unit tests** — app state logic, command building, fuzzy matching (in `app.rs`)
2. **Rendering tests** — assertion-based checks for specific UI elements (in `ui.rs`)
3. **Snapshot tests** — full terminal output captured via `insta` (in `ui.rs`)

### Test Patterns

Follow testing patterns for ratatui apps: https://ratatui.rs/recipes/testing/snapshots/

- Test fixtures should use `.usage.kdl` formatted input, then verify the resulting UI or state.
- Use a representative set of commands covering: nested subcommands, boolean flags, flags with values, positional args, choices, aliases, count flags, and global flags.
- Snapshot tests use `insta`. Review changes with `cargo insta review` and accept new snapshots with `cargo insta test --accept`.

### Running Tests

```sh
# Run all tests
cargo test

# Review snapshot changes interactively
cargo insta review

# Run with acceptance of new snapshots
cargo insta test --accept
```

## Coding Standards

### Rust Conventions

- Target zero clippy warnings: `cargo clippy -- -W warnings`
- Use `color-eyre` for error handling and reporting.
- Keep modules focused: `app.rs` handles state and logic, `ui.rs` handles rendering, `main.rs` is the entry point and event loop.

### Dependencies

Only add dependencies when they provide clear value. Current key dependencies:

| Crate | Purpose |
|---|---|
| `usage-lib` | Parse usage specs (KDL format), no default features |
| `ratatui` | TUI framework |
| `crossterm` | Terminal backend and events |
| `ratatui-interact` | UI components (Breadcrumb, Input, focus management) |
| `ratatui-themes` | Consistent color theming |
| `color-eyre` | Error reporting |
| `insta` | Snapshot testing (dev) |
| `pretty_assertions` | Better test diffs (dev) |

### Code Organization

- Prefer single-file modules until complexity warrants splitting.
- Keep state management (navigation, flag values, arg values) in `app.rs`.
- Keep all rendering logic in `ui.rs`.
- The event loop in `main.rs` should remain thin — delegate to `app.handle_key()` and `app.handle_mouse()`.

## UI Design Principles

- **Keyboard-first**: Prioritize fast keyboard navigation with vim-style keys.
- **Mouse-supported**: Support mouse clicks, scroll wheel, and click-to-activate as secondary input.
- **Live feedback**: Always show the assembled command updating in real time.
- **Efficient use of color**: Use terminal colors to help users navigate quickly. Follow the active theme palette for consistency.
- **Accessible indicators**: Use clear symbols (✓/○ for checkboxes, ▶ for selection caret) that render well across terminal emulators.

## Commit Conventions

- Make focused commits that address one concern at a time.
- Include test changes alongside the code changes they verify.
- Write descriptive commit messages that explain what changed and why.

## Feature Development Workflow

1. Understand the requirement (check REQUIREMENTS.md)
2. Review the specification (check SPECIFICATION.md)
3. Write or update tests that define expected behavior
4. Implement the feature or fix
5. Verify all tests pass (`cargo test`)
6. Verify no clippy warnings (`cargo clippy`)
7. Update documentation to reflect the change
8. Commit with a descriptive message