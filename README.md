# piro

A fast terminal command runner and ReAct coding agent TUI in Rust.

## Features

- **Dual runner & agent**: Run shell commands directly (`!!cmd`), run with agent context (`!cmd`), or talk to the ReAct agent (`prompt`).
- **Clean Ratatui interface**: Horizontal-ruled prompt, bottom-anchored feed, and live command tailing.
- **Scroll & visual mode**: Vi-style navigation (`h`/`j`/`k`/`l`), selection, and instant clipboard copy.
- **Lean binary**: < 2 MiB release binary with zero bloat.

## Quick Start

```bash
cargo build --release
./target/release/piro
```

Optionally specify an LLM model:

```bash
./target/release/piro --model openai/gpt-4o
./target/release/piro --model ollama/qwen2.5-coder:7b
```

## Shortcuts

- `Ctrl+J` / `Ctrl+Enter`: Insert newline in input
- `PageUp` / `Ctrl+S`: Enter Scroll Mode
- `Ctrl+O`: Expand / collapse command output tail
- `Ctrl+C` / `Esc`: Cancel running command or agent turn
- `v` / `V`: Character / line visual selection (in Scroll Mode)
- `y` / `Enter`: Yank selection to clipboard
