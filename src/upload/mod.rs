use anyhow::{anyhow, Result};
use chrono::SecondsFormat;
use serde::Serialize;

use crate::models::{ContentBlock, LoadedSession, Role, TokenUsage};

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
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cli_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
}

#[derive(Serialize)]
struct MessagePayload {
    role: Role,
    timestamp: String,
    content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stop_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    usage: Option<TokenUsage>,
}

pub struct HttpUploader {
    pub server_url: &'static str,
    pub token: String,
}

impl HttpUploader {
    fn do_upload(&self, loaded: &LoadedSession) -> Result<()> {
        ensure_uploadable(loaded)?;

        let payload = UploadRequest {
            session: SessionPayload {
                id: loaded.session.id.clone(),
                agent: loaded.session.agent.to_string(),
                timestamp: loaded
                    .session
                    .timestamp
                    .to_rfc3339_opts(SecondsFormat::Millis, true),
                project_path: loaded.session.project_path.clone(),
                git_branch: loaded.session.git_branch.clone(),
                title: loaded.session.title.clone(),
                cli_version: loaded.session.cli_version.clone(),
                model: loaded.session.model.clone(),
            },
            messages: loaded
                .messages
                .iter()
                .map(|m| MessagePayload {
                    role: m.role,
                    timestamp: m.timestamp.to_rfc3339_opts(SecondsFormat::Millis, true),
                    content: m.content.clone(),
                    model: m.model.clone(),
                    stop_reason: m.stop_reason.clone(),
                    usage: m.usage.clone(),
                })
                .collect(),
        };

        let client = reqwest::blocking::Client::new();
        let resp = client
            .post(format!("{}/api/threads", self.server_url))
            .header("Authorization", format!("Bearer {}", self.token))
            .header("X-Replay-Client", "cli")
            .header("X-Client-Version", env!("CARGO_PKG_VERSION"))
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
                let slug = body["slug"]
                    .as_str()
                    .ok_or_else(|| anyhow!("Server returned 409 without slug"))?;
                // Re-upload: PUT the full payload to update the existing thread
                let put_resp = client
                    .put(format!("{}/api/threads/{}", self.server_url, slug))
                    .header("Authorization", format!("Bearer {}", self.token))
                    .header("X-Replay-Client", "cli")
                    .header("X-Client-Version", env!("CARGO_PKG_VERSION"))
                    .json(&payload)
                    .send()?;
                if put_resp.status().is_success() {
                    let put_body: serde_json::Value = put_resp.json()?;
                    let url = put_body["url"].as_str().unwrap_or("(unknown URL)");
                    eprintln!("Updated successfully!");
                    eprintln!("  {url}");
                    Ok(())
                } else {
                    let status = put_resp.status().as_u16();
                    let err_body: serde_json::Value = put_resp.json().unwrap_or_default();
                    let error = err_body["error"].as_str().unwrap_or("Update failed");
                    Err(anyhow!("Re-upload failed (HTTP {status}): {error}"))
                }
            }
            426 => {
                let body: serde_json::Value = resp.json().unwrap_or_default();
                let message = body["message"]
                    .as_str()
                    .unwrap_or("Your CLI is outdated. Please update to continue.");
                Err(anyhow!("{message}"))
            }
            401 => Err(anyhow!("Not authenticated. Run `replay login` first.")),
            403 => Err(anyhow!("Upload rejected by server.")),
            400 => {
                let body: serde_json::Value = resp.json()?;
                let error = body["error"].as_str().unwrap_or("Invalid request");
                let details = body
                    .get("details")
                    .map(|d| d.to_string())
                    .unwrap_or_default();
                Err(anyhow!("Upload failed: {error}\n{details}"))
            }
            status => Err(anyhow!("Unexpected response: HTTP {status}")),
        }
    }
}

fn ensure_uploadable(loaded: &LoadedSession) -> Result<()> {
    if loaded.messages.is_empty() {
        return Err(anyhow!(
            "Session '{}' has no exportable messages",
            loaded.session.id
        ));
    }

    Ok(())
}

impl Uploader for HttpUploader {
    fn upload(&self, session: &LoadedSession) -> Result<()> {
        self.do_upload(session)
    }
}

/// Uploader that resolves credentials lazily at upload time.
/// This allows the TUI to start without requiring auth upfront.
pub struct LazyUploader;

impl Uploader for LazyUploader {
    fn upload(&self, session: &LoadedSession) -> Result<()> {
        let token = crate::auth::get_token()?
            .ok_or_else(|| anyhow!("Not logged in. Run `replay login` first."))?;
        let server_url = crate::auth::config::server_url();
        let uploader = HttpUploader { server_url, token };
        uploader.do_upload(session)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use chrono::Utc;

    use super::*;
    use crate::models::{Agent, Message, Session};

    fn sample_loaded_session(messages: Vec<Message>) -> LoadedSession {
        LoadedSession {
            session: Session {
                id: "session-1".to_string(),
                agent: Agent::Codex,
                timestamp: Utc::now(),
                project_path: None,
                git_branch: None,
                first_user_message: None,
                source_path: PathBuf::from("rollout.jsonl"),
                title: None,
                cli_version: None,
                model: None,
            },
            messages,
        }
    }

    fn sample_message() -> Message {
        Message {
            role: Role::Assistant,
            content: vec![ContentBlock::Text {
                text: "done".to_string(),
            }],
            timestamp: Utc::now(),
            model: None,
            stop_reason: None,
            usage: None,
        }
    }

    #[test]
    fn rejects_empty_uploads_before_request_construction() {
        let err = ensure_uploadable(&sample_loaded_session(Vec::new())).unwrap_err();

        assert!(err.to_string().contains("has no exportable messages"));
    }

    #[test]
    fn allows_non_empty_uploads() {
        ensure_uploadable(&sample_loaded_session(vec![sample_message()])).unwrap();
    }
}
