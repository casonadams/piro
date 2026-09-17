# Ratatui Ecosystem Integration Spec

## Status

Approved

## Problem

`tuitest` currently strips all ANSI escape codes from command stdout/stderr to prevent terminal corruption, which robs users of valuable syntax highlighting (such as green test passes in `cargo test`, red/green diffs in `git diff`, or directory colors in `ls --color`). Additionally, text input uses a rudimentary custom buffer that lacks standard terminal/Readline editing ergonomics (`Ctrl+W` delete word, `Ctrl+A`/`Ctrl+E` home/end, `Alt+B`/`Alt+F` word navigation) and has no command history recall. The Ratatui ecosystem has mature, battle-tested solutions (`ansi-to-tui`, `tui-input`) that solve these exact concerns. Integrating them will significantly enhance user experience without adding bloat.

## Users and stakeholders

- **CLI Developer**: Runs commands expecting standard ANSI color rendering (e.g. compilers, test runners, git) and standard shell editing shortcuts (`Ctrl+W`, `Ctrl+A`, `Ctrl+E`, `Up`/`Down` history recall).
- **Maintainers**: Benefit from replacing custom text buffer logic with established ecosystem crates while preserving strict size (<1MB binary) and line budgets (<800 lines of code).

## Goals

- **Rich ANSI Color Rendering**: Integrate `ansi-to-tui` to parse raw subprocess output lines into styled Ratatui `Line<'static>` objects with accurate foreground, background, and modifier styles.
- **Advanced Input Ergonomics**: Integrate `tui-input` to provide standard Readline/Emacs shortcuts:
  - `Ctrl+W` / `Alt+Backspace`: Delete previous word
  - `Ctrl+U`: Discard line before cursor
  - `Ctrl+K`: Discard line after cursor
  - `Ctrl+A` / `Home`: Jump to start of line
  - `Ctrl+E` / `End`: Jump to end of line
  - `Alt+B` / `Alt+F` / `Ctrl+Left` / `Ctrl+Right`: Jump word backward/forward
- **Command History**: Implement an in-memory command history buffer:
  - `Up` arrow recalls previous command.
  - `Down` arrow moves toward newer commands (or restores pending draft).
- **Preserve Invariants**:
  - Keep the clean horizontal-ruled input area (no side borders, no `>` prompt).
  - Keep 1-decimal-place duration precision (`Took 0.0s`).
  - Keep right-border alignment via `fit_to_width`.
  - Keep release binary size under 1.0 MiB.

## Non-goals

- Persistent disk storage / SQLite database for shell history across sessions (in-memory history for current session only).
- Interactive curses PTY applications (`vim`, `top`, `nano`) — line-oriented CLI execution remains the target.
- Syntax highlighting inside the input prompt itself (input remains clean white text).

## Current behavior

1. `src/runner.rs` calls `strip_ansi_escapes::strip(...)` on every stdout/stderr line, discarding all colors and styles.
2. `src/app.rs` implements a custom `InputState` supporting only single-character insertion, backspace, delete, and left/right arrows.
3. Pressing `Up` or `Down` in Input Mode does nothing (or triggers scroll mode navigation if mapped).

## Desired behavior

1. Subprocess output lines containing ANSI escape sequences (e.g. `\x1b[32mtest result: ok\x1b[0m`) are converted via `ansi-to-tui` into Ratatui `Line`s with matching colors.
2. When rendered inside a `CommandOutputBlock`, the colored spans render with correct colors, while right border padding is preserved cleanly.
3. The input prompt uses `tui-input` to process keystrokes. Users can use `Ctrl+W` to delete words, `Ctrl+A`/`Ctrl+E` to navigate, and `Alt+B`/`Alt+F` to skip words.
4. Pressing `Up` arrow in Input Mode cycles backward through previously executed commands. Pressing `Down` arrow moves forward.
5. In Scroll Mode, keyboard and mouse selection continues to work identically, allowing users to copy colored or plain text cleanly.

## Requirements

- **REQ-001**: Subprocess runner (`src/runner.rs`) MUST preserve raw bytes/ANSI sequences from stdout and stderr instead of stripping them unconditionally.
- **REQ-002**: `src/ui/blocks.rs` MUST parse ANSI sequences using `ansi-to-tui` into styled Ratatui `Line` objects.
- **REQ-003**: Line wrapping / width fitting in `CommandOutputBlock` MUST properly calculate visual width across styled spans and ensure right border `│` alignment.
- **REQ-004**: `InputState` MUST leverage `tui-input` to handle character insertions, deletions, and standard Readline shortcuts (`Ctrl+W`, `Ctrl+U`, `Ctrl+K`, `Ctrl+A`, `Ctrl+E`, `Alt+B`, `Alt+F`).
- **REQ-005**: Application MUST maintain an in-memory command history list (up to 500 entries) navigable via `Up` and `Down` arrows in Input Mode.
- **REQ-006**: Submitting an identical consecutive command MUST NOT duplicate it in the history list.
- **REQ-007**: Release binary footprint MUST remain under 1.0 MiB (`cargo bloat --release --crates`).

## Invariants and security boundaries

- **No Stdin Races**: No calls to `crossterm::cursor::position()` or `insert_before()` while `EventStream` is active. Fullscreen alternate screen viewport is maintained.
- **Terminal Reset Invariant**: Styles from ANSI sequences MUST NOT bleed across lines or into outer block borders (Ratatui styles are scoped per span).
- **Subprocess Isolation**: Subprocesses run with standard user permissions in user's `$SHELL`.

## Definition of done

- `cargo fmt --check`, `cargo clippy -- -D warnings`, and `cargo test` pass with zero errors/warnings.
- Running a colored command (e.g. `cargo test --color=always` or `git status --color=always`) displays true green, red, and yellow colors inside the command block.
- `Ctrl+W` deletes the previous word in the input prompt.
- Pressing `Up` arrow in the input prompt recalls previously executed commands.
- Release binary size verified under 1.0 MiB via `cargo bloat --release --crates`.

## Acceptance criteria

- **AC-001**: Given a command that emits ANSI colors (e.g. `printf '\033[32mGREEN\033[0m \033[31mRED\033[0m\n'`), when executed, then the output card displays "GREEN" in green and "RED" in red.
- **AC-002**: Given input text `cargo test --release`, when the user presses `Ctrl+W`, then `--release` is deleted, leaving `cargo test `.
- **AC-003**: Given input text `git commit -m "fix"`, when the user presses `Ctrl+A`, then the cursor moves to the start of the line before `g`.
- **AC-004**: Given previously executed commands `echo 1` and `echo 2`, when the user presses `Up` in an empty prompt, then `echo 2` is populated. Pressing `Up` again populates `echo 1`.
- **AC-005**: Given an active selection in Scroll Mode, when yanked (`y`/`Enter`), the copied text contains clean string content without ANSI escape sequence artifacts.

## Edge cases

- **Malformed ANSI sequences**: `ansi-to-tui` gracefully ignores invalid or truncated escape codes without crashing.
- **Overlong colored lines**: Lines wider than the block width are safely truncated or wrapped so the right border `│` remains aligned.
- **Draft preservation**: If the user types a partial command (e.g. `car`), presses `Up` to inspect history, and presses `Down` back to the bottom, the draft `car` is restored.

## Constraints

- Rust stable toolchain.
- `ansi-to-tui` (v8.x, MIT/Apache-2.0 license).
- `tui-input` (v0.15.x, MIT license).
- Binary size under 1.0 MiB.

## Risks and mitigations

- **Risk**: `ansi-to-tui` introduces compile bloat.
  - **Mitigation**: Verify with `cargo bloat`. `ansi-to-tui` is a pure parser with negligible footprint.
- **Risk**: Wide or multibyte ANSI sequences cause border misalignment.
  - **Mitigation**: Use `unicode-width` across parsed spans to calculate exact padding spaces.

## References

- Ratatui documentation: https://docs.rs/ratatui/latest/ratatui/
- `ansi-to-tui` on crates.io: https://crates.io/crates/ansi-to-tui
- `tui-input` on crates.io: https://crates.io/crates/tui-input
