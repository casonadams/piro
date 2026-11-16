# AGENTS.md

## Project Overview

`piro` is a terminal command runner and ReAct agent built as a [Native Shell (NSH)](https://nsh.dev) Rust app, TUI first:
- The root component `piro::App` is mounted by the standalone `piro` binary, which embeds `nsh-platform-tui`'s `AppView` with its own crossterm transport, and by `nsh run --target tui` through `nsh.json`. Other NSH targets reuse the same root.
- No QuickJS: `nsh-platform-tui` uses `default-features = false`.
- Agent, provider and subprocess work runs on a process-wide tokio runtime (`sys::runtime()`). The NSH engine thread never blocks on it and drains events from channels in batches.
- Toolchain is pinned with mise (`mise.toml`: rust, `github:longbridge/nsh`).

## Architecture & Codebase Map

```
src/
├── main.rs       # CLI entry: `piro login`, model validation, launch options, sys::host::run
├── lib.rs        # Modules, `pub use ui::App`, #[native_module] registry for NSH
├── launch.rs     # LaunchOptions set by the binary; model_spec() for the root
├── app/          # Session domain, no NSH types (unit-testable)
│   ├── session.rs# Session state, submit routing, cancel, busy guard, UiRequest
│   ├── apply.rs  # Applying AgentEvent / RunnerMsg to the session
│   ├── channels.rs# Agent/runner channels; reset() drops queued messages of cancelled work
│   ├── feed.rs   # Feed model: cards, command blocks, streamed agent slots
│   ├── history.rs# Prompt history (~/.piro/history), caret helpers
│   ├── route.rs  # parse_input_intent: !!cmd, !cmd, prompts, /clear, /login
│   └── cli.rs    # Argument parsing (--model, `login [provider]`)
├── ui/           # NSH components
│   ├── mod.rs    # App root: feed, spacer row, composer, login dialog, footer
│   ├── bridge.rs # Ui: session behind Rc<RefCell>, mirrored into signals; inbox drain task
│   ├── feed.rs / card_text.rs # Virtual list of cards; pure card text and accents
│   ├── composer.rs # Multiline field between text rules, focus on first layout, history keys
│   ├── keys.rs   # Global and busy-only keymaps
│   ├── login.rs  # /login dialog: authorize URL, pasted code, Claude Code import
│   ├── footer.rs / palette.rs
│   └── headless_tests.rs # nsh-testing headless UI tests
├── sys/          # OS, process and terminal host
│   ├── host.rs   # Standalone AppView host loop (run_app), ~/.piro data dir and log
│   ├── terminal.rs (+ caps.rs, input.rs) # Session modes, capability detection, crossterm -> NSH Input
│   ├── clipboard.rs # NSH ClipboardBackend: OSC 52 between frames (+ pbcopy on macOS)
│   ├── rt.rs     # Process-wide tokio runtime
│   ├── runner.rs # $SHELL -c subprocesses, process groups, kill
│   └── signals.rs / filelog.rs
├── agent/        # ReAct loop
│   ├── message.rs# AgentMessage, AssistantTurn, ToolCall (ids), close_dangling_tool_calls
│   ├── react.rs  # ReActEngine: whole turns, all tool calls per turn, Committed events, hooks
│   └── tool.rs   # Tool trait, BashTool, ToolRegistry, ToolExit
├── auth/         # Credentials
│   ├── store.rs  # ~/.piro/auth.json, atomic 0600 writes
│   ├── pkce.rs   # PKCE S256 + state
│   ├── login.rs  # Blocking `piro login` flow
│   └── claude/   # Claude OAuth: authorize/exchange/refresh, Claude Code credential import
└── provider/     # LLM providers
    ├── mod.rs    # LlmProvider trait, AnyProvider, model spec routing
    ├── sse.rs    # UTF-8 safe SSE line decoder shared by providers
    ├── http.rs   # Shared reqwest client
    ├── openai.rs # OpenAI-compatible adapter (Ollama, OpenAI, OpenRouter, DeepSeek, Groq)
    └── claude/   # Anthropic Messages API: claude/ (OAuth) and anthropic/ (API key)
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
3. **Dependencies**:
   - Maintain a minimal dependency set: no unnecessary umbrella crates, no `chrono`. Prefer what NSH already pulls in.
4. **File Length**:
   - Keep files modular (< 300 lines per file where possible).
5. **Hermetic Testing Invariant**:
   - Unit tests (`cargo test`) MUST be 100% offline, deterministic, and hermetic.
   - Never perform live network I/O, connect to localhost daemons (e.g. Ollama, Docker), or rely on external internet access in tests.
   - All provider communication and external interactions in tests must be mocked.

## Prompt Routing & Keys

- **Routing**:
  - `!!<cmd>`: Direct shell execution (bypasses agent).
  - `!<cmd>`: Runs shell command and feeds output to the ReAct agent context.
  - `<prompt>` (unprefixed): Starts the ReAct agent loop.
  - `/clear`: Clears the feed and agent context. `/login`: Claude sign-in dialog.
- **Composer** (NSH text field): readline keys come from NSH. Enter submits; `Ctrl+J`, `Shift+Enter` and `Alt+Enter` insert a newline. `Up`/`Down` (`Ctrl+P`/`Ctrl+N`) recall history from the first/last line.
- **Global**: `Ctrl+O` toggles the latest output block. `Ctrl+L` clears.
- **Busy**: `Esc` / `Ctrl+C` cancel the running command or agent turn. When idle, NSH defaults apply: `Ctrl+C` copies a selection, or quits without one.
- **Scroll & selection**: NSH built-ins: wheel, PageUp/PageDown, mouse selection, `Ctrl+S`/`Ctrl+R` search, copy over OSC 52. Each card has a `copy` action.

## UI & Layout Invariants

- **Input Prompt**: muted top and bottom rules, drawn as clipped text because NSH's grid layout draws no border around a lone control. No side borders and no prompt glyph. The composer takes focus on its first layout.
- **Separator**: exactly 1 empty row between the feed and the prompt's top rule.
- **Cards**: cyan for user input and agent responses; magenta for thoughts; yellow/green/red/muted for running/success/failed/cancelled commands. Live output keeps an 8-line tail with `... N lines hidden (Ctrl+O to expand) ...`.
- **Verification**: headless tests check text only. Check layout by running the binary in a pty, or with `mise run dev`.

## Quality Gates

Every change must pass all gates with zero warnings (`mise run check` runs all four):

```bash
mise run fmt     # cargo fmt --check
mise run lint    # cargo clippy --all-targets -- -D warnings
mise run test    # cargo test
mise run build   # cargo build --release
```
