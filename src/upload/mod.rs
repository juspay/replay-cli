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
    pub server_url: String,
    pub token: String,
}

impl HttpUploader {
    fn do_upload(&self, loaded: &LoadedSession) -> Result<()> {
        let payload = UploadRequest {
            session: SessionPayload {
                id: loaded.session.id.clone(),
                agent: loaded.session.agent.to_string(),
                timestamp: loaded.session.timestamp.to_rfc3339_opts(SecondsFormat::Millis, true),
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
            401 => Err(anyhow!(
                "Not authenticated. Run `replay login` first."
            )),
            403 => Err(anyhow!("Upload rejected by server.")),
            400 => {
                let body: serde_json::Value = resp.json()?;
                let error = body["error"].as_str().unwrap_or("Invalid request");
                let details = body.get("details").map(|d| d.to_string()).unwrap_or_default();
                Err(anyhow!("Upload failed: {error}\n{details}"))
            }
            status => Err(anyhow!("Unexpected response: HTTP {status}")),
        }
    }
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
