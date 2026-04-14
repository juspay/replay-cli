# replay-cli

Share your AI coding sessions. Upload a conversation from Claude Code or Codex, get a permanent link. That's it.

```bash
curl -sSL https://install.replay.md | sh
replay login
replay              # browse sessions
replay upload <id>  # share one
```

Point it at your local Claude Code or Codex history and it gives you an interactive picker with fuzzy search, a readable conversation view, and a one-command upload to Replay — where the session becomes a browsable thread with inline diffs, tool calls, and the full back-and-forth.

## Features

- Interactive TUI for browsing local sessions across agents
- Fuzzy search across session titles and content
- Readable conversation view — messages, tool calls, diffs, thinking blocks
- One-command upload to Replay with stable URLs
- Browser-based auth flow (Better Auth)

## Supported agents

| Agent | Session location |
|-------|-----------------|
| Claude Code | `~/.claude/projects/**/*.jsonl` |
| Codex | `~/.codex/sessions/rollout-*.jsonl` |

Adding a new agent means implementing the `SessionProvider` trait — see `providers/`.

## Commands

```bash
replay              # Launch the TUI (default)
replay browse       # Same as above
replay upload <id>  # Upload a session by ID
replay login        # Authenticate with the Replay server
replay logout       # Remove stored credentials
replay whoami       # Show the current authenticated user
```

### TUI controls

- **↑/↓** or **j/k** — navigate
- **/** — fuzzy search
- **Enter** — open conversation
- **q** / **Esc** — back / quit

## Getting started

```bash
cargo build --release
# binary at target/release/replay
```

Or for development:

```bash
cargo check         # type-check
cargo test          # run tests
cargo run           # run (defaults to browse)
```

## Project structure

```
src/
  cli.rs            # clap arg parsing
  main.rs           # command routing
  providers/        # SessionProvider trait + Claude/Codex impls
  models/           # Session, Message, Agent, Role
  tui/              # ratatui UI — Elm-style MVU
  upload/           # HTTP upload to the Replay server
  auth/             # Better Auth browser flow
```
