use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use super::SessionProvider;
use crate::models::{Agent, ContentBlock, Message, Role, Session};

pub struct CodexProvider;

impl CodexProvider {
    /// Recursively collect rollout-*.jsonl files under `dir`.
    fn collect_rollout_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                Self::collect_rollout_files(&path, out);
            } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with("rollout-") && name.ends_with(".jsonl") {
                    out.push(path);
                }
            }
        }
    }

    /// Extract plain text from content array for a given type (discovery preview only).
    fn extract_content_text(content: &Value, content_type: &str) -> Option<String> {
        let arr = content.as_array()?;
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|entry| {
                let t = entry.get("type")?.as_str()?;
                if t == content_type {
                    entry.get("text")?.as_str()
                } else {
                    None
                }
            })
            .collect();
        if texts.is_empty() {
            None
        } else {
            Some(texts.join("\n"))
        }
    }

    /// Parse content blocks from Codex response_item format.
    fn parse_content_blocks(content: &Value) -> Vec<ContentBlock> {
        let arr = match content.as_array() {
            Some(a) => a,
            None => return Vec::new(),
        };

        arr.iter()
            .filter_map(|entry| {
                let t = entry.get("type")?.as_str()?;
                match t {
                    "input_text" | "output_text" => {
                        let text = entry.get("text")?.as_str()?.to_string();
                        if text.is_empty() {
                            None
                        } else {
                            Some(ContentBlock::Text { text })
                        }
                    }
                    _ => None,
                }
            })
            .collect()
    }
}

impl SessionProvider for CodexProvider {
    fn discover(&self) -> Result<Vec<Session>> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
        let sessions_dir = home.join(".codex").join("sessions");
        if !sessions_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut rollout_files = Vec::new();
        Self::collect_rollout_files(&sessions_dir, &mut rollout_files);

        let mut sessions = Vec::new();

        for path in rollout_files {
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let reader = BufReader::new(file);

            let mut session_id: Option<String> = None;
            let mut cwd: Option<String> = None;
            let mut git_branch: Option<String> = None;
            let mut session_ts: Option<DateTime<Utc>> = None;
            let mut first_user_msg: Option<String> = None;
            let mut model: Option<String> = None;

            for line in reader.lines() {
                let line = match line {
                    Ok(l) => l,
                    Err(_) => continue,
                };
                if line.is_empty() {
                    continue;
                }

                let val: Value = match serde_json::from_str(&line) {
                    Ok(v) => v,
                    Err(_) => continue,
                };

                let line_type = match val.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => continue,
                };

                if line_type == "session_meta" {
                    if let Some(payload) = val.get("payload") {
                        session_id = payload.get("id").and_then(|v| v.as_str()).map(String::from);
                        cwd = payload.get("cwd").and_then(|v| v.as_str()).map(String::from);
                        git_branch = payload
                            .get("git")
                            .and_then(|g| g.get("branch"))
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        session_ts = payload
                            .get("timestamp")
                            .and_then(|v| v.as_str())
                            .and_then(|s| s.parse::<DateTime<Utc>>().ok());
                        model = payload
                            .get("model")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                    }
                }

                if line_type == "response_item" && first_user_msg.is_none() {
                    if let Some(payload) = val.get("payload") {
                        let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
                        if role == "developer" || role == "user" {
                            if let Some(content) = payload.get("content") {
                                first_user_msg =
                                    Self::extract_content_text(content, "input_text");
                            }
                        }
                    }
                }

                if session_id.is_some() && first_user_msg.is_some() {
                    break;
                }
            }

            // Skip files without session_meta
            let id = match session_id {
                Some(id) => id,
                None => continue,
            };

            let timestamp = session_ts.unwrap_or_else(Utc::now);

            sessions.push(Session {
                id,
                agent: Agent::Codex,
                timestamp,
                project_path: cwd,
                git_branch,
                first_user_message: first_user_msg,
                source_path: path,
                title: None,
                cli_version: None,
                model,
            });
        }

        Ok(sessions)
    }

    fn load_messages(&self, session: &Session) -> Result<Vec<Message>> {
        let file = File::open(&session.source_path)?;
        let reader = BufReader::new(file);
        let mut messages = Vec::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            if line.is_empty() {
                continue;
            }

            let val: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let line_type = match val.get("type").and_then(|v| v.as_str()) {
                Some(t) => t,
                None => continue,
            };

            if line_type != "response_item" {
                continue;
            }

            let payload = match val.get("payload") {
                Some(p) => p,
                None => continue,
            };

            let payload_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
            if payload_type != "message" {
                continue;
            }

            let timestamp = val
                .get("timestamp")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .unwrap_or_else(Utc::now);

            let role_str = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
            let content = match payload.get("content") {
                Some(c) => c,
                None => continue,
            };

            let (role, blocks) = match role_str {
                "developer" | "user" => {
                    let blocks = Self::parse_content_blocks(content);
                    if blocks.is_empty() {
                        continue;
                    }
                    (Role::User, blocks)
                }
                "assistant" => {
                    let blocks = Self::parse_content_blocks(content);
                    if blocks.is_empty() {
                        continue;
                    }
                    (Role::Assistant, blocks)
                }
                _ => continue,
            };

            messages.push(Message {
                role,
                content: blocks,
                timestamp,
                model: None,
                stop_reason: None,
                usage: None,
            });
        }

        Ok(messages)
    }
}
