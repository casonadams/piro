### Completed Refinements & Features

┌───────────────────┬──────────────────────┬─────────────────────────────────┬───────────┐
│ Area              │ Initial State        │ Delivered Solution              │ Status    │
├───────────────────┼──────────────────────┼─────────────────────────────────┼───────────┤
│ Command History   │ Single-use input     │ Persistent history buffer in    │ Completed │
│                   │ buffer               │ ~/.piro/history with Up/Down    │           │
│                   │                      │ & Ctrl+P/N cycling + dedupe     │           │
├───────────────────┼──────────────────────┼─────────────────────────────────┼───────────┤
│ Word Navigation   │ Single-character     │ Full Readline ergonomics:       │ Completed │
│                   │ cursor moves         │ Ctrl+W, Ctrl+A/E, Alt+B/F,      │           │
│                   │                      │ Ctrl+U/K                        │           │
├───────────────────┼──────────────────────┼─────────────────────────────────┼───────────┤
│ ANSI Color        │ Stripped to plain    │ Real-time ANSI escape parsing   │ Completed │
│ Support           │ text                 │ via ansi-to-tui with full style │           │
│                   │                      │ & border alignment              │           │
├───────────────────┼──────────────────────┼─────────────────────────────────┼───────────┤
│ Agent / Tool Call │ User command + CLI   │ FeedItem variants: Thought,     │ Completed │
│ Extensibility     │ output cards only    │ ToolCall, AgentResponse with    │           │
│                   │                      │ custom titled card renderers    │           │
├───────────────────┼──────────────────────┼─────────────────────────────────┼───────────┤
│ The ReAct Loop    │ None (one-off shell  │ Async state machine supervisor  │ Completed │
│                   │ execution only)      │ with Tool trait, BashTool,      │           │
│                   │                      │ MockAgentBackend, prompt        │           │
│                   │                      │ routing (!, !!, prompt), and    │           │
│                   │                      │ instant Esc/Ctrl+C cancellation │           │
└───────────────────┴──────────────────────┴─────────────────────────────────┴───────────┘

──────────────────────────────────────────────────────────────────────────────

### Next Opportunities for Refinement

As `piro` evolves into an autonomous daily driver coding agent harness:

1. **Live LLM API Provider Integration**:
   - Add streaming LLM backend (e.g. Anthropic Claude, OpenAI, Ollama) via lightweight HTTP/SSE client.
2. **Additional Built-in Agent Tools**:
   - `read_file` (with line numbering and windowing)
   - `edit_file` / `write_file`
   - `file_search` / `grep`
3. **Interactive DiffView Block**:
   - Add `FeedItem::DiffView` with side-by-side or unified red/green diff cards for file modifications.
