# Terminal Command Runner TUI Spec

## Status

Draft

## Problem

Developers interacting with command-line tools frequently need to monitor live output streams while maintaining a clean, structured transcript of inputs and command executions. Traditional terminals interleave user inputs, raw stdout/stderr, and prompt characters in a single flat stream without clear semantic boundaries. Modern agent interfaces (such as pi.dev) provide a structured feed of distinct visual blocks (user input block, live-streaming command execution block, status indicators) paired with a persistent, non-intrusive input prompt. Building this foundation in Rust with Ratatui provides a fast, resilient, and extensible core that can later be expanded into a full agentic TUI.

## Users and stakeholders

- **Developer / CLI User**: Runs shell commands and scripts, inspects real-time live tail output, scrolls through execution history, and manages running processes within a unified TUI.
- **Future Agent Integrator**: Relies on a clean, block-oriented feed architecture to plug in AI agent turns, tool calls, and streaming responses.

## Goals

- Provide a split TUI layout in Ratatui consisting of:
  1. A scrollable **Output Feed Area** displaying distinct semantic blocks for user inputs and command outputs.
  2. A clean visual **Separator** delineating feed from input.
  3. A dedicated **User Input Area** with real-time text entry and cursor editing.
  4. A **Footer Area** displaying keyboard shortcuts and runtime status.
- Execute shell commands asynchronously without freezing the UI or dropping frame rates.
- Stream command stdout and stderr in real-time into the active command block.
- Support smooth scrolling (auto-scroll on new output when pinned to the bottom, manual scrollback with keyboard and mouse).
- Provide visual differentiation between user input prompts, running commands, and finished commands (with exit status).
- Lay an extensible message-block foundation for future pi.dev agent capabilities.

## Non-goals

- Full pseudo-terminal (PTY) emulation with interactive curses/ncurses applications (e.g. running `vim` or `top` inside the command runner is out of scope for v1; v1 targets line-based CLI commands like `cargo build`, `ls`, `curl`, `git status`).
- Remote execution, SSH tunneling, or multi-tab session management.
- Multi-agent or LLM API integrations (these will be added in subsequent milestones on top of this block feed).
- Persistent disk storage / database persistence for command history (in-memory per session for v1).

## Current behavior

The repository is currently an empty directory with no existing Rust project or TUI implementation.

## Desired behavior

When the user launches the binary:
1. The application enters the terminal alternate screen with raw mode enabled.
2. The UI renders the three primary sections:
   - Top: Feed area (initially empty or showing a welcoming prompt/hints).
   - Separator line with empty margin.
   - Bottom input box: `> ` prompt with an active cursor.
   - Bottom-most footer: `[Enter] Run  [PgUp/PgDn] Scroll  [Ctrl+C] Cancel/Quit`.
3. When the user types a command (e.g., `cargo test`) and presses `Enter`:
   - The input box clears.
   - A **User Input Block** is appended to the feed displaying `> cargo test` and a timestamp.
   - A **Command Output Block** is immediately appended below it with a `[Running]` status indicator.
   - The command executes asynchronously via the user's `$SHELL -c` (fallback to `sh -c`).
   - Stdout and stderr stream line-by-line into the command block in real time.
   - When the process completes, the block header updates with the exit code (e.g., `[Exit 0]` in green or `[Exit 1]` in red) and duration.
4. If output exceeds the viewport height:
   - The viewport automatically scrolls down to follow live output if already at the bottom.
   - If the user scrolls up (via `PageUp` or Mouse Wheel), auto-scroll pauses to allow reading past output.
   - Scrolling back to the bottom re-engages sticky auto-scroll.
5. If a command is running and the user presses `Ctrl+C`:
   - A cancellation signal is sent to terminate the child process group.
   - The block marks the command as `[Cancelled]`.
   - The application remains running and returns to the ready state.
6. Pressing `Esc` or `Ctrl+C` (when no command is running) cleanly exits the TUI and restores terminal alternate screen and cursor state.

## Requirements

- **REQ-001**: The application MUST initialize a full-screen Ratatui terminal using `crossterm` in alternate screen and raw mode, and MUST restore terminal state on shutdown or panic.
- **REQ-002**: The layout MUST divide the screen vertically into:
  - Output Feed (flexible height, min 1 line)
  - Separator line (1 line bordered or horizontal rule)
  - Input Box (fixed height: 3 lines for border + 1 input line)
  - Footer (fixed height: 1 line)
- **REQ-003**: The feed MUST support rendering distinct block types:
  - `UserInputBlock`: Displays the submitted command line, styled with a distinct border, label, or accent color.
  - `CommandOutputBlock`: Displays command name, status (`Running`, `Success (exit 0)`, `Failed (exit N)`, `Cancelled`), elapsed execution time, and streaming log lines.
- **REQ-004**: Command execution MUST be asynchronous (e.g. using `tokio::process::Command` or a dedicated thread worker) communicating back to the TUI event loop via a non-blocking channel (`mpsc`).
- **REQ-005**: Commands MUST be executed via the system shell (`$SHELL -c <cmd>`, defaulting to `sh -c <cmd>` if unset), preserving environment variables and shell semantics (pipes, redirects).
- **REQ-006**: Input box MUST support basic inline text editing: typing characters, `Backspace`, `Delete`, `Left`/`Right` arrow keys, and cursor positioning.
- **REQ-007**: The feed MUST support scrolling:
  - `PageUp` / `PageDown` or `Shift+Up` / `Shift+Down` to navigate lines up and down.
  - Mouse wheel scrolling up and down if mouse capture is enabled.
  - Auto-scroll lock: automatically advances to bottom when new lines arrive if and only if the view was already at the bottom.
- **REQ-008**: Process lifecycle controls MUST allow interrupting an active command (`Ctrl+C` sends SIGINT/SIGTERM to child process) without quitting the TUI. A second `Ctrl+C` or `Esc` when idle exits the application.
- **REQ-009**: Output blocks MUST cap or wrap long lines according to viewport width and support high-throughput line streams without blocking the UI rendering loop.

## Invariants and security boundaries

- **Terminal Recovery**: An abnormal crash or unhandled panic MUST restore terminal raw mode and alternate screen to avoid leaving the user's terminal in a corrupted state (enforced via panic hook).
- **Subprocess Isolation**: Subprocesses MUST run with the user's standard permissions; no elevated privilege escalation is introduced by the runner.
- **UI Non-blocking**: The main event loop MUST NOT block on process I/O or sleep synchronously; terminal rendering MUST remain responsive under heavy stdout volume.
- **Input Sanitization**: Empty or whitespace-only inputs MUST NOT spawn shell processes.

## Definition of done

- `cargo check`, `cargo clippy`, and `cargo test` pass with zero warnings/errors.
- Executing `cargo run` launches the fullscreen TUI interface without visual flicker.
- Running a short command (`echo "hello world"`) displays user block, command block, exit 0, and output.
- Running a streaming command (`for i in {1..5}; do echo $i; sleep 0.5; done`) displays live streaming lines in the command block as they arrive.
- Scrolling up during streaming stops auto-scroll; scrolling to the bottom resumes auto-scroll.
- Pressing `Ctrl+C` during a long-running command interrupts the command cleanly without exiting the TUI.
- Pressing `Esc` exits cleanly and restores the terminal.

## Acceptance criteria

- **AC-001**: Given an empty input box, when the user types `ls -la` and presses `Enter`, then a User Input block containing `ls -la` is added to the feed, followed by a Command Output block showing `[Running]`.
- **AC-002**: Given a running command producing output, when output lines are written to stdout/stderr, then the lines appear incrementally inside the command block within 50ms without UI freezing.
- **AC-003**: Given a command that finishes with exit code 0, when execution ends, then the command block header displays `[Exit 0]` in green and execution duration in seconds/milliseconds.
- **AC-004**: Given a command that finishes with non-zero exit code (e.g. `false`), when execution ends, then the header displays `[Exit 1]` in red.
- **AC-005**: Given an active long command (e.g. `sleep 10`), when the user presses `Ctrl+C`, then the child process terminates, the command block displays `[Cancelled]`, and the input box becomes ready for the next command.
- **AC-006**: Given feed content that overflows the feed height, when the viewport is at the bottom, incoming lines cause the viewport to keep the latest lines in view.
- **AC-007**: Given feed content that overflows the feed height, when the user scrolls up with `PageUp`, incoming lines do not alter the current scroll offset.
- **AC-008**: Given an idle runner, when the user presses `Esc` or `Ctrl+C`, the TUI exits cleanly, leaves no orphan processes, and restores the terminal prompt.

## Edge cases

- **Very high stdout volume** (e.g., `cat /dev/urandom | base64` or `find /`): Buffering and throttling must ensure the TUI does not exhaust memory or lock the rendering loop (cap log lines or chunk render frames).
- **ANSI escape codes in output**: Commands emitting color/style sequences (e.g. `ls --color=always`, `cargo test`) should either be parsed into Ratatui `Span`s or safely stripped so raw escape codes do not corrupt the terminal layout.
- **Window resize**: Terminal SIGWINCH resize events must immediately trigger layout recomputation and reflow without crashing.
- **Rapid Enter key presses**: Pressing Enter while a command is already executing should queue the command, reject it with a visual notification, or disallow running simultaneous commands until the first completes (v1: allow one active command at a time; notify if busy).

## Constraints

- Built in standard Rust (stable toolchain).
- Framework: `ratatui` (latest 0.29+) with `crossterm` backend.
- Async runtime: `tokio` for async process execution, timers, and event multiplexing.
- Cross-platform compatible (macOS, Linux; Windows compatible where `$SHELL` or `powershell.exe` is available).

## Risks and mitigations

- **Risk**: Terminal left in broken state if application panics.
  - **Mitigation**: Install custom panic hook (`better-panic` or standard `std::panic::set_hook`) that explicitly drops alternate screen and disables raw mode before printing backtrace.
- **Risk**: Subprocess produces infinite output or hangs indefinitely.
  - **Mitigation**: Implement explicit process group cancellation on `Ctrl+C` and visual line count / ring-buffer capping.
- **Risk**: Excessive rendering on fast output drains CPU.
  - **Mitigation**: Decouple event ingestion (channel batching / tick rate) from frame rendering (render on tick or state change at max 60fps).

## Open questions

- *None blocking.* Non-blocking follow-up: In milestone 2, should we support ANSI color parsing in stdout (e.g. via `ansi-to-tui` crate)? (Default assumption: Start with plain text / stripped ANSI, then add `ansi-to-tui` parser).

## References

- Ratatui Documentation: https://docs.rs/ratatui/latest/ratatui/
- Crossterm Documentation: https://docs.rs/crossterm/latest/crossterm/
- Tokio Process Documentation: https://docs.rs/tokio/latest/tokio/process/index.html
- Pi.dev UI Paradigm: block-oriented feed with pinned bottom command prompt.
