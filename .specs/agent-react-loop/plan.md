# ReAct Agent Loop Implementation Plan

## Overview

Implement an asynchronous, non-blocking ReAct (Reasoning + Acting) loop in `piro` with pluggable tool execution, agent-first prompt routing (`!`, `!!`, and plain prompt), and styled feed cards for agent thoughts, tool calls, and responses.

Source Spec: `.specs/agent-react-loop/spec.md`

---

## Architectural Slices

```
src/
├── agent/
│   ├── mod.rs          # ReAct state machine loop & AgentEvent streaming
│   ├── tool.rs         # Tool trait & BashTool implementation
│   └── backend.rs      # AgentBackend trait & MockAgentBackend
├── app.rs              # Extended FeedItem enum, agent process tracking, routing
├── runner.rs           # Core subprocess execution (reused by BashTool)
├── ui/
│   ├── blocks.rs       # Thought, ToolCall, and Response visual card renderers
│   ├── feed.rs         # Virtual scroll slice & text selection extractor updates
│   └── mod.rs          # Layout & footer status integration (RUNNING / THINKING)
└── main.rs             # Event multiplexing: crossterm events, runner rx, agent rx
```

---

## Vertical Slices & Tasks

### Slice 1: Feed Data Model & UI Card Renderers (Estimate: 3 pts)
**Goal**: Expand `FeedItem` and implement Ratatui box-drawing renderers for agent thoughts, tool calls, and agent responses, with full selection text extraction and word wrapping.

- [ ] **Task 1.1**: Update `FeedItem` in `src/app.rs`:
  - Add `AgentThought(String)`.
  - Add `ToolCall { name: String, input: String, block: CommandOutputBlock }`.
  - Add `AgentResponse(String)`.
- [ ] **Task 1.2**: Add card renderers in `src/ui/blocks.rs`:
  - `render_agent_thought_block(thought: &str, width: usize)`: Distinct Magenta border `┌─ Thought ───┐`, italic/dim styling, word wrapping.
  - `render_tool_call_block(tool: &str, input: &str, block: &CommandOutputBlock, width: usize)`: Blue/Cyan border `┌─ Tool: <tool>(<args>) ───┐`, embeds tool output tail, duration, and status.
  - `render_agent_response_block(response: &str, width: usize)`: Green/Cyan border `┌─ Agent ───┐`, bold response content, word wrapping.
- [ ] **Task 1.3**: Update `src/ui/feed.rs`:
  - Update `format_feed_item` to dispatch to the new renderers.
  - Update `extract_raw_line` to support copying clean unwrapped text from thoughts, tool blocks, and agent responses.
- [ ] **Task 1.4**: Unit tests for new card formatting, border alignment (`inner_w`), and selection extraction.

### Slice 2: Tool Abstraction & Bash Tool (Estimate: 2 pts)
**Goal**: Define the asynchronous `Tool` trait and implement `BashTool` reusing `piro`'s existing subprocess management.

- [ ] **Task 2.1**: Create `src/agent/tool.rs`:
  - Define `Tool` trait:
    ```rust
    pub trait Tool: Send + Sync {
        fn name(&self) -> &'static str;
        fn description(&self) -> &'static str;
        async fn execute(&self, args: &str, tx: mpsc::UnboundedSender<RunnerMsg>) -> Result<String, String>;
    }
    ```
- [ ] **Task 2.2**: Implement `BashTool`:
  - Delegates to `runner::spawn_command`.
  - Aggregates stdout/stderr into a single result string for agent observation while simultaneously streaming lines to the UI via `RunnerMsg`.
- [ ] **Task 2.3**: Unit tests for tool execution, success observation, and error handling.

### Slice 3: ReAct Engine & Mock/Scripted Backend (Estimate: 3 pts)
**Goal**: Build the core async ReAct state machine and a deterministic scripted backend to verify the loop without external dependencies.

- [ ] **Task 3.1**: Define `AgentEvent` and `AgentBackend` in `src/agent/backend.rs`:
  - `StepResult`: `Thought(String)`, `CallTool { tool: String, input: String }`, `FinalAnswer(String)`.
  - `AgentBackend` trait: `async fn next_step(&mut self, history: &[AgentMessage]) -> Result<StepResult, String>`.
  - `MockAgentBackend`: Scriptable/rule-based agent that responds to prompts (e.g. keywords like `count`, `search`, `status`) with thoughts, tool calls (e.g. `ls`, `git status`), observations, and final answers.
- [ ] **Task 3.2**: Implement the ReAct loop supervisor in `src/agent/mod.rs`:
  - Coordinates turns: Prompt -> Thought -> Tool Execution -> Observation -> Loop (up to max iterations).
  - Channels events back to UI:
    ```rust
    pub enum AgentEvent {
        Thought(String),
        ToolStart { tool: String, input: String },
        ToolOutputLine(String),
        ToolDone { status: CommandStatus, duration: Duration },
        Response(String),
        Finished,
        Cancelled,
        Error(String),
    }
    ```
  - Exposes `AgentHandle` with `abort()` capability for instant `Esc`/`Ctrl+C` cancellation.
- [ ] **Task 3.3**: Unit tests for complete multi-step ReAct loop and loop cancellation.

### Slice 4: Prompt Routing & Event Multiplexing Integration (Estimate: 2 pts)
**Goal**: Wire prompt routing (`!!`, `!`, agent-first prompt) and `AgentEvent` stream into `src/main.rs` and `src/app.rs`.

- [ ] **Task 4.1**: Input parsing & routing in `src/app.rs`:
  - `!!<cmd>`: Direct shell runner (original one-off runner behavior).
  - `!<cmd>`: Spawns shell command and queues output directly into agent context upon completion.
  - `<prompt>`: Spawns ReAct agent supervisor with prompt.
- [ ] **Task 4.2**: Event loop integration in `src/main.rs`:
  - Add `agent_rx: mpsc::UnboundedReceiver<AgentEvent>` to `tokio::select!`.
  - Handle thought streaming, tool execution card creation/updates, and final response addition.
  - Wire `Esc` / `Ctrl+C` to cancel both `active_process` and `active_agent`.
- [ ] **Task 4.3**: Update footer status:
  - Display ` THINKING ` or ` AGENT RUNNING ` when the agent is actively executing turns.

### Slice 5: Quality Gates, Testing & Bloat Verification (Estimate: 1 pt)
**Goal**: Verify all quality gates, line limits, and binary size budgets.

- [ ] **Task 5.1**: Run full test suite: `cargo test`.
- [ ] **Task 5.2**: Run formatting and clippy: `cargo fmt --check`, `cargo clippy -- -D warnings`.
- [ ] **Task 5.3**: Run size benchmarks: `tokei -e target` and `cargo bloat --release --crates` (< 1.0 MiB target).

---

## Verification Plan

1. **Automated Unit Tests**:
   - `cargo test`: Verify all feed item renderers, selection extraction, tool execution, and mock ReAct loop transitions pass.
2. **Interactive Manual Checks**:
   - `!!echo "hello world"`: Runs directly as single command output block.
   - `!git status`: Runs command and feeds observation to agent.
   - `List all files and summarize`: Triggers agent loop -> Thought card appears -> Tool call card appears and streams output -> Final answer card appears.
   - Press `Esc` mid-run: Instantly cancels loop and returns status to `READY`.
   - Press `Ctrl+S`: Visual mode navigates smoothly through thoughts, tool outputs, and answers, copying unwrapped text cleanly via `y` / `Enter`.
