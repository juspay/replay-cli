# CLI Integration with replay.md Backend

This document describes the changes needed in `replay-cli` to connect it to the `replay.md` backend.

---

## Overview

Two changes are needed:

1. **Replace `DummyUploader`** with a real HTTP uploader that POSTs sessions to the backend
2. **Update the default server URL** from `https://app.replay.dev` to `https://replay.md`

---

## Change 1: Real HTTP Uploader

### File: `src/upload/mod.rs`

Replace `DummyUploader` with `HttpUploader`. The `reqwest` dependency is already available (behind the `auth` feature flag) — it should be moved to a non-optional dependency, or the upload feature should also be gated behind `auth`.

**Recommended approach:** Make `reqwest` non-optional since upload requires auth anyway, and `auth` is the practical default feature.

### Upload Endpoint

```
POST {server_url}/api/threads
```

### Required Headers

```
Authorization: Bearer {token}
Content-Type: application/json
X-Replay-Client: cli
```

The `X-Replay-Client: cli` header is **required** — the server rejects uploads without it (returns 403).

The `token` comes from `auth::get_token()` which reads `~/.config/replay/credentials.json`.

### Request Body (JSON) — Structured Format (preferred)

The server accepts structured content blocks. See `CLI_DATA_SPEC.md` for the full specification of what data is available in the JSONL files.

```json
{
  "session": {
    "id": "string",
    "agent": "claude" | "codex",
    "timestamp": "ISO 8601 string",
    "project_path": "string or omit",
    "git_branch": "string or omit",
    "title": "string or omit",
    "cli_version": "string or omit",
    "model": "string or omit"
  },
  "messages": [
    {
      "role": "user" | "assistant",
      "timestamp": "ISO 8601 string",
      "content": [
        { "type": "text", "text": "..." },
        { "type": "thinking", "thinking": "..." },
        { "type": "tool_use", "id": "...", "name": "Edit", "input": { "file_path": "...", "old_string": "...", "new_string": "..." } },
        { "type": "tool_result", "tool_use_id": "...", "content": "..." }
      ],
      "model": "claude-opus-4-6",
      "stop_reason": "end_turn",
      "usage": { "input_tokens": 1234, "output_tokens": 567 }
    }
  ]
}
```

### Request Body (JSON) — Legacy Format (still supported)

The server also accepts the flat text format for backward compatibility:

```json
{
  "session": {
    "id": "string",
    "agent": "claude" | "codex",
    "timestamp": "ISO 8601 string",
    "project_path": "string or omit",
    "git_branch": "string or omit"
  },
  "messages": [
    {
      "role": "user" | "assistant",
      "text": "message content",
      "timestamp": "ISO 8601 string"
    }
  ]
}
```

### Field Mapping from Rust Models

| Rust field                    | JSON field               | Notes                                      |
|-------------------------------|--------------------------|--------------------------------------------|
| `session.id`                  | `session.id`             | Direct                                     |
| `session.agent`               | `session.agent`          | Use `Display` impl: `"claude"` or `"codex"`|
| `session.timestamp`           | `session.timestamp`      | `chrono::DateTime<Utc>` → `.to_rfc3339()`  |
| `session.project_path`        | `session.project_path`   | `Option<String>` — omit if `None`          |
| `session.git_branch`          | `session.git_branch`     | `Option<String>` — omit if `None`          |
| `message.role`                | `messages[].role`        | `"user"` or `"assistant"`                  |
| `message.text`                | `messages[].text`        | Direct                                     |
| `message.timestamp`           | `messages[].timestamp`   | `.to_rfc3339()`                            |

### Response: 201 Created (Success)

```json
{
  "id": "uuid",
  "slug": "aB3kX9mQ2r",
  "url": "https://replay.md/t/aB3kX9mQ2r",
  "visibility": "private",
  "message_count": 42
}
```

Print the URL to stderr so the user can open it.

### Response: 409 Conflict (Duplicate)

```json
{
  "error": "Session already uploaded",
  "slug": "aB3kX9mQ2r",
  "url": "https://replay.md/t/aB3kX9mQ2r"
}
```

This means the same `session.id` was already uploaded by this user. Print the existing URL.

### Response: 401 Unauthorized

User is not logged in or token expired. Print: `"Not authenticated. Run 'replay login' first."`

### Response: 403 Forbidden

Missing `X-Replay-Client: cli` header. This shouldn't happen if the header is set correctly.

### Response: 400 Bad Request

```json
{
  "error": "Invalid request body",
  "details": [...]
}
```

Validation failed. Print the error message.

---

### Implementation: `HttpUploader`

```rust
// src/upload/mod.rs

use anyhow::{anyhow, Result};
use serde::Serialize;

use crate::models::{LoadedSession, Role};

pub trait Uploader {
    fn upload(&self, session: &LoadedSession) -> Result<()>;
}

#[derive(Serialize)]
struct UploadRequest {
    session: SessionPayload,
    messages: Vec<MessagePayload>,
}

#[derive(Serialize)]
struct SessionPayload {
    id: String,
    agent: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    git_branch: Option<String>,
}

#[derive(Serialize)]
struct MessagePayload {
    role: String,
    text: String,
    timestamp: String,
}

pub struct HttpUploader {
    pub server_url: String,
    pub token: String,
}

impl Uploader for HttpUploader {
    fn upload(&self, loaded: &LoadedSession) -> Result<()> {
        let payload = UploadRequest {
            session: SessionPayload {
                id: loaded.session.id.clone(),
                agent: loaded.session.agent.to_string(), // "claude" or "codex"
                timestamp: loaded.session.timestamp.to_rfc3339(),
                project_path: loaded.session.project_path.clone(),
                git_branch: loaded.session.git_branch.clone(),
            },
            messages: loaded
                .messages
                .iter()
                .map(|m| MessagePayload {
                    role: match m.role {
                        Role::User => "user".to_string(),
                        Role::Assistant => "assistant".to_string(),
                    },
                    text: m.text.clone(),
                    timestamp: m.timestamp.to_rfc3339(),
                })
                .collect(),
        };

        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(format!("{}/api/threads", self.server_url))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Replay-Client", "cli")
            .json(&payload)
            .send()?;

        match resp.status().as_u16() {
            201 => {
                let body: serde_json::Value = resp.json()?;
                let url = body["url"].as_str().unwrap_or("(unknown URL)");
                eprintln!("Uploaded successfully!");
                eprintln!("  {url}");
                eprintln!("  visibility: private (change at {url})");
                Ok(())
            }
            409 => {
                let body: serde_json::Value = resp.json()?;
                let url = body["url"].as_str().unwrap_or("(unknown URL)");
                eprintln!("Session already uploaded: {url}");
                Ok(())
            }
            401 => Err(anyhow!(
                "Not authenticated. Run `replay login` first."
            )),
            403 => Err(anyhow!("Upload rejected by server.")),
            400 => {
                let body: serde_json::Value = resp.json()?;
                let error = body["error"].as_str().unwrap_or("Invalid request");
                Err(anyhow!("Upload failed: {error}"))
            }
            status => Err(anyhow!("Unexpected response: HTTP {status}")),
        }
    }
}
```

---

### Implementation: Wire up in `main.rs`

Replace `DummyUploader` usage with `HttpUploader` in both `run_browse` and `run_upload`:

```rust
// In run_upload():
use crate::auth;

fn run_upload(session_id: &str) -> Result<()> {
    let token = auth::get_token()?
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run `replay login` first."))?;
    let server_url = auth::config::server_url(); // Need to make config public

    let providers = make_providers();
    let sessions = discover_all(&providers);

    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

    for p in &providers {
        if let Ok(messages) = p.load_messages(session) {
            let loaded = LoadedSession {
                session: session.clone(),
                messages,
            };
            let uploader = upload::HttpUploader { server_url, token };
            uploader.upload(&loaded)?;
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "Could not load messages for session '{}'",
        session_id
    ))
}
```

**Note:** `auth::config` is currently private (`mod config` in `auth/mod.rs`). Either:
- Make `config` module `pub(crate)`, or
- Re-export `server_url` from `auth/mod.rs`: `pub use config::server_url;`

Similarly for `run_browse`, pass `HttpUploader` to `App::new()` instead of `DummyUploader`.

---

## Change 2: Update Default Server URL

### File: `src/auth/config.rs`

```rust
const DEFAULT_SERVER_URL: &str = "https://replay.md";
```

Change from `"https://app.replay.dev"` to `"https://replay.md"`.

The `REPLAY_SERVER_URL` env var override still works for local development:
```
REPLAY_SERVER_URL=http://localhost:3000 replay login
```

---

## Change 3: Make `reqwest` non-optional

### File: `Cargo.toml`

Move `reqwest` from optional to required (upload needs it regardless of auth feature):

```toml
[dependencies]
reqwest = { version = "0.12", features = ["blocking", "json"] }
```

Remove `"dep:reqwest"` from the `auth` feature list.

Alternatively, gate upload behind the `auth` feature too (since upload requires a token). This is the simpler path — just keep everything as-is and ensure the `auth` feature is enabled by default:

```toml
[features]
default = ["auth"]
auth = ["dep:tiny_http", "dep:reqwest", "dep:open", "dep:rand"]
```

---

## Change 4: Make `auth::config` accessible

### File: `src/auth/mod.rs`

Change `mod config;` to `pub(crate) mod config;` so `main.rs` can call `auth::config::server_url()`.

Or add a re-export:
```rust
pub use config::server_url;
```

---

## Testing

1. **Login:** `REPLAY_SERVER_URL=http://localhost:3000 replay login` → browser opens → authenticate → "Logged in successfully."
2. **Whoami:** `REPLAY_SERVER_URL=http://localhost:3000 replay whoami` → "Logged in as {name} ({email})"
3. **Upload:** `REPLAY_SERVER_URL=http://localhost:3000 replay upload {session_id}` → "Uploaded successfully!" with URL
4. **Duplicate:** Run the same upload again → "Session already uploaded: {url}"
5. **No auth:** Delete `~/.config/replay/credentials.json`, then upload → "Not logged in."

---

## Summary of Files to Change

| File                    | Change                                              |
|-------------------------|-----------------------------------------------------|
| `src/upload/mod.rs`     | Add `HttpUploader`, keep `Uploader` trait            |
| `src/main.rs`           | Use `HttpUploader` instead of `DummyUploader`        |
| `src/auth/mod.rs`       | Make `config` module `pub(crate)` or re-export       |
| `src/auth/config.rs`    | Change default URL to `https://replay.md`            |
| `Cargo.toml`            | Make `auth` feature default, or make reqwest required|
