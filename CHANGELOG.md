# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.4](https://github.com/mgood/tuisage/compare/v0.1.3...v0.1.4) - 2026-04-16

### Added

- *(ui)* cleaner styling for keyboard shortcut bar ([#19](https://github.com/mgood/tuisage/pull/19))

### Other

- Fix test failures from "echo" change

## [0.1.3](https://github.com/mgood/tuisage/compare/v0.1.2...v0.1.3) - 2026-03-23

### Other

- add notes on binary installation

## [0.1.2](https://github.com/mgood/tuisage/compare/v0.1.1...v0.1.2) - 2026-03-23

### Other

- install directions for crates.io

## [0.1.1](https://github.com/mgood/tuisage/compare/v0.1.0...v0.1.1) - 2026-03-23

### Other

- release 0.1.0
- Use "echo" for the sample command
- Add support for flag negation pattern (tristate) ([#11](https://github.com/mgood/tuisage/pull/11))
- add mise tasks for sample and install
- Fix demo.tape
- Create CONTRIBUTING.md
- Add snapshot tests for panel and select scrollbars
- Add hover highlight for items under the mouse cursor
- Add scrollbar widget to Commands, Flags, and Args panels
- Fix select box mouse handling with scroll offset and scrollbar
- Remove completion caching, run fresh each time
- Update docs: remove completion caching, run fresh each time
- Remove vscode settings
- Replace --spec-cmd flag with trailing arguments
- Update docs to replace --spec-cmd with trailing arguments
- Link to hosted demo GIF
- Add note on terminal compatibility
- Add MIT license
- Clean up README
- Add vhs "tape" for recording demos
- Use compact keyboard shortcut glyphs in help bar
- Update docs for padding removal, filter_active, and 199 tests
- Remove panel padding for consistency, show all choices on select reopen
- Update IMPLEMENTATION.md for selection background and help overlay changes
- Add selection background to Commands panel and align help text
- Update IMPLEMENTATION.md for help text overlay rendering
- Use layout-based right-alignment for help text instead of space padding
- Fix overflow crash when reopening choice select
- Remove top border from choice select and align with text
- Update docs for unified choice select + text input UX
- Unify choice select with inline text input
- Update IMPLEMENTATION.md for dynamic completions feature
- Add dynamic completions from usage spec complete directives
- Add dynamic completions feature to requirements and specification
- Update docs for custom Widget implementations
- Extract CommandPreview, HelpBar, and SelectList as Widget implementations
- Update IMPLEMENTATION.md for widgets.rs and navigation refactoring
- Extract reusable widget helpers and consolidate navigation logic
- Update docs: test counts, Backspace clear behavior, right-aligned theme indicator
- Right-align theme name with T: prefix, Backspace clears flags and args
- Update docs: remove print-only mode references, fix test counts, update help bar description
- Remove dead code: current_help() and its test
- Update docs for theme picker feature
- Add theme picker: T or click theme name to open, live preview, Enter/Esc
- Remove help text line, add help text to Arguments panel
- Remove print-only mode (p key on Preview)
- Align choice select box with value position, add mouse click selection
- Update IMPLEMENTATION.md for completed features
- Add inline choice select box for flags and args with choices
- Move Command Preview pane to top of screen
- Fix filter auto-select not working in Arguments panel
- Change Ctrl+Enter to Ctrl+R for executing commands from any panel
- Update docs: Ctrl+R shortcut, preview at top, args auto-select, inline choice select box
- Update docs: Args panel now supports filtering
- Implement filtering for Args panel
- Update docs for simplified Esc and Ctrl+Enter shortcut
- Simplify Esc behavior and add Ctrl+Enter global execute
- Update docs for two-phase filter behavior and add applied-filter help bar
- Fix applied filter behavior: nav, highlighting, Esc, and pane switching
- Update docs for filter mode fixes and emoji indicator
- Fix filter mode bugs and improve UI
- Update IMPLEMENTATION.md test count to 148
- Fix startup sync, global flags, filtered navigation, filter visual cues, and separate name/help matching
- Update planning docs for filter improvements, startup sync, and global flags fix
- Remove border from output pane during command execution
- Add vertical line prefix to subcommands for clearer hierarchy
- Fix PTY resize using in-place screen_mut().set_size()
- Fix PTY resize to properly recreate vt100 parser
- Implement PTY resize feature
- Remove breadcrumb bar references from documentation
- Remove dead_code annotation from prev_theme (now used by [ key)
- Rename misleading breadcrumb tests to command path visibility tests
- Add ]/[ theme cycling, Enter on Commands navigation, and p print-only mode
- Update planning docs to accurately reflect implementation state
- Update docs for command execution feature
- Add command execution with embedded PTY terminal
- Update snapshots after removing dash separator
- remove dash before help text
- Fix cursor position display and improve description alignment
- Improve UI layout and description styling
- Update docs and mise.toml for new CLI flags
- Refactor CLI: add --cmd, --spec-cmd, --spec-file flags; remove stdin support
- Add clap and clap_usage for argument parsing and usage spec generation
- Add basic mise config
- Update IMPLEMENTATION.md and SPECIFICATION.md for flat list approach
- Switch to flat list with compact indentation, full-path matching, fix navigation
- Replace TreeView with flat list rendering for full styling control
- Fix character highlighting visibility on selected items
- Update SPECIFICATION.md for Pattern-based filtering
- Update IMPLEMENTATION.md to reflect Pattern-based matching
- Add test for Pattern-based fuzzy matching with indices
- Use Pattern instead of Atom for proper fuzzy matching
- Update IMPLEMENTATION.md for character highlighting
- Implement color-based filtering and match highlighting
- Update docs for color-based filtering and match highlighting
- Implement subdued filtering with auto-selection
- Update planning docs for subdued filtering design
- Update docs for nucleo-matcher integration
- Integrate nucleo-matcher for scored fuzzy filtering
- Update docs for breadcrumb removal and tree structure changes
- Remove breadcrumb and root node from tree
- Update docs for panel visibility fix
- Fix panel visibility: commands tree and arguments always visible
- Update docs for TreeView command navigation
- Replace flat command list with TreeView for command selection
- Update docs for TreeView implementation
- Remove library references from REQUIREMENTS.md; move to SPECIFICATION.md
- splitting up planning documents into requirments/specification
- Document new keyboard inputs
- Document keyboard interactions and visual indicators in README
- Fix 3 UX bugs: checkmark style, shared edit_input state, count flag decrement
- Document some more design requirements
- List some additional crates to use for features
- Add .gitignore for build outputs
- more detail on dev process
- Add some more detail on testing
- Initial description of the application design

## [0.1.0](https://github.com/mgood/tuisage/releases/tag/v0.1.0) - 2026-03-13

### Other

- Initial release
