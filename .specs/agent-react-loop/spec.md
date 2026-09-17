# ReAct Agent Loop & Extensible Tool Calling Spec

## Status

Approved

## Problem

`piro` is currently a single-command shell runner: each user input triggers an isolated `$SHELL -c <cmd>` execution and displays a single command output card. To evolve into an autonomous terminal agent (inspired by pi.dev, Claude Code, and Aider), `piro` requires an asynchronous ReAct (Reasoning + Acting) loop. The agent must be able to ponder a goal, invoke tools (such as bash, file inspection), observe their results, and iterate autonomously until producing a final answer, while preserving `piro`'s signature non-blocking TUI responsiveness, visual tmux-style selection, full ANSI rendering, and strict low-bloat binary size invariants.

## Users and stakeholders

- **Developer / Agent Operator**: Interacts with an autonomous terminal assistant that can reason and execute multi-step tool commands to answer questions or solve tasks in the repository.
- **Maintainers**: Benefit from a clean, decoupled architecture: UI feed presentation is separate from the async agent engine, and tools adhere to an extensible trait without heavy third-party framework dependencies.

## Goals

- **Agent-First Prompt Ergonomics**:
  - Unprefixed user input (e.g. `Find all rust files and count lines`) triggers the ReAct agent loop.
  - Prefix `! <cmd>` (or `!<cmd>`): Executes the shell command and automatically passes its output into the agent context.
  - Prefix `!! <cmd>` (or `!!<cmd>`): Executes raw shell command directly without invoking the agent (preserving existing one-off shell runner behavior).
- **Extensible `FeedItem` Model**:
  - Expand `FeedItem` to represent distinct agent interaction steps:
    - `UserInput(String)`: The original prompt or command.
    - `AgentThought(String)`: Intermediate reasoning from the agent (distinct styling, e.g. Magenta/Purple border).
    - `ToolCall { id: String, tool: String, input: String, output: CommandOutputBlock }`: Represents a tool invocation and its streaming output card.
    - `AgentResponse(String)`: Final answer from the agent (clean response card or styled block).
    - `CommandOutput(CommandOutputBlock)`: Direct shell execution (`!!`).
- **Asynchronous ReAct Engine**:
  - Drive a finite state machine: `Idle` -> `Thinking` -> `ToolCalling` -> `Observing` -> `FinalAnswer` / `Done`.
  - Fully asynchronous and non-blocking, integrated via `tokio::sync::mpsc` into the master event loop in `src/main.rs`.
  - Support instant cancellation at any step via `Esc` or `Ctrl+C`, aborting active tool processes and agent reasoning turns.
- **Pluggable `Tool` Trait**:
  - Lightweight, async tool abstraction:
    ```rust
    pub trait Tool: Send + Sync {
        fn name(&self) -> &'static str;
        fn description(&self) -> &'static str;
        async fn execute(&self, args: &str, tx: mpsc::UnboundedSender<RunnerMsg>) -> Result<String, String>;
    }
    ```
  - Built-in `BashTool` wrapping `piro`'s existing process supervisor (`ProcessHandle`, process group kill).
- **Deterministic Mock/Scripted Agent Backend**:
  - Provide a testable agent backend for verifying the loop, multi-step tool calls, and UI rendering without requiring external API keys or network requests.
- **Preserve All Existing Invariants**:
  - Tmux-style visual char/line selection, soft-wrap unwrap copying (`y`/`Enter`), smooth scrolling (`j`/`k`/mouse), live tailing, and right-border alignment must work across all new card types.
  - Zero bloat: Maintain total Rust code under strict size limits, with release binary footprint remaining under 1.0 MiB.

## Non-goals

- Network HTTP client implementation or live third-party LLM API provider integration (Anthropic, OpenAI, Ollama) in this phase. The backend will be decoupled via an `AgentBackend` trait so real providers can be plugged in during the subsequent step.
- Multi-agent orchestration, swarm protocols, or vector database embeddings.
- Persistent session database or conversation branching.

## Current behavior

1. All input submitted in `src/app.rs` is directly passed to `runner::spawn_command(cmd, tx)`.
2. `FeedItem` only has two variants: `UserInput(String)` and `CommandOutput(CommandOutputBlock)`.
3. There is no concept of agent reasoning, multi-step tool execution, or contextual feedback loops.

## Desired behavior

1. When a user enters `Find TODO items in the project`:
   - An `AgentThought` card appears (e.g. `Searching for TODO comments in source code...`).
   - A `ToolCall` card appears for `bash: rg "TODO"`, streaming the output lines live using the existing tailing card.
   - The agent receives the observation and emits another thought or a final `AgentResponse` card.
2. When a user enters `!cargo check`:
   - A `CommandOutputBlock` runs `cargo check` and on completion feeds the output as an observation to the agent for interpretation.
3. When a user enters `!!cargo test`:
   - The command runs directly as an isolated shell command, exactly like `piro` does today.
4. Pressing `Esc` or `Ctrl+C` while the agent is running immediately halts any active tool process and stops the agent loop, leaving all rendered feed items intact.

## Requirements

- **REQ-001**: User input routing:
  - `!!<cmd>` executes direct shell runner without agent.
  - `!<cmd>` executes shell command and injects output into agent conversation.
  - Any other input initiates the ReAct agent loop.
- **REQ-002**: `FeedItem` variants:
  - Add `AgentThought(String)`, `AgentResponse(String)`, and `ToolCall { name: String, input: String, block: CommandOutputBlock }`.
- **REQ-003**: Card rendering:
  - Distinct border styling and visual indicators for Agent Thoughts (e.g. Magenta `│ Thought: ...`), Tool Calls (e.g. Blue/Cyan header with tool name + args), and Agent Responses (e.g. Green/Cyan).
  - All new cards must respect word wrapping, padding, border bounds (`inner_w = width - 4`), and selection highlighting.
- **REQ-004**: Tool abstraction:
  - Define `Tool` trait with `name()`, `description()`, and `execute(...)`.
  - Implement `BashTool` reusing `runner::spawn_command`.
- **REQ-005**: ReAct Engine:
  - Implement an async state machine driving the loop up to a maximum step limit (e.g. 10 steps).
  - Stream events (`AgentEvent::Thought`, `AgentEvent::ToolStart`, `AgentEvent::ToolOutput`, `AgentEvent::ToolDone`, `AgentEvent::Response`, `AgentEvent::Finished`, `AgentEvent::Error`) across an `mpsc` channel.
- **REQ-006**: Cancellation:
  - `App::cancel_active()` terminates both in-flight tool processes and the agent task supervisor.
- **REQ-007**: Selection & Clipboard integration:
  - Text extraction in `ui/feed.rs` must correctly extract unformatted or unwrapped text from thought cards, tool cards, and agent response cards.

## Invariants and security boundaries

- **Zero-Bloat Invariant**: No external agent framework crates (no LangChain, no rig-core, no async-trait if std desugaring suffices). Minimal dependencies only.
- **TUI Responsiveness**: The UI draw loop must never block on tool execution or agent reasoning; communication must occur via async unbounded or bounded mpsc channels.
- **Process Group Isolation**: Any tool execution invoking bash commands must retain `process_group(0)` and `kill_on_drop(true)` with `SIGTERM`/`SIGKILL` cleanup.
- **Layout Invariant**: Exactly 1 empty row separator between feed and input rules; clean top/bottom divider rules on the input prompt with no side borders.

## Definition of done

- Unit tests cover:
  - Input routing (`!!`, `!`, agent prompt).
  - ReAct loop progression through thought, tool call, observation, and final answer.
  - Cancellation of agent loops mid-flight.
  - Selection text extraction across all `FeedItem` card types.
  - Formatting and border alignment for new card renderers.
- All quality gates pass: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`.
- Binary size verified via `cargo bloat --release --crates` remains under 1.0 MiB.

## Acceptance criteria

- **AC-001**: Given input `!!ls -la`, when submitted, system executes `ls -la` directly as a single `CommandOutput` block without triggering the agent.
- **AC-002**: Given input `!git status`, when submitted, system executes `git status` and upon completion passes stdout/stderr to the agent.
- **AC-003**: Given input `Count Rust lines`, when submitted, system spawns the ReAct agent, renders an `AgentThought`, renders a `ToolCall` for bash, executes it, captures output, and renders an `AgentResponse`.
- **AC-004**: Given an active multi-step agent loop, when `Esc` or `Ctrl+C` is pressed, all active subprocesses and background tasks terminate within 100ms, and the status bar returns to `READY`.
- **AC-005**: Given thoughts and responses in the feed, entering Scroll Mode (`Ctrl+S`) allows highlighting and copying their text cleanly via `v`/`V`/mouse and `y`/`Enter`.

## Edge cases

- Agent tool call returns empty output: display `(no output)` without crashing or panicking.
- Agent tool command fails (non-zero exit): feed error into the observation so agent can observe and self-correct.
- Agent loop reaches max iterations without concluding: safely terminate with a notice card `Max steps reached`.
- User clears screen (`Ctrl+L`): clears all feed items and resets scroll offset without crashing active or idle agent.

## Constraints

- Rust 2024 edition.
- Codebase size: Keep code modular and compact (< 300 lines per file).
- No new heavy crates.

## References

- `src/runner.rs`: Subprocess spawning and `ProcessHandle`.
- `src/app.rs`: State machine and feed storage.
- `src/ui/blocks.rs`: Card border and text styling.
- `src/ui/feed.rs`: Virtual scroll and selection unwrap.
- `TODO.md`: Agent / Tool Call Extensibility roadmap.
