### 2. Next Opportunities for Refinement

As we prepare to introduce more pi.dev agent functionality, here are the most
impactful quality-of-life additions:

┌───────────────────┬──────────────────────┬─────────────────────────────────┐
│ Area              │ Current State        │ Natural Next Step               │
├───────────────────┼──────────────────────┼─────────────────────────────────┤
│ Command History   │ Single-use input     │ Up / Down arrows in Input Mode  │
│                   │ buffer               │ to cycle through previously     │
│                   │                      │ executed commands               │
├───────────────────┼──────────────────────┼─────────────────────────────────┤
│ Word Navigation   │ Single-character     │ Ctrl+W to delete word, Ctrl+A / │
│                   │ cursor moves         │ Ctrl+E for start/end of line,   │
│                   │                      │ Alt+B / Alt+F to skip words     │
├───────────────────┼──────────────────────┼─────────────────────────────────┤
│ ANSI Color        │ Stripped to plain    │ Preserve ANSI color codes (e.g. │
│ Support           │ text for safety      │ green checkmarks in cargo test  │
│                   │                      │ or colored ls) via ansi-to-tui  │
├───────────────────┼──────────────────────┼─────────────────────────────────┤
│ Agent / Tool Call │ User command + CLI   │ Add new FeedItem variants:      │
│ Extensibility     │ output cards         │ AgentMessage, ToolExecution,    │
│                   │                      │ DiffView                        │
└───────────────────┴──────────────────────┴─────────────────────────────────┘

──────────────────────────────────────────────────────────────────────────────

### Verdict

The core foundation (async process execution, styled block feed, tmux visual
selection, and clean split layout) is solid, responsive, and completely free
of bloat.

