# piro

A terminal command runner and ReAct coding agent, built as a [Native Shell](https://nsh.dev) Rust app (TUI first).

## Features

- **Runner and agent in one prompt**: run shell commands directly (`!!cmd`), run with agent context (`!cmd`), or talk to the ReAct agent (`prompt`).
- **NSH interface**: bottom-anchored feed of cards, live command tailing, a ruled multiline prompt, and NSH's own selection, search, and OSC 52 copy.
- **Providers**: Ollama, OpenAI, OpenRouter, DeepSeek, Groq (OpenAI-compatible), and Claude through subscription OAuth (`claude/...`) or an API key (`anthropic/...`).

## Quick Start

Tools are pinned with [mise](https://mise.jdx.dev):

```bash
mise install
mise run build
./target/release/piro
```

Choose a model:

```bash
./target/release/piro --model ollama/qwen2.5-coder:7b
./target/release/piro --model openai/gpt-4o
ANTHROPIC_API_KEY=... ./target/release/piro --model anthropic/sonnet
```

Claude subscription login (or `/login` inside the TUI):

```bash
./target/release/piro login claude
./target/release/piro --model claude/sonnet
```

`piro login claude` offers to import existing Claude Code credentials. Otherwise it opens the browser and asks for the pasted code. Credentials are stored in `~/.piro/auth.json` with mode 0600. Note: Anthropic limits subscription OAuth to its own clients, so using it from piro may get the account flagged.

Run in the NSH terminal development host, which rebuilds on change:

```bash
mise run dev
```

## Keys

- `Enter`: submit. `Ctrl+J` / `Shift+Enter`: newline
- `Up` / `Down`: history
- `Ctrl+O`: expand or collapse the latest command output
- `Esc` / `Ctrl+C`: cancel the running command or agent turn
- `Ctrl+S` / `Ctrl+R`: search the screen. Mouse drag selects, `Ctrl+C` copies
- `/clear`, `/login`
