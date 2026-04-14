# replay-cli

[![License](https://img.shields.io/badge/license-Apache%202.0-blue.svg)](./LICENSE)
[![Release](https://img.shields.io/github/v/release/skillsynchq/replay-cli?color=stone)](https://github.com/skillsynchq/replay-cli/releases)
[![Website](https://img.shields.io/badge/web-replay.md-stone)](https://replay.md)

The command-line companion to [Replay](https://replay.md). Browse your local Claude Code and Codex sessions in a TUI, then upload the good ones to share as reviewable threads.

## Install

```bash
curl -sSL https://install.replay.md | sh
replay login
replay              # browse sessions
replay upload <id>  # share one
```

Point it at your local Claude Code or Codex history and it gives you an interactive picker with fuzzy search, a readable conversation view, and a one-command upload to [replay.md](https://replay.md) — where the session becomes a browsable thread with inline diffs, tool calls, and the full back-and-forth.

## Features

- Interactive TUI for browsing local sessions across agents
- Fuzzy search across session titles and content
- Readable conversation view — messages, tool calls, diffs, thinking blocks
- One-command upload to Replay with stable URLs
- Browser-based auth flow (Better Auth)

## Supported agents

| Agent       | Session location                        |
|-------------|-----------------------------------------|
| Claude Code | `~/.claude/projects/**/*.jsonl`         |
| Codex       | `~/.codex/sessions/rollout-*.jsonl`     |

Adding a new agent means implementing the `SessionProvider` trait — see `src/providers/`.

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

## Related projects

- [**replay**](https://github.com/skillsynchq/replay) — the web app at [replay.md](https://replay.md) where uploaded sessions live.

## Development

```bash
cargo check         # type-check
cargo test          # run tests
cargo run           # run (defaults to browse)
cargo build --release
# binary at target/release/replay
```

The TUI in `src/tui/` is structured as an Elm-style MVU loop. New agents are added by implementing `SessionProvider` in `src/providers/`.

## Contributing

Issues and pull requests are welcome. For anything non-trivial, open an issue first so we can talk through the direction.

## License

[Apache 2.0](./LICENSE)
