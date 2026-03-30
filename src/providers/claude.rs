use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use super::SessionProvider;
use crate::models::{Agent, ContentBlock, ImageSource, Message, Role, Session, TokenUsage};

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

    /// Extract plain text from a user message content field (for discovery preview only).
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

    /// Parse content blocks from the Anthropic Messages API format.
    fn parse_content_blocks(content: &Value) -> Vec<ContentBlock> {
        match content {
            Value::String(s) => vec![ContentBlock::Text { text: s.clone() }],
            Value::Array(arr) => arr
                .iter()
                .filter_map(|entry| {
                    let t = entry.get("type")?.as_str()?;
                    match t {
                        "text" => {
                            let text = entry.get("text")?.as_str()?.to_string();
                            Some(ContentBlock::Text { text })
                        }
                        "thinking" => {
                            let thinking = entry.get("thinking")?.as_str()?.to_string();
                            Some(ContentBlock::Thinking { thinking })
                        }
                        "tool_use" => {
                            let id = entry.get("id")?.as_str()?.to_string();
                            let name = entry.get("name")?.as_str()?.to_string();
                            let input = entry
                                .get("input")
                                .cloned()
                                .unwrap_or(Value::Object(Default::default()));
                            Some(ContentBlock::ToolUse { id, name, input })
                        }
                        "tool_result" => {
                            let tool_use_id = entry.get("tool_use_id")?.as_str()?.to_string();
                            let content = entry
                                .get("content")
                                .cloned()
                                .unwrap_or(Value::String(String::new()));
                            let is_error = entry
                                .get("is_error")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            Some(ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                is_error,
                            })
                        }
                        "image" => {
                            let source = entry.get("source")?;
                            let source_type = source
                                .get("type")
                                .and_then(|v| v.as_str())
                                .unwrap_or("base64")
                                .to_string();
                            let media_type = source
                                .get("media_type")
                                .and_then(|v| v.as_str())?
                                .to_string();
                            let data = source.get("data").and_then(|v| v.as_str())?.to_string();
                            Some(ContentBlock::Image {
                                source: ImageSource {
                                    source_type,
                                    media_type,
                                    data,
                                },
                            })
                        }
                        _ => None,
                    }
                })
                .collect(),
            _ => Vec::new(),
        }
    }

    /// Parse token usage from the message object.
    fn parse_usage(message: &Value) -> Option<TokenUsage> {
        let usage = message.get("usage")?;
        Some(TokenUsage {
            input_tokens: usage.get("input_tokens")?.as_u64()?,
            output_tokens: usage.get("output_tokens")?.as_u64()?,
            cache_creation_input_tokens: usage
                .get("cache_creation_input_tokens")
                .and_then(|v| v.as_u64()),
            cache_read_input_tokens: usage
                .get("cache_read_input_tokens")
                .and_then(|v| v.as_u64()),
        })
    }
}

impl SessionProvider for ClaudeProvider {
    fn agent(&self) -> Agent {
        Agent::Claude
    }

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
            let mut cli_version: Option<String> = None;
            let mut model: Option<String> = None;
            let mut title: Option<String> = None;
            let mut summary: Option<String> = None;

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

                match line_type {
                    "summary" => {
                        if summary.is_none() {
                            summary = val
                                .get("summary")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        }
                    }
                    "user" => {
                        if session_id.is_none() {
                            session_id = val
                                .get("sessionId")
                                .and_then(|v| v.as_str())
                                .map(String::from);
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
                        if cli_version.is_none() {
                            cli_version = val
                                .get("version")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        }
                        // Skip meta messages and messages starting with '<' for summary
                        if first_user_msg.is_none() {
                            let is_meta = val
                                .get("isMeta")
                                .and_then(|v| v.as_bool())
                                .unwrap_or(false);
                            if !is_meta {
                                if let Some(content) =
                                    val.get("message").and_then(|m| m.get("content"))
                                {
                                    if let Some(text) = Self::extract_user_text(content) {
                                        if !text.starts_with('<') {
                                            first_user_msg = Some(text);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    "assistant" => {
                        if model.is_none() {
                            model = val
                                .get("message")
                                .and_then(|m| m.get("model"))
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        }
                    }
                    "custom-title" => {
                        title = val
                            .get("customTitle")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                    }
                    "agent-name" => {
                        // Use agent-name as fallback title
                        if title.is_none() {
                            title = val
                                .get("agentName")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        }
                    }
                    _ => {}
                }
            }

            let id = session_id.unwrap_or_else(|| {
                path.file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string()
            });

            let timestamp = earliest_ts.unwrap_or_else(Utc::now);

            // Prefer summary over first user message for preview
            let preview = summary.or(first_user_msg);

            sessions.push(Session {
                id,
                agent: Agent::Claude,
                timestamp,
                project_path: cwd,
                git_branch,
                first_user_message: preview,
                source_path: path,
                title,
                cli_version,
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

            let timestamp = val
                .get("timestamp")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse::<DateTime<Utc>>().ok())
                .unwrap_or_else(Utc::now);

            match line_type {
                "user" => {
                    if let Some(content) = val.get("message").and_then(|m| m.get("content")) {
                        let blocks = Self::parse_content_blocks(content);
                        if blocks.is_empty() {
                            continue;
                        }
                        messages.push(Message {
                            role: Role::User,
                            content: blocks,
                            timestamp,
                            model: None,
                            stop_reason: None,
                            usage: None,
                        });
                    }
                }
                "assistant" => {
                    if let Some(msg_obj) = val.get("message") {
                        if let Some(content) = msg_obj.get("content") {
                            let blocks = Self::parse_content_blocks(content);
                            if blocks.is_empty() {
                                continue;
                            }
                            let model = msg_obj
                                .get("model")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            let stop_reason = msg_obj
                                .get("stop_reason")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                            let usage = Self::parse_usage(msg_obj);
                            messages.push(Message {
                                role: Role::Assistant,
                                content: blocks,
                                timestamp,
                                model,
                                stop_reason,
                                usage,
                            });
                        }
                    }
                }
                _ => continue,
            }
        }

        Ok(messages)
    }
}
