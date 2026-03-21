use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use super::SessionProvider;
use crate::models::{Agent, Message, Role, Session};

pub struct ClaudeProvider;

impl ClaudeProvider {
    /// Recursively collect *.jsonl files under `dir`, skipping subagents/ and tool-results/.
    fn collect_jsonl_files(dir: &PathBuf, out: &mut Vec<PathBuf>) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if name_str == "subagents" || name_str == "tool-results" {
                    continue;
                }
                Self::collect_jsonl_files(&path, out);
            } else if path.extension().map_or(false, |e| e == "jsonl") {
                out.push(path);
            }
        }
    }

    /// Extract text from a user message content field.
    /// Content can be a plain String or an Array of objects (with type "text"/"tool_result").
    fn extract_user_text(content: &Value) -> Option<String> {
        match content {
            Value::String(s) => Some(s.clone()),
            Value::Array(arr) => {
                let texts: Vec<&str> = arr
                    .iter()
                    .filter_map(|entry| {
                        let t = entry.get("type")?.as_str()?;
                        if t == "text" {
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
            _ => None,
        }
    }

    /// Extract text from an assistant message content array.
    /// Collect entries where type == "text", skip "thinking", "tool_use", "tool_result".
    fn extract_assistant_text(content: &Value) -> Option<String> {
        let arr = content.as_array()?;
        let texts: Vec<&str> = arr
            .iter()
            .filter_map(|entry| {
                let t = entry.get("type")?.as_str()?;
                if t == "text" {
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
}

impl SessionProvider for ClaudeProvider {
    fn discover(&self) -> Result<Vec<Session>> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
        let projects_dir = home.join(".claude").join("projects");
        if !projects_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut jsonl_files = Vec::new();
        Self::collect_jsonl_files(&projects_dir, &mut jsonl_files);

        let mut sessions = Vec::new();

        for path in jsonl_files {
            let file = match File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let reader = BufReader::new(file);

            let mut session_id: Option<String> = None;
            let mut cwd: Option<String> = None;
            let mut git_branch: Option<String> = None;
            let mut first_user_msg: Option<String> = None;
            let mut earliest_ts: Option<DateTime<Utc>> = None;

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

                // Track earliest timestamp across all lines
                if let Some(ts_str) = val.get("timestamp").and_then(|v| v.as_str()) {
                    if let Ok(ts) = ts_str.parse::<DateTime<Utc>>() {
                        if earliest_ts.is_none() || ts < earliest_ts.unwrap() {
                            earliest_ts = Some(ts);
                        }
                    }
                }

                let line_type = match val.get("type").and_then(|v| v.as_str()) {
                    Some(t) => t,
                    None => continue,
                };

                if line_type == "user" && first_user_msg.is_none() {
                    if session_id.is_none() {
                        session_id =
                            val.get("sessionId").and_then(|v| v.as_str()).map(String::from);
                    }
                    if cwd.is_none() {
                        cwd = val.get("cwd").and_then(|v| v.as_str()).map(String::from);
                    }
                    if git_branch.is_none() {
                        git_branch = val
                            .get("gitBranch")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                    }

                    if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
                        first_user_msg = Self::extract_user_text(content);
                    }
                }

                // Once we have what we need, stop scanning
                if session_id.is_some() && first_user_msg.is_some() && earliest_ts.is_some() {
                    break;
                }
            }

            let id = session_id.unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });

            let timestamp = earliest_ts.unwrap_or_else(Utc::now);

            sessions.push(Session {
                id,
                agent: Agent::Claude,
                timestamp,
                project_path: cwd,
                git_branch,
                first_user_message: first_user_msg,
                source_path: path,
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

            let timestamp = val
                .get("timestamp")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .unwrap_or_else(Utc::now);

            match line_type {
                "user" => {
                    if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
                        // Skip tool_result messages (they have content as array with tool_result entries)
                        if let Value::Array(arr) = content {
                            let has_only_tool_results = arr.iter().all(|e| {
                                e.get("type").and_then(|t| t.as_str()) == Some("tool_result")
                            });
                            if has_only_tool_results {
                                continue;
                            }
                        }

                        if let Some(text) = Self::extract_user_text(content) {
                            if !text.is_empty() {
                                messages.push(Message {
                                    role: Role::User,
                                    text,
                                    timestamp,
                                });
                            }
                        }
                    }
                }
                "assistant" => {
                    if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
                        if let Some(text) = Self::extract_assistant_text(content) {
                            if !text.is_empty() {
                                messages.push(Message {
                                    role: Role::Assistant,
                                    text,
                                    timestamp,
                                });
                            }
                        }
                    }
                }
                _ => continue,
            }
        }

        Ok(messages)
    }
}
