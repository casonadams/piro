# AGENTS.md

## Project Overview

`tuitest` is a high-performance terminal command runner TUI written in Rust using [Ratatui](https://docs.rs/ratatui/latest/ratatui/) and [Crossterm](https://docs.rs/crossterm/latest/crossterm/). It delivers an asynchronous, non-blocking interface inspired by the pi.dev aesthetic:
- A scrollable, bottom-anchored output feed above.
- An empty space separator.
- A horizontal-ruled command input prompt (clean lines, no side borders, no prompt symbol).
- A minimal bottom footer status bar with integrated scrollback indicators.

## Architectural Design & Codebase Map

```
src/
├── main.rs              # Entry point & async event loop (tokio::select!)
├── app.rs               # Pure domain state machine (App, InputState, FeedItem, ScrollState)
├── runner.rs            # Async subprocess runner ($SHELL -c <cmd>) and stream reader
├── terminal.rs          # Alternate screen, raw mode, and panic recovery guard (TerminalGuard)
└── ui/
    ├── mod.rs           # Master layout coordinator (Feed, Separator, Input, Footer)
    ├── feed.rs          # Output area (virtual scrolling, bottom-anchoring, line slicing)
    ├── blocks.rs        # Visual card renderers (UserInputBlock, CommandOutputBlock, duration)
    ├── input.rs         # User input row (top & bottom divider rules, cursor positioning)
    └── footer.rs        # Bottom status indicator (READY / RUNNING) and scrollback hints
```

### Module Responsibilities

- **`src/main.rs`**: Directly multiplexes terminal events, background process messages, and timer ticks via `tokio::select!`.
  - Routes keys based on `app.mode` (`Mode::Input` vs `Mode::Scroll`).
  - Direct selection on `crossterm::event::EventStream`, `tokio::sync::mpsc::UnboundedReceiver<RunnerMsg>`, and `tokio::time::Interval`.
- **`src/app.rs`**:
  - `Mode`: Enum with `Input` and `Scroll`.
  - `InputState`: Lean, UTF-8-safe text entry buffer using string byte indexing.
  - `ScrollState`: Virtual scroll calculations with sticky auto-scroll when at the bottom, and manual scrollback offset tracking.
  - `FeedItem`: `UserInput(String)` and `CommandOutput(CommandOutputBlock)`.
  - `App`: Central state store managing process `AbortHandle`, input state, mode, and feed history.
- **`src/runner.rs`**: Executes commands asynchronously via `$SHELL -c` (fallback to `/bin/sh -c`), streaming stdout and stderr line-by-line while stripping raw ANSI escape sequences. Uses Tokio's native `AbortHandle` for instant cancellation on `Ctrl+C`.
- **`src/terminal.rs`**: RAII `TerminalGuard` enabling raw mode, alternate screen, and mouse capture. Installs a custom panic hook ensuring the terminal is restored to normal mode under all crash conditions.
- **`src/ui/`**: Modular presentation layer.
  - `blocks.rs`: Renders distinct bordered cards for user input and command output using reusable `box_border` and `box_line` helpers.
  - `feed.rs`: Computes visible slices for the feed, handles line padding to anchor content at the bottom of the feed area when content is shorter than the viewport.
  - `input.rs`: Renders the input area bounded by top and bottom horizontal divider rules with the active cursor.
  - `footer.rs`: Renders `READY` or `RUNNING` status and active scrollback notifications.

## Engineering Rigor & Size Invariants

All future contributions and modifications MUST adhere to these strict size and rigor constraints:

1. **Lines of Code (Tokei Budget)**:
   - Total codebase size MUST remain under **800 lines of Rust** (`tokei -e target` < 700 lines of actual code).
   - No single file should exceed 300 lines. Keep components modular, focused, and free of boilerplate.
2. **Binary Footprint Budget**:
   - Release binary size MUST remain under **1.0 MiB** (target: ~960 KiB).
   - Verify with `cargo bloat --release --crates`.
3. **Dependency Zero-Bloat Policy**:
   - Do NOT add heavy or umbrella crates (e.g. use `futures-util` with `default-features = false` instead of `futures`).
   - Do NOT re-add `chrono` or date/time parsing crates unless strictly mandated.
   - Maintain `tokio` with minimal feature set: `["rt-multi-thread", "macros", "process", "io-util", "sync", "time"]`.
   - Maintain `ratatui` with `default-features = false, features = ["crossterm", "std"]` to avoid transitive calendar/time dependencies.
4. **Release Profile Configuration**:
   - Keep `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = true` in `[profile.release]`.

## Scroll Mode & Copy Behavior (Tmux-Style)

The application provides two operational modes:
1. **Input Mode (`Mode::Input`)**:
   - Default mode. Active hardware cursor sits in the input prompt.
   - Keystrokes type commands; `Enter` runs them.
   - `PageUp`, `Ctrl+S`, or `Esc` (when input is empty) enters **Scroll Mode**.
   - Footer displays ` READY ` (or ` RUNNING `).
2. **Scroll Mode (`Mode::Scroll`)**:
   - Navigation & Cursor in feed:
     - `k` / `Up`: move cursor up 1 line
     - `j` / `Down`: move cursor down 1 line
     - `PageUp` / `Ctrl+U`: move cursor up 10 lines
     - `PageDown` / `Ctrl+D`: move cursor down 10 lines
     - `g` / `Home`: jump to top of feed
     - `G` / `End`: jump to bottom of feed
     - `i` / `q`: exit Scroll Mode and return to input prompt.
   - **Visual Line Selection (Tmux-Style)**:
     - Press `v` or `Space` to toggle visual selection anchor.
     - Move with `k`/`j` or `Up`/`Down` to extend the highlighted selection range across lines.
     - Highlighted lines are visually inverted (`Modifier::REVERSED`).
     - Press `y` or `Enter` to yank (copy) selected lines to the system clipboard via OSC 52 escape codes and macOS `pbcopy`.
     - Press `Esc` to cancel visual selection (or exit scroll mode if no selection is active).
   - **Character-Level Mouse Drag Selection (Maki-Style)**:
     - Left-click and drag across any text to select words, spans, or multi-line blocks with **exact character-level precision** (not forced to full lines).
     - The selection is highlighted in real time directly on the Ratatui frame buffer (`Modifier::REVERSED`).
     - Releasing the mouse button scrapes the buffer cells directly, automatically strips box-drawing border glyphs (`│`), and copies clean text to the clipboard.
   - **One-Key Block Copy**:
     - Pressing `b` in Scroll Mode instantly copies the entire output of the current command block to the system clipboard.
   - Footer displays:
     - Default: ` SCROLL MODE   [↑/↓/jk] Move  [v] Select  [b] Copy block  [i/q/Esc] Return `
     - When Selecting (Mouse): ` SELECTING   [Drag across text, release mouse to copy] `
     - When Selecting (Keyboard): ` VISUAL SELECT   [↑/↓/jk] Extend  [y/Enter] Copy  [v/Space/Esc] Cancel `
     - On copy: shows temporary ` [Copied to clipboard!] ` confirmation.

## UI & Layout Invariants

1. **User Input Area**:
   - Bounded **only** by top and bottom horizontal divider rules `─...─` in `Color::DarkGray`.
   - **No side borders** (`│`), **no corner glyphs** (`┌ ┐ └ ┘`), and **no prompt characters** (`>`).
   - Text entry sits directly on the middle row between the two rules with the hardware cursor properly positioned.
2. **Empty Space Separator**:
   - Exactly 1 blank row separates the output feed from the input area's top divider rule.
3. **Scrollback Notifications**:
   - When scrolled up into history, the scroll offset notice is displayed in the **footer** next to `READY` (e.g. ` READY   Scrollback active: 4 lines up (PgDn / End to return)`).
   - Never draw scroll indicator text into the input area's divider rules.
4. **Command Output Blocks**:
   - `$ <command>` is placed on the first row inside the block in bold white.
   - A blank spacer row follows the command line.
   - If a command produces no stdout/stderr, `(no output)` is rendered in dimmed text (`Color::DarkGray` + `Modifier::DIM`).
   - If a command exits with a non-zero code, `Command exited with code <N>` is displayed in bold red followed by a blank spacer row.
   - If a command is cancelled via `Ctrl+C`, `Command cancelled` is displayed in bold yellow followed by a blank spacer row.
   - The final line displays `Took X.Xs` (or `Running X.Xs` while active) in dimmed text with 1-decimal-place precision (`{:.1}s`).
5. **Right Border Alignment**:
   - All block lines must be formatted using `fit_to_width` and `(area.width - 1)` to prevent right border clipping on terminal emulators.

## Development Workflows & Quality Gates

Every change must pass all four quality gates with zero warnings:

```bash
# 1. Format check
cargo fmt --check

# 2. Strict Clippy check (zero warnings permitted)
cargo clippy -- -D warnings

# 3. Unit and integration tests
cargo test

# 4. Code & binary bloat verification
tokei -e target
cargo bloat --release --crates
```
