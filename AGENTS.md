# AGENTS.md

## Project Overview

`tuitest` is a terminal command runner TUI built in Rust using [Ratatui](https://docs.rs/ratatui/latest/ratatui/) and [Crossterm](https://docs.rs/crossterm/latest/crossterm/). It presents a split interface inspired by the pi.dev aesthetic: an asynchronous, scrollable output feed above, an empty separator space, a clean horizontal-ruled command input prompt, and a minimal footer status bar.

## Architecture & Codebase Map

```
src/
├── main.rs              # Application entrypoint & event loop (tokio::select!)
├── app.rs               # Domain state machine (App, InputState, FeedItem, ScrollState)
├── runner.rs            # Async subprocess runner ($SHELL -c <cmd>) and stream reader
├── terminal.rs          # Alternate screen, raw mode, and panic recovery guard (TerminalGuard)
└── ui/
    ├── mod.rs           # Master layout coordinator (Feed, Separator, Input, Footer)
    ├── feed.rs          # Output area (virtual scrolling, bottom-anchoring, line slicing)
    ├── blocks.rs        # Visual card renderers (UserInputBlock, CommandOutputBlock, duration)
    ├── input.rs         # User input row (top & bottom divider rules, cursor positioning)
    └── footer.rs        # Bottom status indicator (READY / RUNNING) and scrollback hints
```

### Key Components

- **`src/main.rs`**: Single asynchronous event loop using `tokio::select!` multiplexing `crossterm::event::EventStream`, background process output (`runner_rx`), and tick intervals. Zero unnecessary background event-forwarder threads.
- **`src/app.rs`**:
  - `InputState`: UTF-8-safe text entry buffer with byte-indexed character insertion, backspace, delete, and arrow navigation.
  - `ScrollState`: Virtual scroll calculations with sticky auto-scroll when at the bottom, and manual scrollback offset tracking.
  - `FeedItem`: Semantic enum containing `UserInputBlock` and `CommandOutputBlock`.
  - `App`: Central state store managing process abort handles, input, and feed history.
- **`src/runner.rs`**: Executes commands via `$SHELL -c` (fallback to `/bin/sh -c`), streaming stdout and stderr asynchronously while stripping raw ANSI escape sequences. Uses Tokio's native `AbortHandle` for immediate `Ctrl+C` process cancellation.
- **`src/terminal.rs`**: RAII struct `TerminalGuard` enabling raw mode, alternate screen, and mouse capture. Installs a panic hook to guarantee terminal restoration even on unexpected panics.
- **`src/ui/`**: Modularized presentation layer.
  - `blocks.rs`: Renders distinct bordered cards for user input and command output with clean duration formatting (`Took 0.0s`) and exit codes (`[Exit 1]`).
  - `feed.rs`: Computes visible slices for the feed and pushes blocks down to sit right above the input prompt.
  - `input.rs`: Renders the input area bounded by top and bottom horizontal divider rules without prompt characters (`>`) or side borders.
  - `footer.rs`: Renders `READY` or `RUNNING` status and active scrollback offsets.

## Development Workflows & Quality Gates

Run these commands before submitting any changes:

```bash
# Format check
cargo fmt --check

# Strict Clippy lint check
cargo clippy -- -D warnings

# Run all unit and integration tests
cargo test

# Build debug binary
cargo build

# Build optimized release binary
cargo build --release

# Launch application
cargo run
```

## Key UX & Styling Rules

1. **User Input Area**: Bounded only by top and bottom horizontal rules `─...─` with no side borders (`│`) and no prompt symbol (`>`).
2. **Empty Space Separator**: Exactly 1 blank row separates the output feed from the input area's top rule.
3. **Scroll Notification**: When scrolled up into history, the scroll offset notice is displayed in the **footer** next to `READY` (e.g. ` READY   Scrollback active: 4 lines up (PgDn / End to return)`). Do not draw scroll indicator text into the input area's divider rules.
4. **Command Output Blocks**:
   - `$ <command>` is placed on the first row inside the block.
   - A blank spacer line precedes and follows output lines.
   - If a command produces no stdout/stderr, `(no output)` is rendered in dimmed text.
   - If a command fails (non-zero exit code), `Command exited with code <N>` is displayed in bold red.
   - The last row displays `Took X.Xs` in dimmed text with 1-decimal-place precision (e.g. `Took 0.0s` or `Took 12.5s`).
5. **Right Border Alignment**: All block lines must be sized with `fit_to_width` using `unicode-width` and `(area.width - 1)` to prevent right border clipping in terminal emulators.
