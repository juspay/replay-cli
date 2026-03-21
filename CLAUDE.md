# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
cargo build                      # Debug build
cargo build --release            # Release build
cargo run                        # Default: browse command (launches TUI)
cargo run -- upload <session_id> # Upload a specific session
cargo test                       # Run all tests
cargo check                      # Type-check without building
```

## Architecture

Rust CLI tool (`replay`) for browsing and uploading AI coding sessions (Claude Code, Codex). Two commands: `browse` (default, interactive TUI) and `upload`.

### Key Layers

- **CLI** (`cli.rs`) — clap derive-based arg parsing, routes to `run_browse()` or `run_upload()` in `main.rs`
- **Providers** (`providers/`) — `SessionProvider` trait with `discover()` (metadata scan) and `load_messages()` (full parse). Implementations:
  - `ClaudeProvider` — reads `~/.claude/projects/**/*.jsonl`, skips `subagents/` and `tool-results/`
  - `CodexProvider` — reads `~/.codex/sessions/rollout-*.jsonl`
- **Models** (`models/`) — `Session`, `Message`, `LoadedSession`, `Agent` enum (Claude/Codex), `Role` enum (User/Assistant)
- **TUI** (`tui/`) — ratatui + crossterm, Elm-style MVU pattern:
  - `app.rs` — state + `Msg` enum + update logic, fuzzy search via `nucleo-matcher`
  - `picker.rs` — session list view
  - `conversation.rs` — message detail view
  - `input.rs` — keyboard event → `Msg` dispatch
- **Upload** (`upload/`) — `Uploader` trait, currently `DummyUploader` placeholder

### Adding a New Provider

Implement `SessionProvider` trait (discover + load_messages), add to the provider list in `main.rs`.
