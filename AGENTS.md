# AGENTS.md

## Project Overview

`piro` is a high-performance terminal command runner and ReAct agent TUI in Rust using Ratatui and Crossterm:
- Scrollable, bottom-anchored output feed with collapsible pi.dev-style live tailing.
- Horizontal-ruled command prompt: clean lines, no side borders, no prompt glyph.
- Minimal status bar with mode badges (`READY`, `RUNNING`, `THINKING`, `SCROLL`, `VISUAL`).

## Architecture & Codebase Map

```
src/
├── main.rs       # Entry point, tokio::select! loop, and terminal event multiplexing
├── events.rs     # Key & mouse dispatch (InputCtx, ScrollCtx, NavBounds)
├── app.rs        # Domain state (App, FeedItem, CommandOutputBlock, ScrollState)
├── runner.rs     # Async subprocess runner ($SHELL -c), process groups, SIGKILL
├── terminal.rs   # Raw mode, alternate screen, mouse capture, panic recovery
├── clipboard.rs  # OSC 52 and platform clipboard integrations
├── agent/
│   ├── mod.rs    # ReAct loop supervisor, AgentEvent stream, AgentHandle
│   ├── tool.rs   # Tool trait, BashTool, ToolRegistry
│   └── backend.rs# AgentBackend trait, AgentMessage, MockAgentBackend
└── ui/
    ├── mod.rs    # Master layout (Feed, Separator, Input, Footer)
    ├── feed.rs   # Virtual scrolling, selection slicing, text extraction
    ├── blocks.rs # Titled cards: UserInput, CommandOutput (tools), Thought, Agent
    ├── input.rs  # Input prompt bordered by top/bottom divider rules
    ├── footer.rs # Status badges and active scrollback notifications
    └── text.rs   # ANSI-aware word wrapping and display width calculation
```

## Engineering Rigor & Code Invariants

1. **Function Size & Struct Convention**:
   - **Max 3 arguments per function** (`too-many-arguments-threshold = 3`).
   - **When 3 or more arguments are needed**, the 3rd argument **MUST be a named domain struct** bundling the contextual arguments (e.g. `InputCtx`, `ScrollCtx`, `CardLayout`, `AgentContext`).
   - **Never pass ad-hoc tuples** across function boundaries (e.g. use `ScrollBounds { total, view }` instead of `(usize, usize)`).
   - **Max 40 lines per function** (`too-many-lines-threshold = 40`). Extract single-responsibility helpers.
   - **Cognitive complexity $\le 15$**.
2. **Zero-Warning & Zero-Override Policy (`#[allow(...)]` Forbidden)**:
   - Never suppress compiler or Clippy warnings (`#[allow(dead_code)]`, `#[allow(unused)]`, etc.).
   - Wire all code into execution paths, stub consumers, or place behind `#[cfg(test)]` if test-only.
3. **Binary & Bloat Budget**:
   - Release binary MUST remain **under 1.0 MiB** (target: ~830 KiB) with `opt-level = 3`, `lto = "fat"`, `panic = "abort"`, and `strip = true`.
   - Maintain minimal dependency set: no heavy umbrella crates, no `chrono`.
4. **File Length**:
   - Keep files modular (< 300 lines per file where possible).

## Prompt Routing & Operational Modes

### 1. Input Mode (`Mode::Input`)
- **Routing**:
  - `!!<cmd>`: Direct shell execution (bypasses agent).
  - `!<cmd>`: Runs shell command and feeds output to the ReAct agent context.
  - `<prompt>` (unprefixed): Automatically triggers autonomous ReAct agent loop.
- **Editing & Shortcuts**:
  - Readline: `Ctrl+W` / `Alt+Backspace` (word rubout), `Ctrl+A` / `Ctrl+E` (home/end), `Alt+B` / `Alt+F` (skip words), `Ctrl+U` / `Ctrl+K` (kill line).
  - History: `Up` / `Down` or `Ctrl+P` / `Ctrl+N` (persisted in `~/.piro/history`).
  - Navigation: `PageUp` / `Ctrl+S` (enter Scroll Mode), `Ctrl+O` (toggle live tail expand).
  - Cancellation: `Esc` / `Ctrl+C` immediately cancels in-flight processes and agent turns.

### 2. Scroll Mode (`Mode::Scroll`)
- **Navigation**: `h`/`j`/`k`/`l`, arrows, `g`/`G` (top/bottom), `Ctrl+U`/`Ctrl+D` (half page), `PageUp`/`PageDown`.
- **Selection**: `v` / `Space` (character visual), `V` (line visual), mouse drag.
- **Copy**: `y` (yank in place), `Enter` (yank and return to input), mouse release (instant copy & return).
- **Block Copy**: `b` copies active card's full output. `Esc` / `q` exits.

## UI & Layout Invariants

- **Input Prompt**: Top and bottom divider rules `─...─` in `Color::DarkGray` only. No side borders, corners, or prompt symbols.
- **Separator**: Exactly 1 empty row between feed and prompt top rule.
- **Card Styling**:
  - Width: Exactly `inner_w = width - 4` (`│ ` on left, ` │` on right).
  - Borders: Cyan for user input/agent response, Magenta for thoughts, Yellow/Green/Red/DarkGray for running/success/failed/cancelled commands.
  - Live Tailing: Compact 8-line tail with `... N lines hidden (Ctrl+O to expand) ...`.
  - Unwrapped Copy: Rejoin soft-wrapped continuation rows with spaces, not artificial newlines.

## Quality Gates

Every change must pass all gates with zero warnings:

```bash
cargo fmt --check
cargo clippy -- -D warnings
cargo test
cargo build --release
```
