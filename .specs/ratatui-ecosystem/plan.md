# Ratatui Ecosystem Integration Implementation Plan

## Research

- **`ansi-to-tui`**: Provides `IntoText` extension trait on `[u8]` and `&str`. Converts ANSI sequences into `ratatui::text::Text` composed of `Line` and `Span` with accurate `Color` and `Modifier`. Does not require heavy dependencies.
- **`tui-input`**: The de-facto Ratatui input widget. Implements `Input::handle(&mut self, request)` and integrates with `crossterm::event::KeyEvent`. Provides complete Readline word navigation, deletions, and cursor handling.
- **Binary Footprint Budget**: Current release binary is 979.9 KiB. Adding `ansi-to-tui` and `tui-input` is projected to add less than 20 KiB, keeping total binary size well within the 1.0 MiB ceiling.

## Reuse

- `ansi_to_tui::IntoText` to parse raw subprocess output lines.
- `tui_input::Input` for input buffer management and Readline key dispatch.
- Existing `box_border` and `box_line` helpers in `src/ui/blocks.rs`.
- Existing `clipboard::copy_to_clipboard` for OSC 52 + `pbcopy`.

## Invariants and security boundaries

- **Memory & Size Invariants**: Codebase size remains under 900 lines of Rust (`tokei -e target` < 800 lines of code). Release binary remains under 1.0 MiB (`cargo bloat --release --crates`).
- **No Stdin Races**: No hardware cursor query calls (`\x1b[6n`) while `EventStream` is active. Fullscreen viewport is preserved.
- **Style Isolation**: All ANSI styles are scoped to individual spans and do not affect card borders or input divider rules.

## Quality gates

- `cargo fmt --check`
- `cargo clippy -- -D warnings`
- `cargo test`
- `cargo bloat --release --crates` (< 1.0 MiB)
- `tokei -e target`

## Definition of done

- Subprocesses emitting ANSI colors display rich colors in command output cards.
- `Ctrl+W` deletes previous word in the input prompt.
- `Up` and `Down` arrows in Input Mode cycle through command history.
- All quality gates pass with zero warnings.
- Release binary is under 1.0 MiB.

---

## Vertical Slices

### Slice 1: Rich ANSI Output Rendering (`ansi-to-tui`)

- **Goal**: Preserve and parse ANSI escape sequences from command output into colored Ratatui Spans inside command blocks.
- **Acceptance criteria**:
  - `src/runner.rs` streams raw output lines without stripping ANSI sequences.
  - `src/ui/blocks.rs` converts lines via `ansi-to-tui` into styled `Line`s.
  - Right borders `│` remain properly aligned with exact padding.

#### Task 1.1: Add `ansi-to-tui` and preserve ANSI in runner [2]
**Do:** Add `ansi-to-tui` to `Cargo.toml`. In `src/runner.rs`, stop stripping ANSI codes before sending over `RunnerMsg::Line`.
**Verify:** `cargo check` -- builds cleanly.

#### Task 1.2: Render parsed ANSI Spans in `blocks.rs` [3]
**Do:** In `src/ui/blocks.rs`, parse output lines with `ansi_to_tui::IntoText`. Calculate visual width across spans using `unicode-width`, truncate or pad to `inner_w`, and render inside `box_line`.
**Tests:** Unit test in `src/ui/blocks.rs` verifying ANSI styled line formatting and width alignment.
**Verify:** `cargo test` -- tests pass.

---

### Slice 2: Advanced Input Editing (`tui-input`)

- **Goal**: Replace custom character insertion/deletion in `InputState` with `tui-input`.
- **Acceptance criteria**:
  - Standard typing, backspace, delete continue to work.
  - `Ctrl+W` deletes previous word.
  - `Ctrl+A` / `Ctrl+E` jump to start/end of line.
  - `Ctrl+U` / `Ctrl+K` kill text before/after cursor.
  - Pinned input row between horizontal divider rules is preserved.

#### Task 2.1: Add `tui-input` and integrate into `InputState` [2]
**Do:** Add `tui-input` to `Cargo.toml`. Refactor `src/app.rs` `InputState` to wrap `tui_input::Input`. Expose `value()`, `cursor()`, and `handle_key(&mut self, key: KeyEvent)`.
**Tests:** Unit tests in `src/app.rs` testing `Ctrl+W` word deletion and cursor bounds.
**Verify:** `cargo test` -- all input tests pass.

#### Task 2.2: Wire Readline shortcuts in `src/main.rs` [2]
**Do:** In `src/main.rs`, delegate input editing keys to `app.input.handle_key(key)`.
**Verify:** `cargo run` -- test `Ctrl+W`, `Ctrl+A`, `Ctrl+E`.

---

### Slice 3: Command History Recall (`Up`/`Down`)

- **Goal**: Implement command history navigation with `Up` and `Down` arrows in Input Mode.
- **Acceptance criteria**:
  - Pressing `Enter` adds the command to history (ignoring empty or duplicate consecutive entries).
  - Pressing `Up` cycles backward through history.
  - Pressing `Down` cycles forward, restoring the in-progress draft at the bottom.

#### Task 3.1: Implement History Ring Buffer in `src/app.rs` [2]
**Do:** Add `history: Vec<String>`, `history_idx: Option<usize>`, and `draft_input: String` to `App`. Implement `history_prev()` and `history_next()` methods.
**Tests:** Unit tests in `src/app.rs` verifying history cycling, draft preservation, and duplicate deduplication.
**Verify:** `cargo test` -- tests pass.

#### Task 3.2: Wire `Up`/`Down` in `handle_input_mode_key` [1]
**Do:** In `src/main.rs`, route `KeyCode::Up` to `app.history_prev()` and `KeyCode::Down` to `app.history_next()`.
**Verify:** `cargo run` -- execute commands and recall with `Up`/`Down`.

---

### Slice 4: Bloat & Quality Verification

- **Goal**: Verify binary size, formatting, clippy, and update documentation.
- **Acceptance criteria**:
  - `cargo fmt --check` clean.
  - `cargo clippy -- -D warnings` clean.
  - `cargo test` all pass.
  - `cargo bloat --release --crates` confirms binary size < 1.0 MiB.
  - `AGENTS.md` updated with new features and keybindings.

#### Task 4.1: Quality gates and bloat verification [2]
**Do:** Run all 4 quality gates, update `AGENTS.md`, and make a clean git commit.
**Verify:** `cargo bloat --release --crates` -- binary size is under 1.0 MiB.
