# Terminal Command Runner TUI Implementation Plan

## Research

- **Ratatui & Crossterm Event Loop**: Ratatui 0.30+ works seamlessly with `crossterm` 0.29. A standard pattern in async Ratatui applications is multiplexing crossterm events (using `crossterm::event::EventStream` or a dedicated background task reading `event::read()`) with internal application events (stdout lines, process exit notifications, tick timers) over a `tokio::sync::mpsc::channel`.
- **Async Process Execution in Tokio**: `tokio::process::Command` supports piping `stdout` and `stderr`. By wrapping both in `tokio::io::BufReader` and reading lines asynchronously (or merging them with `tokio_stream::StreamExt`), lines can be pushed directly to the event channel without blocking the UI rendering loop.
- **Process Cancellation & Group Signaling**: When a command like `sleep 10` is executed via `$SHELL -c`, killing the parent shell doesn't always terminate child processes unless process group termination is handled (e.g. `process.kill()` or setting process group). Tokio's `child.kill()` will terminate the spawned shell.
- **Panic Hook & Terminal Restoration**: Crossterm raw mode and alternate screen must be restored on unexpected panic. Installing a custom panic hook via `std::panic::set_hook` ensures the terminal cursor is restored and alternate screen exited before the panic message is printed.

## Reuse

- **Crossterm**: `crossterm::terminal::{enter_alternate_screen, leave_alternate_screen, enable_raw_mode, disable_raw_mode}`, `crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent}`.
- **Ratatui Widgets & Layout**:
  - `Layout::default().direction(Direction::Vertical).constraints(...)` for the split layout.
  - `Block`, `Borders`, `BorderType::Rounded` for styled cards.
  - `Paragraph`, `Text`, `Line`, `Span`, `Style`, `Color`, `Modifier` for formatted feed items and status badges.
- **Tokio**: `tokio::sync::mpsc` for message passing, `tokio::process::Command` for shell execution, `tokio::io::AsyncBufReadExt` for line-by-line stream reading.

## Invariants and security boundaries

- **Terminal Recovery Invariant**: Under all exit conditions (clean exit, user cancellation, or unexpected Rust panic), the terminal must restore raw mode, alternate screen, and cursor visibility.
- **UI Non-blocking Invariant**: UI rendering and input handling must never block on subprocess I/O. Commands run on asynchronous background tasks; output lines are transferred via buffered channels.
- **Domain vs Transport Separation**:
  - The domain model (`Feed`, `FeedItem`, `CommandStatus`, `ScrollState`, `InputState`) remains pure Rust data structures with no direct terminal drawing code.
  - The TUI layer (`ui.rs`) acts as a pure renderer transforming state into Ratatui widgets.
  - The runner (`runner.rs`) acts as an asynchronous adapter handling OS process lifecycle.

## Quality gates

- **Format**: `cargo fmt --check`
- **Lint**: `cargo clippy -- -D warnings`
- **Type-check and tests**: `cargo test`
- **Clean builds**: Zero compilation warnings on `cargo build`.

## Definition of done

- `cargo clippy` and `cargo test` pass with 0 errors/warnings.
- `cargo run` displays the 4-region layout (Feed, Separator, Input box, Footer).
- Typing in the input box updates text and cursor position with full backspace, delete, and left/right arrow support.
- Submitting a command appends a `UserInputBlock` and an active `CommandOutputBlock`.
- Output streams in real-time into the command block as lines arrive.
- When finished, the command block displays execution duration and exit code badge.
- `Ctrl+C` cleanly aborts an active command without quitting the app; `Esc` or `Ctrl+C` when idle exits cleanly.
- Feed supports scrolling (auto-scroll down when at bottom, manual scroll up with PgUp / mouse wheel).

## Assumptions

- Target OS is Unix/macOS/Linux with standard `$SHELL` (fallback to `sh -c`).
- Single active command execution at a time for v1.
- ANSI escape codes are stripped or sanitized to prevent terminal corruption.
- In-memory feed storage without SQLite or file-based history for v1.

## Risks (with mitigation)

- **Risk**: Process hangs or generates rapid unbounded output flooding memory.
  - **Mitigation**: Buffer output lines up to a reasonable cap (e.g. 5,000 lines per command) and drop oldest lines if cap is exceeded.
- **Risk**: Terminal left frozen if panic occurs.
  - **Mitigation**: Install a robust panic hook before entering raw mode that disables raw mode and restores terminal buffers.
- **Risk**: Child process ignores termination signal on Ctrl+C.
  - **Mitigation**: Use `child.start_kill()` on Tokio `Child` handle to send SIGKILL/SIGTERM.

## Dependencies

- `ratatui` (v0.30): TUI rendering library (MIT license).
- `crossterm` (v0.29): Terminal manipulation, alternate screen, event loop (MIT license).
- `tokio` (v1.53, features `["full"]` or `["rt-multi-thread", "macros", "process", "io-util", "sync", "time"]`): Async runtime and process runner (MIT license).
- `strip-ansi-escapes` (v0.2): Stripping raw ANSI sequences to prevent broken TUI line rendering (Apache-2.0 / MIT).

## Decisions

- **Decision: Alternate screen (fullscreen) vs Inline viewport**: Chosen Fullscreen alternate screen. Offers smooth virtual scrolling, deterministic layout heights, and sticky input/footer placement matching the pi.dev look and feel.
- **Decision: Shell execution (`$SHELL -c`) vs Direct exec**: Chosen `$SHELL -c` with fallback to `/bin/sh -c`. Ensures user environment variables, aliases, pipes, and shell expansions work naturally.
- **Decision: Centralized Event Multiplexing**: Crossterm events and background process events are merged into a single `tokio::sync::mpsc::channel<AppEvent>`. Keeps application state updates single-threaded and eliminates locking or concurrency bugs.

## Out of scope

- Full PTY emulation for interactive ncurses tools (`vim`, `htop`).
- Multi-command simultaneous execution / job manager tabs.
- Persistent database storage of command transcripts.
- AI LLM streaming / tool call integration (reserved for subsequent milestones).

---

## Vertical Slices

### Slice 1: Project Scaffolding, Terminal Lifecycle, and Layout Shell

- **Goal**: Initialize the Rust project with dependencies, set up safe terminal initialization/panic hooks, and render the static 4-region layout (Feed, Separator, Input, Footer).
- **Acceptance criteria**:
  - Project builds and runs cleanly via `cargo run`.
  - Terminal enters alternate screen in raw mode on start and reliably restores on exit (`q` or `Esc`) or panic.
  - UI draws the split layout with placeholder borders and footer hints.

#### Task 1.1: Initialize Cargo project and dependencies [1]
**Do:** Create `Cargo.toml` with `ratatui`, `crossterm`, `tokio`, and `strip-ansi-escapes`. Set up project directory structure (`src/main.rs`, `src/app.rs`, `src/ui.rs`, `src/event.rs`, `src/runner.rs`).
**Verify:** `cargo check` -- completes successfully with all dependencies downloaded and resolved.

#### Task 1.2: Implement Terminal Guard and Panic Hook [2]
**Do:** In `src/main.rs`, implement `TerminalGuard` RAII struct or setup/cleanup functions that enable raw mode, enter alternate screen, install a custom panic hook (that disables raw mode and leaves alternate screen before calling original hook), and clean up on drop.
**Verify:** `cargo run` -- terminal enters alternate screen; pressing a quit key exits cleanly and restores bash/zsh prompt without corruption.

#### Task 1.3: Implement Layout Rendering in `ui.rs` [2]
**Do:** In `src/ui.rs`, construct the 4 vertical areas using `Layout::vertical`:
  1. Feed area (`Constraint::Min(3)`)
  2. Separator (`Constraint::Length(1)`)
  3. Input box (`Constraint::Length(3)`)
  4. Footer (`Constraint::Length(1)`)
Draw borders and labels for each section.
**Verify:** `cargo run` -- displays the 4 distinct UI areas clearly delineated on the screen.

**Slice 1 Verification**: Run `cargo clippy && cargo run` -- application launches in fullscreen alternate screen, renders the 4 sections, and exits cleanly upon pressing `Esc`.

---

### Slice 2: Interactive Input Field & Cursor Handling

- **Goal**: Implement an editable input prompt with real-time character typing, backspace, delete, cursor positioning, and Enter submission.
- **Acceptance criteria**:
  - Typing inserts characters at the current cursor position.
  - `Backspace` deletes preceding character; `Delete` deletes character at cursor.
  - `Left` and `Right` arrow keys move cursor without moving off bounds; `Home` and `End` jump to ends.
  - Terminal cursor is visually positioned in the input box at the exact character index.
  - Pressing `Enter` clears the input buffer and emits a submitted command event.

#### Task 2.1: Implement `InputState` domain model [2]
**Do:** In `src/app.rs`, implement `InputState` with methods: `insert(char)`, `delete_back()`, `delete_forward()`, `move_left()`, `move_right()`, `move_start()`, `move_end()`, `take_value() -> String`.
**Tests:** Unit tests in `src/app.rs` covering typing, backspacing in middle, delete at boundaries, and cursor indexing with UTF-8 characters.
**Verify:** `cargo test` -- all input state unit tests pass.

#### Task 2.2: Wire Input Events and Cursor Drawing in TUI [2]
**Do:** In `src/event.rs` and `src/ui.rs`, handle keyboard events (`Char`, `Backspace`, `Delete`, `Left`, `Right`, `Home`, `End`). In `ui.rs`, render the input text inside the input box and use `frame.set_cursor_position(Position::new(x, y))` to display the terminal cursor.
**Verify:** `cargo run` -- typing characters displays them immediately; arrow keys navigate cursor accurately; cursor matches text position.

**Slice 2 Verification**: Run `cargo test && cargo run` -- test full editing capabilities inside the input box.

---

### Slice 3: Feed Data Model & Block Rendering

- **Goal**: Implement the feed data structures and render distinct styled cards for User Input and Command Output.
- **Acceptance criteria**:
  - Submitted input creates a `UserInputBlock` rendered as a distinct rounded card with a prompt icon and timestamp.
  - A corresponding `CommandOutputBlock` is created showing command status header (`Running`, `Exit 0`, etc.) and log lines.
  - Feed items reflow cleanly when terminal is resized.

#### Task 3.1: Define Feed Data Models in `src/app.rs` [2]
**Do:** Define `FeedItem` enum:
  - `UserInput { id: usize, command: String, timestamp: String }`
  - `CommandOutput { id: usize, command: String, status: CommandStatus, lines: Vec<String>, started_at: Instant, finished_at: Option<Instant> }`
  - `CommandStatus { Running, Success(i32), Failed(i32), Cancelled }`
Add helper methods to append and retrieve items.
**Tests:** Unit tests in `src/app.rs` verifying feed additions, status transitions, and line appending.
**Verify:** `cargo test` -- feed data model tests pass.

#### Task 3.2: Implement Block Renderers in `src/ui.rs` [3]
**Do:** In `src/ui.rs`, implement rendering for `FeedItem`:
  - `render_user_input_block`: Distinct bordered block (cyan/blue border) with `> <command>` and timestamp.
  - `render_command_output_block`: Bordered block with status pill (yellow `[Running]`, green `[Exit 0]`, red `[Exit N]`, gray `[Cancelled]`), execution duration, and lines of output.
**Verify:** `cargo run` -- with mock feed items, feed renders stacked blocks with distinct visual borders and status badges.

**Slice 3 Verification**: Run `cargo test && cargo run` -- verified feed visual presentation with styled user input cards and command cards.

---

### Slice 4: Async Command Execution & Live Streaming

- **Goal**: Execute commands entered by the user asynchronously via `$SHELL -c`, stream output lines in real time, record exit status, and handle cancellation.
- **Acceptance criteria**:
  - Submitting a command (e.g. `ls -la`) spawns a background process and streams stdout/stderr lines into the active block.
  - The UI remains completely responsive (smooth input, no stuttering) while command is executing.
  - When the process completes, duration and exit code are updated.
  - Pressing `Ctrl+C` terminates the running process and marks the block as `[Cancelled]`.
  - Empty or whitespace inputs are ignored.

#### Task 4.1: Implement Async Command Runner in `src/runner.rs` [3]
**Do:** Implement `CommandRunner` which spawns `$SHELL -c <cmd>` using `tokio::process::Command` with `stdout(Stdio::piped())` and `stderr(Stdio::piped())`. Read lines asynchronously with `BufReader` and send `RunnerEvent::Line(String)`, `RunnerEvent::Done(ExitStatus)` to an mpsc channel. Strip ANSI escape sequences using `strip-ansi-escapes`. Support cancellation handle via `tokio::sync::oneshot` or `child.kill()`.
**Tests:** Integration tests verifying command runner emits lines and exits with correct status for commands like `echo hello` and `sh -c 'exit 42'`.
**Verify:** `cargo test runner` -- tests pass.

#### Task 4.2: Connect Runner Events to App Event Loop [3]
**Do:** In `src/main.rs` and `src/app.rs`:
  - When Enter is pressed with text, create a `UserInput` and `CommandOutput` block, then trigger runner.
  - Handle `RunnerEvent::Line`: append line to current active `CommandOutput` block.
  - Handle `RunnerEvent::Done`: update status with exit code and completion time.
  - Handle `Ctrl+C`: if running, abort process; if idle, exit application.
**Verify:** `cargo run` -- run `for i in 1 2 3; do echo "line $i"; sleep 0.5; done`; watch lines appear live in the block, ending in `[Exit 0]`. Press `Ctrl+C` during `sleep 10` and verify it cancels immediately.

**Slice 4 Verification**: Run `cargo test && cargo run` -- verified real-time line streaming, status updates, and `Ctrl+C` process cancellation.

---

### Slice 5: Scrollback, Auto-Scroll Lock, and Mouse Wheel Navigation

- **Goal**: Enable vertical scrolling in the feed with sticky auto-scroll locking.
- **Acceptance criteria**:
  - When at the bottom, incoming log lines auto-scroll down to keep the latest lines visible.
  - Pressing `PageUp` or scrolling mouse wheel up scrolls up into history and pauses auto-scroll.
  - Pressing `PageDown` or scrolling mouse wheel down scrolls towards the bottom; reaching bottom re-enables auto-scroll.
  - A scroll indicator or scrollbar reflects current scroll position.

#### Task 5.1: Implement `ScrollState` and Viewport Calculation [2]
**Do:** In `src/app.rs`, implement `ScrollState`:
  - `scroll_offset: usize`
  - `auto_scroll: bool`
  - Methods: `scroll_up(delta)`, `scroll_down(delta)`, `scroll_to_bottom()`, `on_new_content()`.
**Tests:** Unit tests in `src/app.rs` testing auto-scroll state transitions when scrolling up and back down.
**Verify:** `cargo test scroll` -- unit tests pass.

#### Task 5.2: Wire Scroll Keys, Mouse Wheel, and Ratatui Rendering [2]
**Do:** In `src/event.rs`, handle `KeyCode::PageUp`, `KeyCode::PageDown`, `MouseEventKind::ScrollUp`, `MouseEventKind::ScrollDown`. In `src/ui.rs`, slice the feed items / rendered lines according to `scroll_offset` and viewport height. Render scrollbar widget on the right edge of the feed area.
**Verify:** `cargo run` -- run a command producing 100 lines (e.g., `seq 1 100`). Viewport stays pinned to the bottom. Scroll up with mouse or PageUp to inspect earlier numbers; scroll down to bottom to re-lock.

**Slice 5 Verification**: Run `cargo test && cargo run` -- verified scrolling up and down, auto-scroll locking, and scrollbar indicator.

---

## Final Verification

1. **Static Analysis**:
   ```bash
   cargo fmt --check
   cargo clippy -- -D warnings
   cargo test
   ```
2. **Interactive Run Validation**:
   - Launch application: `cargo run`
   - Test basic command: `echo "Hello Ratatui"` -> user block + output block + `[Exit 0]`.
   - Test streaming: `python3 -c "import time; [print(f'tick {i}', flush=True) or time.sleep(0.3) for i in range(5)]"` -> live lines appear one by one.
   - Test cancellation: `sleep 10` -> press `Ctrl+C` -> command terminates immediately with `[Cancelled]`.
   - Test error exit: `false` or `exit 1` -> command exits with `[Exit 1]`.
   - Test pipe / shell syntax: `ls | grep Cargo | wc -l` -> correct piped output.
   - Test scrollback: `seq 1 200` -> scroll up with PageUp/mouse wheel, check line 1, scroll down to 200, verify auto-scroll re-engages.
   - Test clean exit: press `Esc` -> terminal prompt restored cleanly with no artifacts or broken cursor.
