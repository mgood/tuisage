# Overview
Create a TUI application for interacting with commands based on the `usage` specification:
https://usage.jdx.dev/spec/

The `usage` source code is here https://github.com/jdx/usage

The TUI should be built using https://github.com/ratatui/ratatui

For an initial proof of concept, this app should take in a usage file like `mycli.usage.kdl`, show the user an interactive command to select the options from a TUI, then print the full command and return.

The typical workflow will allow the user to select from a list of commands, interactively set options specific to that command to produce a full command that they could run.

# Development process

Start by producing a PLAN.md to outline the development steps, then build an initial proof-of-concept demonstrating the main features, once that's complete we can iterate on UX and additional features.
You are working in a git branch, so use local commits to checkpoint development progress along the way and give history if you get stuck on anything.

# Testing
Development should follow a testing-focused development process. Tests should be written as an integral part of making any changes. When possible tests should be written first to reproduce an issue, then update the code to pass the test. However, tests may also be used to inspect the current behavior and then make modifications to acheive the desired result.
Follow testing patterns for Raratui apps: https://ratatui.rs/recipes/testing/snapshots/
Start by testing individual components for handling different types of input, then test for a larger combination of options in a full app.
Tests should be structured to specify the usage.kdl formatted input, then test the resulting UI.

Start with a selection of a few different commands with a representative set of arguments.

Widgets that take text input should use fzf-style matching to allow the user to type a name to filter the list of choices.

The TUI should present a friendly UI that would be geared toward repeatedly running various commands for a tool. It should make efficient use of terminal coloring to help the user navigate and quickly use the application. It should prioritize fast keyboard navigation, though should support mouse inputs as well for selecting from widgets.

# Features
## Fuzzy matching
Use https://crates.io/crates/nucleo-matcher to filter options based on fuzzy matching

## Colors
We'll use https://crates.io/crates/ratatui-themes to help ensure the colors follow a consistent pattern. Later we could consider a theme picker.

## UI Components
We should use components from here to handle mouse and focus management, and a number of good basic UI components:
https://crates.io/crates/ratatui-interact
