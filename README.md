# Replay CLI

A Rust CLI tool for browsing and uploading AI coding sessions from [Claude Code](https://claude.ai/code) and [Codex](https://openai.com/index/introducing-codex/).

## Install

```bash
cargo build --release
# binary at target/release/replay
```

## Usage

```bash
replay              # Launch interactive TUI session browser (default)
replay browse       # Same as above
replay upload <id>  # Upload a session by ID
replay login        # Authenticate with the Replay server
replay logout       # Remove stored credentials
replay whoami       # Show current authenticated user
```

### TUI Controls

- **↑/↓** or **j/k** — navigate session list
- **/** — fuzzy search sessions
- **Enter** — view conversation
- **q** / **Esc** — back / quit

## Supported Agents

| Agent | Session location |
|-------|-----------------|
| Claude Code | `~/.claude/projects/**/*.jsonl` |
| Codex | `~/.codex/sessions/rollout-*.jsonl` |

## Architecture

- **CLI** — clap-based arg parsing (`cli.rs`)
- **Providers** — pluggable `SessionProvider` trait for discovering and loading sessions
- **Models** — `Session`, `Message`, `LoadedSession`, `Agent`, `Role`
- **TUI** — ratatui + crossterm, Elm-style MVU pattern with fuzzy search via `nucleo-matcher`
- **Upload** — HTTP upload to Replay server with auth
- **Auth** — Browser-based Better Auth flow

## Development

```bash
cargo check          # Type-check
cargo test           # Run tests
cargo build          # Debug build
cargo run            # Run (defaults to browse)
```
