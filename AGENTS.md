# AGENTS.md

## Project Overview

`piro` is a high-performance terminal command runner TUI written in Rust using [Ratatui](https://docs.rs/ratatui/latest/ratatui/) and [Crossterm](https://docs.rs/crossterm/latest/crossterm/). It delivers an asynchronous, non-blocking interface inspired by the pi.dev aesthetic:
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
- **`src/runner.rs`**: Executes commands asynchronously via `$SHELL -c` (fallback to `/bin/sh -c`), streaming stdout and stderr line-by-line while preserving rich ANSI escape sequences parsed via `ansi-to-tui`.
  - Spawns subprocesses with `CLICOLOR=1`, `CLICOLOR_FORCE=1`, `FORCE_COLOR=1`, `COLORTERM=truecolor`, and `TERM=xterm-256color` so that CLI tools (`cargo`, `git`, `ls`, `grep`) emit full terminal colors even when piped.
  - Spawns with `process_group(0)` and `kill_on_drop(true)`.
  - Exposes `ProcessHandle` which uses `libc::kill(-pid, SIGKILL)` to immediately terminate the entire process group in its tracks on `Esc` or `Ctrl+C`.
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
4. **Zero-Warning & Zero-Override Policy (`#[allow(...)]` Forbidden)**:
   - Do NOT use `#[allow(dead_code)]`, `#[allow(unused)]`, or any other compiler/clippy warning suppressions.
   - Every function, struct, and variant must either be genuinely wired into calling paths, stubbed with a real consumer, or placed behind `#[cfg(test)]` if test-only.
   - All code must compile cleanly and strictly on its own merits without suppressions.
5. **Release Profile Configuration**:
   - Keep `opt-level = 3`, `lto = "fat"`, `codegen-units = 1`, `panic = "abort"`, and `strip = true` in `[profile.release]`.

## Scroll Mode & Copy Behavior (Tmux-Style)

The application provides two operational modes:
1. **Input Mode (`Mode::Input`)**:
   - Default mode. Active hardware cursor sits in the input prompt.
   - Keystrokes type commands; `Enter` runs them.
   - **Keybindings & Actions**:
     - `Ctrl+D`: Exit application (when prompt is empty) / delete char forward.
     - `Ctrl+C`: Clear user input prompt immediately.
     - `Esc`: Cancel active running command (or enter scroll mode if prompt is empty).
     - `Ctrl+L`: Clear feed history (fresh canvas).
     - `Ctrl+S` / `PageUp`: Enter Scroll Mode.
   - **Command History (Bash-Style Persistence)**:
     - Automatically saves executed commands to `.piro/history` across sessions (checks local `./.piro/` then defaults to `~/.piro/history`).
     - `Up` / `Ctrl+P`: cycles backward through previous commands.
     - `Down` / `Ctrl+N`: cycles forward, restoring any in-progress draft.
     - Automatically deduplicates consecutive identical commands.
   - **Unix Readline Editing**:
     - `Ctrl+W` / `Alt+Backspace`: bash `unix-word-rubout` (deletes backward across word to whitespace).
     - `Ctrl+U`: kill line to start.
     - `Ctrl+K`: kill line to end.
     - `Ctrl+A` / `Home`: jump to start of line.
     - `Ctrl+E` / `End`: jump to end of line.
     - `Alt+B` / `Alt+F`: jump word backward / forward.
   - Footer displays ` READY ` (or ` RUNNING `).
2. **Scroll Mode (`Mode::Scroll`)**:
   - Navigation & Cursor in feed:
     - `k` / `Up`: move cursor up 1 line
     - `j` / `Down`: move cursor down 1 line
     - `PageUp`: jump full page up (`viewport_height` lines)
     - `PageDown`: jump full page down (`viewport_height` lines)
     - `Ctrl+U`: jump half page up (`viewport_height / 2` lines)
     - `Ctrl+D`: jump half page down (`viewport_height / 2` lines)
     - `g` / `Home`: jump to top of feed
     - `G` / `End`: jump to bottom of feed
   - **Exit & Yank Behavior**:
     - `q`: **Drop out only** (returns to input prompt without clearing selection or moving cursor).
     - `y`: **Yank and keep cursor where it is** (copies selection to clipboard, flashes `[Copied!]`, clears highlight, and remains in Scroll Mode).
     - `Enter`: **Yank and return to input** (copies selection to clipboard, flashes `[Copied!]`, clears highlight, and returns to input prompt).
     - `Esc`: Cancel active selection (or exit Scroll Mode if no selection).
   - **Unified Selection Model (Maki-Style)**:
     - **Mouse**:
       - **Scroll Up**: Enters Scroll Mode and scrolls up into history.
       - **Scroll Down**: Smoothly moves the feed and cursor down towards the bottom. Only scrolling *past* the bottom-most line exits Scroll Mode back to the input prompt.
       - **Left-Click & Drag**: Highlights words, spans, or multi-line blocks with exact character precision. **Releasing the mouse immediately copies the text and clears the highlight** (`selection_state = None`, matching maki).
     - **Keyboard Selection**:
       - Move with `h`/`j`/`k`/`l` or arrow keys with the visible hardware cursor positioned directly in the feed.
       - **Character Selection (`v` / `Space`)**: Anchors character/span selection; moving extends the highlighted span.
       - **Full-Line Selection (`V` / `Shift+V`)**: Anchors full-line selection; moving extends the selection across full lines.
       - **Yank / Copy (`y` / `Enter`)**: Copies selected text to the system clipboard (OSC 52 and macOS `pbcopy`), immediately clears the highlight, and returns to the input prompt.
       - **Cancel (`Esc`)**: Clears the selection (or exits scroll mode if not selecting).
     - **One-Key Block Copy (`b`)**: Pressing `b` in Scroll Mode instantly copies the entire output of the current command block.
     - **Expand / Collapse (`o` / `Ctrl+O`)**: Toggles between the compact 8-line live tail and the fully expanded command output block.
   - Footer displays:
     - Default: ` SCROLL   [hjkl] Move  [v] Char sel  [V] Line sel  [b] Copy block  [i/q/Esc] Return `
     - When Selecting (Char): ` VISUAL   [hjkl/arrows] Extend  [y/Enter] Copy  [v/Space/Esc] Cancel `
     - When Selecting (Line): ` VISUAL LINE   [jk/arrows] Extend lines  [y/Enter] Copy  [v/V/Esc] Cancel `
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
   - Block edges (`┌`, `│`, `└` and `┐`, `│`, `┘`) extend **all the way to the screen edges** (columns 0 and `width - 1`).
   - Content has **1 space of padding inside the block borders** (`│ ` on left, ` │` on right).
   - **Live Tailing (pi.dev-style)**: By default, blocks display a compact live tail of the last 8 lines with `... N lines hidden (Ctrl+O to expand) ...`. Pressing `Ctrl+O` (or `o` in Scroll Mode) expands to the full output.
   - Long output lines and commands are **word-wrapped at word boundaries** on screen, preserving full ANSI colors.
   - **Soft-Wrap Unwrapping**: When copying text, soft-wrapped continuation rows are automatically rejoined with space instead of newline, guaranteeing that long lines copy cleanly without artificial line breaks.
   - `$ <command>` is placed on the first row inside the block in bold white.
   - A blank spacer row follows the command line.
   - Output lines preserve terminal ANSI color codes parsed via `ansi-to-tui` into colored Ratatui Spans without border misalignment.
   - If a command produces no stdout/stderr, `(no output)` is rendered in dimmed text (`Color::DarkGray` + `Modifier::DIM`).
   - If a command exits with a non-zero code, `Command exited with code <N>` is displayed in bold red followed by a blank spacer row.
   - If a command is cancelled via `Ctrl+C`, `Command cancelled` is displayed in bold yellow followed by a blank spacer row.
   - The final line displays `Took X.Xs` (or `Running X.Xs` while active) in dimmed text with 1-decimal-place precision (`{:.1}s`).
5. **Border Alignment & Padding**:
   - All block lines are formatted using exact `inner_w = width - 4` math so left border `│` sits at column 0 and right border `│` sits at column `width - 1`.

## Development Workflows & Quality Gates

Every change must pass all four quality gates with zero warnings:

```bash
# 1. Format check
cargo fmt --check

# 2. Strict Clippy check (zero warnings permitted, zero #[allow(...)] overrides)
cargo clippy -- -D warnings

# 3. Unit and integration tests
cargo test

# 4. Code & binary bloat verification
tokei -e target
cargo bloat --release --crates
```
