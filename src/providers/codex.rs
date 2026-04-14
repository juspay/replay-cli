use anyhow::Result;
use chrono::{DateTime, Utc};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

use super::SessionProvider;
use crate::models::{Agent, ContentBlock, ImageSource, Message, Role, Session, TokenUsage};

pub struct CodexProvider;

#[derive(Clone)]
struct QueuedMessage {
    #[allow(dead_code)]
    sequence: usize,
    role: Role,
    content: Vec<ContentBlock>,
    timestamp: DateTime<Utc>,
    model: Option<String>,
    stop_reason: Option<String>,
    usage: Option<TokenUsage>,
    result_call_id: Option<String>,
    is_fallback_result: bool,
}

impl QueuedMessage {
    fn into_message(self) -> Message {
        Message {
            role: self.role,
            content: self.content,
            timestamp: self.timestamp,
            model: self.model,
            stop_reason: self.stop_reason,
            usage: self.usage,
        }
    }
}

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

    fn parse_timestamp(val: &Value) -> DateTime<Utc> {
        val.get("timestamp")
            .and_then(|v| v.as_str())
            .and_then(|s| s.parse::<DateTime<Utc>>().ok())
            .unwrap_or_else(Utc::now)
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

    fn is_setup_text(text: &str) -> bool {
        let trimmed = text.trim_start();
        trimmed.starts_with("<environment_context>")
            || trimmed.starts_with("<permissions instructions>")
            || trimmed.starts_with("<collaboration_mode>")
            || trimmed.starts_with("<sandbox_mode>")
            || trimmed.starts_with("<approval_policy>")
    }

    fn is_setup_message(content: &Value) -> bool {
        let arr = match content.as_array() {
            Some(arr) => arr,
            None => return false,
        };

        let mut saw_text = false;
        for entry in arr {
            let Some(kind) = entry.get("type").and_then(|v| v.as_str()) else {
                return false;
            };
            match kind {
                "input_text" | "output_text" => {
                    let Some(text) = entry.get("text").and_then(|v| v.as_str()) else {
                        return false;
                    };
                    if !Self::is_setup_text(text) {
                        return false;
                    }
                    saw_text = true;
                }
                _ => return false,
            }
        }

        saw_text
    }

    fn parse_data_url_image(image_url: &str) -> Option<ImageSource> {
        let (header, data) = image_url.split_once(',')?;
        let media_type = header
            .strip_prefix("data:")?
            .strip_suffix(";base64")?
            .to_string();

        Some(ImageSource {
            source_type: "base64".to_string(),
            media_type,
            data: data.to_string(),
        })
    }

    /// Parse content blocks from Codex message content.
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
                    "input_image" => {
                        let image_url = entry.get("image_url")?.as_str()?;
                        let source = Self::parse_data_url_image(image_url)?;
                        Some(ContentBlock::Image { source })
                    }
                    _ => None,
                }
            })
            .collect()
    }

    fn parse_json_string_or_raw(raw: &str) -> Value {
        serde_json::from_str(raw).unwrap_or_else(|_| Value::String(raw.to_string()))
    }

    fn parse_optional_json_string(raw: Option<&str>) -> Value {
        raw.map(Self::parse_json_string_or_raw)
            .unwrap_or_else(|| Value::Object(Default::default()))
    }

    fn command_value_to_display(value: &Value) -> Option<String> {
        match value {
            Value::String(text) => Some(text.clone()),
            Value::Array(parts) => {
                let strings: Vec<String> = parts
                    .iter()
                    .filter_map(|part| match part {
                        Value::String(text) => Some(text.clone()),
                        Value::Number(number) => Some(number.to_string()),
                        Value::Bool(flag) => Some(flag.to_string()),
                        _ => None,
                    })
                    .collect();

                if strings.is_empty() {
                    None
                } else if strings.len() >= 3
                    && matches!(strings[0].as_str(), "bash" | "sh" | "zsh")
                    && matches!(strings[1].as_str(), "-lc" | "-c")
                {
                    Some(strings[2].clone())
                } else {
                    Some(strings.join(" "))
                }
            }
            _ => None,
        }
    }

    fn normalize_shell_tool_input(input: Value) -> (String, Value) {
        let Value::Object(obj) = input else {
            return ("Bash".to_string(), input);
        };

        let mut normalized = Map::new();

        if let Some(command) = obj
            .get("cmd")
            .and_then(Self::command_value_to_display)
            .or_else(|| obj.get("command").and_then(Self::command_value_to_display))
        {
            normalized.insert("command".to_string(), Value::String(command));
        }

        if let Some(workdir) = obj
            .get("workdir")
            .or_else(|| obj.get("cwd"))
            .and_then(|value| value.as_str())
        {
            normalized.insert("workdir".to_string(), Value::String(workdir.to_string()));
        }

        if normalized.is_empty() {
            ("Bash".to_string(), Value::Object(obj))
        } else {
            ("Bash".to_string(), Value::Object(normalized))
        }
    }

    fn extract_apply_patch_file_paths(patch: &str) -> Vec<String> {
        patch
            .lines()
            .filter_map(|line| {
                line.strip_prefix("*** Update File: ")
                    .or_else(|| line.strip_prefix("*** Add File: "))
                    .or_else(|| line.strip_prefix("*** Delete File: "))
                    .map(|path| path.to_string())
            })
            .collect()
    }

    fn normalize_apply_patch_input(input: Value) -> (String, Value) {
        let Value::String(patch) = input else {
            return ("ApplyPatch".to_string(), input);
        };

        enum PatchOp {
            Add {
                file_path: String,
                content: String,
            },
            Update {
                file_path: String,
                old_string: String,
                new_string: String,
            },
        }

        struct EditHunk {
            old_lines: Vec<String>,
            new_lines: Vec<String>,
        }

        enum PendingPatch {
            Add {
                file_path: String,
                lines: Vec<String>,
            },
            Update {
                file_path: String,
                hunks: Vec<EditHunk>,
                current_hunk: Option<EditHunk>,
            },
        }

        impl PendingPatch {
            fn finish(self) -> Option<PatchOp> {
                match self {
                    PendingPatch::Add { file_path, lines } => Some(PatchOp::Add {
                        file_path,
                        content: lines.join("\n"),
                    }),
                    PendingPatch::Update {
                        file_path,
                        mut hunks,
                        current_hunk,
                    } => {
                        if let Some(hunk) = current_hunk {
                            hunks.push(hunk);
                        }
                        if hunks.len() != 1 {
                            return None;
                        }
                        let hunk = hunks.pop()?;
                        Some(PatchOp::Update {
                            file_path,
                            old_string: hunk.old_lines.join("\n"),
                            new_string: hunk.new_lines.join("\n"),
                        })
                    }
                }
            }
        }

        let mut ops = Vec::new();
        let mut current: Option<PendingPatch> = None;

        for line in patch.lines() {
            if line == "*** Begin Patch" || line == "*** End Patch" || line == "*** End of File" {
                continue;
            }

            if let Some(file_path) = line.strip_prefix("*** Add File: ") {
                if let Some(pending) = current.take() {
                    let Some(op) = pending.finish() else {
                        let files = Self::extract_apply_patch_file_paths(&patch);
                        return (
                            "ApplyPatch".to_string(),
                            json!({ "patch": patch, "files": files }),
                        );
                    };
                    ops.push(op);
                }
                current = Some(PendingPatch::Add {
                    file_path: file_path.to_string(),
                    lines: Vec::new(),
                });
                continue;
            }

            if let Some(file_path) = line.strip_prefix("*** Update File: ") {
                if let Some(pending) = current.take() {
                    let Some(op) = pending.finish() else {
                        let files = Self::extract_apply_patch_file_paths(&patch);
                        return (
                            "ApplyPatch".to_string(),
                            json!({ "patch": patch, "files": files }),
                        );
                    };
                    ops.push(op);
                }
                current = Some(PendingPatch::Update {
                    file_path: file_path.to_string(),
                    hunks: Vec::new(),
                    current_hunk: None,
                });
                continue;
            }

            if line.starts_with("*** Delete File: ") || line.starts_with("*** Move to: ") {
                let files = Self::extract_apply_patch_file_paths(&patch);
                return (
                    "ApplyPatch".to_string(),
                    json!({ "patch": patch, "files": files }),
                );
            }

            match current.as_mut() {
                Some(PendingPatch::Add { lines, .. }) => {
                    if let Some(content) = line.strip_prefix('+') {
                        lines.push(content.to_string());
                    }
                }
                Some(PendingPatch::Update {
                    hunks,
                    current_hunk,
                    ..
                }) => {
                    if line.starts_with("@@") {
                        if let Some(hunk) = current_hunk.take() {
                            hunks.push(hunk);
                        }
                        *current_hunk = Some(EditHunk {
                            old_lines: Vec::new(),
                            new_lines: Vec::new(),
                        });
                        continue;
                    }

                    let Some(hunk) = current_hunk.as_mut() else {
                        continue;
                    };

                    if let Some(content) = line.strip_prefix(' ') {
                        let content = content.to_string();
                        hunk.old_lines.push(content.clone());
                        hunk.new_lines.push(content);
                    } else if let Some(content) = line.strip_prefix('-') {
                        hunk.old_lines.push(content.to_string());
                    } else if let Some(content) = line.strip_prefix('+') {
                        hunk.new_lines.push(content.to_string());
                    }
                }
                None => {}
            }
        }

        if let Some(pending) = current {
            let Some(op) = pending.finish() else {
                let files = Self::extract_apply_patch_file_paths(&patch);
                return (
                    "ApplyPatch".to_string(),
                    json!({ "patch": patch, "files": files }),
                );
            };
            ops.push(op);
        }

        if ops.len() != 1 {
            let files = Self::extract_apply_patch_file_paths(&patch);
            return (
                "ApplyPatch".to_string(),
                json!({ "patch": patch, "files": files }),
            );
        }

        match ops.pop().unwrap() {
            PatchOp::Add { file_path, content } => (
                "Write".to_string(),
                json!({
                    "file_path": file_path,
                    "content": content,
                }),
            ),
            PatchOp::Update {
                file_path,
                old_string,
                new_string,
            } => (
                "Edit".to_string(),
                json!({
                    "file_path": file_path,
                    "old_string": old_string,
                    "new_string": new_string,
                }),
            ),
        }
    }

    fn normalize_function_tool(name: &str, input: Value) -> (String, Value) {
        match name {
            "exec_command" | "shell" => Self::normalize_shell_tool_input(input),
            _ => (name.to_string(), input),
        }
    }

    fn normalize_custom_tool(name: &str, input: Value) -> (String, Value) {
        match name {
            "apply_patch" => Self::normalize_apply_patch_input(input),
            _ => (name.to_string(), input),
        }
    }

    fn parse_reasoning_summary(payload: &Value) -> Option<String> {
        let summary = payload.get("summary")?.as_array()?;
        let parts: Vec<&str> = summary
            .iter()
            .filter_map(|entry| {
                if entry.get("type").and_then(|v| v.as_str()) == Some("summary_text") {
                    entry.get("text").and_then(|v| v.as_str())
                } else {
                    None
                }
            })
            .collect();

        if parts.is_empty() {
            None
        } else {
            Some(parts.join("\n\n"))
        }
    }

    fn parse_last_token_usage(payload: &Value) -> Option<TokenUsage> {
        let usage = payload
            .get("info")
            .and_then(|info| info.get("last_token_usage"))?;

        Some(TokenUsage {
            input_tokens: usage.get("input_tokens")?.as_u64()?,
            output_tokens: usage.get("output_tokens")?.as_u64()?,
            cache_creation_input_tokens: None,
            cache_read_input_tokens: usage.get("cached_input_tokens").and_then(|v| v.as_u64()),
        })
    }

    fn create_tool_use_message(
        sequence: usize,
        role: Role,
        timestamp: DateTime<Utc>,
        model: Option<String>,
        id: String,
        name: String,
        input: Value,
    ) -> QueuedMessage {
        QueuedMessage {
            sequence,
            role,
            content: vec![ContentBlock::ToolUse { id, name, input }],
            timestamp,
            model,
            stop_reason: None,
            usage: None,
            result_call_id: None,
            is_fallback_result: false,
        }
    }

    fn create_tool_result_message(
        sequence: usize,
        timestamp: DateTime<Utc>,
        call_id: String,
        content: Value,
        is_error: bool,
        is_fallback_result: bool,
    ) -> QueuedMessage {
        QueuedMessage {
            sequence,
            role: Role::User,
            content: vec![ContentBlock::ToolResult {
                tool_use_id: call_id.clone(),
                content,
                is_error,
            }],
            timestamp,
            model: None,
            stop_reason: None,
            usage: None,
            result_call_id: Some(call_id),
            is_fallback_result,
        }
    }

    fn format_exec_command_output(payload: &Value) -> String {
        if let Some(output) = payload.get("aggregated_output").and_then(|v| v.as_str()) {
            return output.to_string();
        }

        let stdout = payload
            .get("stdout")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let stderr = payload
            .get("stderr")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        match (stdout.is_empty(), stderr.is_empty()) {
            (false, false) => format!("{stdout}\n{stderr}"),
            (false, true) => stdout.to_string(),
            (true, false) => stderr.to_string(),
            (true, true) => String::new(),
        }
    }

    fn parse_custom_tool_output(raw: &str) -> (Value, bool) {
        let parsed = Self::parse_json_string_or_raw(raw);

        match parsed {
            Value::Object(ref obj) => {
                let content = obj
                    .get("output")
                    .cloned()
                    .unwrap_or_else(|| Value::String(raw.to_string()));
                let is_error = obj
                    .get("metadata")
                    .and_then(|v| v.get("exit_code"))
                    .and_then(|v| v.as_i64())
                    .is_some_and(|code| code != 0);
                (content, is_error)
            }
            other => (other, false),
        }
    }

    fn format_web_search_result(payload: &Value) -> Value {
        let action = payload.get("action").cloned().unwrap_or(Value::Null);
        let query = payload.get("query").and_then(|v| v.as_str()).unwrap_or("");

        let text = match action.get("type").and_then(|v| v.as_str()) {
            Some("search") => {
                let queries = action
                    .get("queries")
                    .and_then(|v| v.as_array())
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|item| item.as_str())
                            .collect::<Vec<_>>()
                            .join("\n")
                    })
                    .filter(|queries| !queries.is_empty());
                queries.unwrap_or_else(|| query.to_string())
            }
            Some("open_page") => action
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or(query)
                .to_string(),
            Some("find_in_page") => {
                let pattern = action.get("pattern").and_then(|v| v.as_str()).unwrap_or("");
                let url = action.get("url").and_then(|v| v.as_str()).unwrap_or(query);
                if pattern.is_empty() {
                    url.to_string()
                } else if url.is_empty() {
                    pattern.to_string()
                } else {
                    format!("Find `{pattern}` in {url}")
                }
            }
            _ => serde_json::to_string(&action).unwrap_or_default(),
        };

        Value::String(text)
    }

    fn parse_web_search_call_id(payload: &Value) -> Option<String> {
        payload
            .get("call_id")
            .and_then(|v| v.as_str())
            .map(String::from)
    }
}

impl SessionProvider for CodexProvider {
    fn agent(&self) -> Agent {
        Agent::Codex
    }

    fn discover(&self) -> Result<Vec<Session>> {
        let home = dirs::home_dir().ok_or_else(|| anyhow::anyhow!("no home dir"))?;
        let sessions_dir = home.join(".codex").join("sessions");
        Self::discover_in_dir(&sessions_dir)
    }

    fn load_messages(&self, session: &Session) -> Result<Vec<Message>> {
        let file = File::open(&session.source_path)?;
        let reader = BufReader::new(file);

        let mut queued: Vec<QueuedMessage> = Vec::new();
        let mut sequence = 0usize;
        let mut current_turn_id: Option<String> = None;
        let mut turn_models: HashMap<String, String> = HashMap::new();
        let mut turn_usage: HashMap<String, TokenUsage> = HashMap::new();
        let mut last_assistant_text_by_turn: HashMap<String, usize> = HashMap::new();
        let mut canonical_results: HashSet<String> = HashSet::new();
        let mut pending_web_search_ids: HashMap<String, Vec<String>> = HashMap::new();
        let mut unresolved_web_search_indices: HashMap<String, Vec<usize>> = HashMap::new();

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

            let timestamp = Self::parse_timestamp(&val);

            match line_type {
                "turn_context" => {
                    if let Some(payload) = val.get("payload") {
                        current_turn_id = payload
                            .get("turn_id")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        if let (Some(turn_id), Some(model)) = (
                            current_turn_id.as_ref(),
                            payload.get("model").and_then(|v| v.as_str()),
                        ) {
                            turn_models.insert(turn_id.clone(), model.to_string());
                        }
                    }
                }
                "event_msg" => {
                    let Some(payload) = val.get("payload") else {
                        continue;
                    };
                    let payload_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match payload_type {
                        "task_started" => {
                            current_turn_id = payload
                                .get("turn_id")
                                .and_then(|v| v.as_str())
                                .map(String::from);
                        }
                        "task_complete" => {
                            if let Some(turn_id) = payload.get("turn_id").and_then(|v| v.as_str()) {
                                if let Some(&idx) = last_assistant_text_by_turn.get(turn_id) {
                                    if let Some(model) = turn_models.get(turn_id) {
                                        queued[idx].model = Some(model.clone());
                                    }
                                    if let Some(usage) = turn_usage.get(turn_id) {
                                        queued[idx].usage = Some(usage.clone());
                                    }
                                }
                            }
                        }
                        "token_count" => {
                            if let (Some(turn_id), Some(usage)) = (
                                current_turn_id.as_ref(),
                                Self::parse_last_token_usage(payload),
                            ) {
                                turn_usage.insert(turn_id.clone(), usage);
                            }
                        }
                        "exec_command_end" => {
                            let Some(call_id) = payload
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            else {
                                continue;
                            };

                            canonical_results.insert(call_id.clone());
                            sequence += 1;
                            queued.push(Self::create_tool_result_message(
                                sequence,
                                timestamp,
                                call_id,
                                Value::String(Self::format_exec_command_output(payload)),
                                payload
                                    .get("exit_code")
                                    .and_then(|v| v.as_i64())
                                    .is_some_and(|code| code != 0),
                                false,
                            ));
                        }
                        "web_search_end" => {
                            let Some(call_id) = payload
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            else {
                                continue;
                            };

                            let action_key = payload
                                .get("action")
                                .and_then(|action| serde_json::to_string(action).ok())
                                .unwrap_or_default();
                            pending_web_search_ids
                                .entry(action_key.clone())
                                .or_default()
                                .push(call_id.clone());

                            if let Some(indices) =
                                unresolved_web_search_indices.get_mut(&action_key)
                            {
                                if let Some(index) = indices.pop() {
                                    if let Some(ContentBlock::ToolUse { ref mut id, .. }) =
                                        queued[index].content.get_mut(0)
                                    {
                                        *id = call_id.clone();
                                    }
                                }
                            }

                            canonical_results.insert(call_id.clone());
                            sequence += 1;
                            queued.push(Self::create_tool_result_message(
                                sequence,
                                timestamp,
                                call_id,
                                Self::format_web_search_result(payload),
                                false,
                                false,
                            ));
                        }
                        _ => {}
                    }
                }
                "response_item" => {
                    let Some(payload) = val.get("payload") else {
                        continue;
                    };
                    let payload_type = payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match payload_type {
                        "message" => {
                            let role_str =
                                payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
                            let Some(content) = payload.get("content") else {
                                continue;
                            };

                            match role_str {
                                "user" => {
                                    if Self::is_setup_message(content) {
                                        continue;
                                    }
                                    let blocks = Self::parse_content_blocks(content);
                                    if blocks.is_empty() {
                                        continue;
                                    }
                                    sequence += 1;
                                    queued.push(QueuedMessage {
                                        sequence,
                                        role: Role::User,
                                        content: blocks,
                                        timestamp,
                                        model: None,
                                        stop_reason: None,
                                        usage: None,
                                        result_call_id: None,
                                        is_fallback_result: false,
                                    });
                                }
                                "assistant" => {
                                    let blocks = Self::parse_content_blocks(content);
                                    if blocks.is_empty() {
                                        continue;
                                    }
                                    sequence += 1;
                                    queued.push(QueuedMessage {
                                        sequence,
                                        role: Role::Assistant,
                                        content: blocks,
                                        timestamp,
                                        model: current_turn_id
                                            .as_ref()
                                            .and_then(|turn_id| turn_models.get(turn_id))
                                            .cloned(),
                                        stop_reason: None,
                                        usage: None,
                                        result_call_id: None,
                                        is_fallback_result: false,
                                    });
                                    if let Some(turn_id) = current_turn_id.as_ref() {
                                        last_assistant_text_by_turn
                                            .insert(turn_id.clone(), queued.len() - 1);
                                    }
                                }
                                _ => {}
                            }
                        }
                        "reasoning" => {
                            let Some(thinking) = Self::parse_reasoning_summary(payload) else {
                                continue;
                            };

                            sequence += 1;
                            queued.push(QueuedMessage {
                                sequence,
                                role: Role::Assistant,
                                content: vec![ContentBlock::Thinking { thinking }],
                                timestamp,
                                model: current_turn_id
                                    .as_ref()
                                    .and_then(|turn_id| turn_models.get(turn_id))
                                    .cloned(),
                                stop_reason: None,
                                usage: None,
                                result_call_id: None,
                                is_fallback_result: false,
                            });
                        }
                        "function_call" => {
                            let Some(call_id) = payload
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            else {
                                continue;
                            };
                            let raw_name = payload
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("tool")
                                .to_string();
                            let raw_input = Self::parse_optional_json_string(
                                payload.get("arguments").and_then(|v| v.as_str()),
                            );
                            let (name, input) = Self::normalize_function_tool(&raw_name, raw_input);

                            sequence += 1;
                            queued.push(Self::create_tool_use_message(
                                sequence,
                                Role::Assistant,
                                timestamp,
                                current_turn_id
                                    .as_ref()
                                    .and_then(|turn_id| turn_models.get(turn_id))
                                    .cloned(),
                                call_id,
                                name,
                                input,
                            ));
                        }
                        "function_call_output" => {
                            let Some(call_id) = payload
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            else {
                                continue;
                            };
                            let content = payload
                                .get("output")
                                .and_then(|v| v.as_str())
                                .map(|s| Value::String(s.to_string()))
                                .unwrap_or_else(|| Value::String(String::new()));

                            sequence += 1;
                            queued.push(Self::create_tool_result_message(
                                sequence, timestamp, call_id, content, false, true,
                            ));
                        }
                        "custom_tool_call" => {
                            let Some(call_id) = payload
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            else {
                                continue;
                            };
                            let raw_name = payload
                                .get("name")
                                .and_then(|v| v.as_str())
                                .unwrap_or("custom_tool")
                                .to_string();
                            let raw_input = match payload.get("input") {
                                Some(Value::String(raw)) => Self::parse_json_string_or_raw(raw),
                                Some(other) => other.clone(),
                                None => Value::Object(Default::default()),
                            };
                            let (name, input) = Self::normalize_custom_tool(&raw_name, raw_input);

                            sequence += 1;
                            queued.push(Self::create_tool_use_message(
                                sequence,
                                Role::Assistant,
                                timestamp,
                                current_turn_id
                                    .as_ref()
                                    .and_then(|turn_id| turn_models.get(turn_id))
                                    .cloned(),
                                call_id,
                                name,
                                input,
                            ));
                        }
                        "custom_tool_call_output" => {
                            let Some(call_id) = payload
                                .get("call_id")
                                .and_then(|v| v.as_str())
                                .map(String::from)
                            else {
                                continue;
                            };
                            let raw = payload
                                .get("output")
                                .and_then(|v| v.as_str())
                                .unwrap_or_default();
                            let (content, is_error) = Self::parse_custom_tool_output(raw);

                            canonical_results.insert(call_id.clone());
                            sequence += 1;
                            queued.push(Self::create_tool_result_message(
                                sequence, timestamp, call_id, content, is_error, false,
                            ));
                        }
                        "web_search_call" => {
                            let action = payload.get("action").cloned().unwrap_or(Value::Null);
                            let action_key =
                                serde_json::to_string(&action).unwrap_or_else(|_| String::new());
                            let call_id = if let Some(id) = Self::parse_web_search_call_id(payload)
                            {
                                id
                            } else if let Some(ids) = pending_web_search_ids.get_mut(&action_key) {
                                ids.pop()
                                    .unwrap_or_else(|| format!("web_search:{sequence}"))
                            } else {
                                format!("web_search:{sequence}")
                            };

                            sequence += 1;
                            queued.push(Self::create_tool_use_message(
                                sequence,
                                Role::Assistant,
                                timestamp,
                                current_turn_id
                                    .as_ref()
                                    .and_then(|turn_id| turn_models.get(turn_id))
                                    .cloned(),
                                call_id.clone(),
                                "WebSearch".to_string(),
                                action,
                            ));

                            if call_id.starts_with("web_search:") {
                                unresolved_web_search_indices
                                    .entry(action_key)
                                    .or_default()
                                    .push(queued.len() - 1);
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }

        let mut messages = Vec::new();
        for entry in queued.into_iter().filter(|entry| {
            if !entry.is_fallback_result {
                return true;
            }

            match entry.result_call_id.as_ref() {
                Some(call_id) => !canonical_results.contains(call_id),
                None => true,
            }
        }) {
            messages.push(entry.into_message());
        }

        Ok(messages)
    }
}

impl CodexProvider {
    fn discover_in_dir(sessions_dir: &PathBuf) -> Result<Vec<Session>> {
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
            let mut cli_version: Option<String> = None;

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
                        cwd = payload
                            .get("cwd")
                            .and_then(|v| v.as_str())
                            .map(String::from);
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
                            .or_else(|| payload.get("model_name"))
                            .and_then(|v| v.as_str())
                            .map(String::from);
                        cli_version = payload
                            .get("cli_version")
                            .and_then(|v| v.as_str())
                            .map(String::from);
                    }
                }

                if line_type == "response_item" && first_user_msg.is_none() {
                    if let Some(payload) = val.get("payload") {
                        let payload_type =
                            payload.get("type").and_then(|v| v.as_str()).unwrap_or("");
                        if payload_type != "message" {
                            continue;
                        }

                        let role = payload.get("role").and_then(|v| v.as_str()).unwrap_or("");
                        if role != "user" {
                            continue;
                        }

                        if let Some(content) = payload.get("content") {
                            if Self::is_setup_message(content) {
                                continue;
                            }
                            first_user_msg = Self::extract_content_text(content, "input_text");
                        }
                    }
                }
            }

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
                cli_version,
                model,
            });
        }

        Ok(sessions)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    fn write_rollout(lines: &[&str]) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("replay-codex-tests-{unique}"));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("rollout-test.jsonl");
        fs::write(&path, lines.join("\n")).unwrap();
        path
    }

    fn sample_session(path: PathBuf) -> Session {
        Session {
            id: "session-1".to_string(),
            agent: Agent::Codex,
            timestamp: "2026-04-01T00:00:00Z".parse().unwrap(),
            project_path: None,
            git_branch: None,
            first_user_message: None,
            source_path: path,
            title: None,
            cli_version: None,
            model: None,
        }
    }

    #[test]
    fn discovery_prefers_first_real_user_message_and_cli_version() {
        let root = std::env::temp_dir().join(format!(
            "replay-codex-discover-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let sessions_dir = root
            .join(".codex")
            .join("sessions")
            .join("2026")
            .join("04")
            .join("01");
        fs::create_dir_all(&sessions_dir).unwrap();
        let path = sessions_dir.join("rollout-test.jsonl");
        fs::write(&path, [
            r#"{"timestamp":"2026-04-01T00:00:00Z","type":"session_meta","payload":{"id":"session-1","timestamp":"2026-04-01T00:00:00Z","cwd":"/repo","cli_version":"0.104.0","git":{"branch":"main"}}}"#,
            r#"{"timestamp":"2026-04-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>\n..."},{"type":"input_text","text":"</permissions instructions>"}]}}"#,
            r#"{"timestamp":"2026-04-01T00:00:02Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>"}]}}"#,
            r#"{"timestamp":"2026-04-01T00:00:03Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Fix the upload parser"}]}}"#,
        ].join("\n")).unwrap();

        let sessions =
            CodexProvider::discover_in_dir(&root.join(".codex").join("sessions")).unwrap();
        let session = sessions
            .into_iter()
            .find(|session| session.source_path == path)
            .unwrap();

        assert_eq!(
            session.first_user_message.as_deref(),
            Some("Fix the upload parser")
        );
        assert_eq!(session.cli_version.as_deref(), Some("0.104.0"));
        assert_eq!(session.project_path.as_deref(), Some("/repo"));
        assert_eq!(session.git_branch.as_deref(), Some("main"));
    }

    #[test]
    fn loads_tool_calls_results_images_and_usage() {
        let path = write_rollout(&[
            r#"{"timestamp":"2026-04-01T00:00:00Z","type":"turn_context","payload":{"turn_id":"turn-1","model":"gpt-5.2-codex"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Please inspect this image"},{"type":"input_image","image_url":"data:image/png;base64,Zm9v"}]}}"#,
            r#"{"timestamp":"2026-04-01T00:00:02Z","type":"response_item","payload":{"type":"reasoning","summary":[{"type":"summary_text","text":"Inspecting project files"}],"content":null,"encrypted_content":"secret"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:03Z","type":"response_item","payload":{"type":"function_call","name":"exec_command","arguments":"{\"cmd\":\"ls\"}","call_id":"call-shell"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:04Z","type":"event_msg","payload":{"type":"exec_command_end","call_id":"call-shell","aggregated_output":"file1\nfile2\n","stdout":"","stderr":"","exit_code":0}}"#,
            r#"{"timestamp":"2026-04-01T00:00:05Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call-shell","output":"mirror output"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:06Z","type":"response_item","payload":{"type":"custom_tool_call","status":"completed","call_id":"call-patch","name":"apply_patch","input":"*** Begin Patch\n*** Update File: src/main.rs\n@@\n-old\n+new\n*** End Patch"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:07Z","type":"response_item","payload":{"type":"custom_tool_call_output","call_id":"call-patch","output":"{\"output\":\"Success. Updated the following files:\\nM src/main.rs\\n\",\"metadata\":{\"exit_code\":0,\"duration_seconds\":0.0}}"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:08Z","type":"event_msg","payload":{"type":"web_search_end","call_id":"ws-1","query":"Next.js docs","action":{"type":"search","query":"Next.js docs","queries":["Next.js docs","Next.js cache docs"]}}}"#,
            r#"{"timestamp":"2026-04-01T00:00:09Z","type":"response_item","payload":{"type":"web_search_call","status":"completed","action":{"type":"search","query":"Next.js docs","queries":["Next.js docs","Next.js cache docs"]}}}"#,
            r#"{"timestamp":"2026-04-01T00:00:10Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done."}]}}"#,
            r#"{"timestamp":"2026-04-01T00:00:11Z","type":"event_msg","payload":{"type":"token_count","info":{"last_token_usage":{"input_tokens":100,"cached_input_tokens":25,"output_tokens":40,"reasoning_output_tokens":5,"total_tokens":145}}}}"#,
            r#"{"timestamp":"2026-04-01T00:00:12Z","type":"event_msg","payload":{"type":"task_complete","turn_id":"turn-1"}}"#,
        ]);

        let provider = CodexProvider;
        let session = sample_session(path);
        let messages = provider.load_messages(&session).unwrap();

        assert_eq!(messages.len(), 9);

        assert!(matches!(messages[0].content[0], ContentBlock::Text { .. }));
        assert!(matches!(messages[0].content[1], ContentBlock::Image { .. }));

        assert!(matches!(
            messages[1].content[0],
            ContentBlock::Thinking { ref thinking } if thinking == "Inspecting project files"
        ));

        assert!(matches!(
            messages[2].content[0],
            ContentBlock::ToolUse { ref id, ref name, ref input }
                if id == "call-shell"
                    && name == "Bash"
                    && input.get("command").and_then(|v| v.as_str()) == Some("ls")
        ));

        assert!(matches!(
            messages[3].content[0],
            ContentBlock::ToolResult { ref tool_use_id, ref content, is_error }
                if tool_use_id == "call-shell" && content == &Value::String("file1\nfile2\n".to_string()) && !is_error
        ));

        assert!(matches!(
            messages[4].content[0],
            ContentBlock::ToolUse { ref id, ref name, ref input }
                if id == "call-patch"
                    && name == "Edit"
                    && input.get("file_path").and_then(|v| v.as_str()) == Some("src/main.rs")
                    && input.get("old_string").and_then(|v| v.as_str()) == Some("old")
                    && input.get("new_string").and_then(|v| v.as_str()) == Some("new")
        ));

        assert!(matches!(
            messages[5].content[0],
            ContentBlock::ToolResult { ref tool_use_id, ref content, is_error }
                if tool_use_id == "call-patch" && content == &Value::String("Success. Updated the following files:\nM src/main.rs\n".to_string()) && !is_error
        ));

        assert!(matches!(
            messages[6].content[0],
            ContentBlock::ToolResult { ref tool_use_id, ref content, is_error }
                if tool_use_id == "ws-1" && content == &Value::String("Next.js docs\nNext.js cache docs".to_string()) && !is_error
        ));

        assert!(matches!(
            messages[7].content[0],
            ContentBlock::ToolUse { ref id, ref name, .. }
                if id == "ws-1" && name == "WebSearch"
        ));

        assert!(
            matches!(messages[8].content[0], ContentBlock::Text { ref text } if text == "Done.")
        );
        assert_eq!(messages[8].model.as_deref(), Some("gpt-5.2-codex"));
        let usage = messages[8].usage.as_ref().unwrap();
        assert_eq!(usage.input_tokens, 100);
        assert_eq!(usage.output_tokens, 40);
        assert_eq!(usage.cache_read_input_tokens, Some(25));
    }

    #[test]
    fn skips_encrypted_only_reasoning_and_setup_messages() {
        let path = write_rollout(&[
            r#"{"timestamp":"2026-04-01T00:00:00Z","type":"response_item","payload":{"type":"message","role":"developer","content":[{"type":"input_text","text":"<permissions instructions>\n..."},{"type":"input_text","text":"</permissions instructions>"}]}}"#,
            r#"{"timestamp":"2026-04-01T00:00:01Z","type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n  <cwd>/repo</cwd>\n</environment_context>"}]}}"#,
            r#"{"timestamp":"2026-04-01T00:00:02Z","type":"response_item","payload":{"type":"reasoning","summary":[],"content":null,"encrypted_content":"secret"}}"#,
            r#"{"timestamp":"2026-04-01T00:00:03Z","type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Actual response"}]}}"#,
        ]);

        let provider = CodexProvider;
        let session = sample_session(path);
        let messages = provider.load_messages(&session).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(
            matches!(messages[0].content[0], ContentBlock::Text { ref text } if text == "Actual response")
        );
    }

    #[test]
    fn keeps_legacy_shell_tool_arguments() {
        let path = write_rollout(&[
            r#"{"timestamp":"2025-09-30T08:01:07.949Z","type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"],\"workdir\":\"/repo\"}","call_id":"call-shell"}}"#,
            r#"{"timestamp":"2025-09-30T08:01:08.000Z","type":"response_item","payload":{"type":"function_call_output","call_id":"call-shell","output":"Command output"}}"#,
        ]);

        let provider = CodexProvider;
        let session = sample_session(path);
        let messages = provider.load_messages(&session).unwrap();

        assert_eq!(messages.len(), 2);
        assert!(matches!(
            messages[0].content[0],
            ContentBlock::ToolUse { ref name, ref input, .. }
                if name == "Bash"
                    && input.get("command").and_then(|v| v.as_str()) == Some("ls")
                    && input.get("workdir").and_then(|v| v.as_str()) == Some("/repo")
        ));
        assert!(matches!(
            messages[1].content[0],
            ContentBlock::ToolResult { ref tool_use_id, ref content, .. }
                if tool_use_id == "call-shell" && content == &Value::String("Command output".to_string())
        ));
    }

    #[test]
    fn normalizes_add_file_patch_to_write_tool() {
        let path = write_rollout(&[
            r#"{"timestamp":"2026-04-01T00:00:00Z","type":"response_item","payload":{"type":"custom_tool_call","status":"completed","call_id":"call-write","name":"apply_patch","input":"*** Begin Patch\n*** Add File: src/new.rs\n+fn main() {\n+    println!(\"hi\");\n+}\n*** End Patch"}}"#,
        ]);

        let provider = CodexProvider;
        let session = sample_session(path);
        let messages = provider.load_messages(&session).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(
            messages[0].content[0],
            ContentBlock::ToolUse { ref id, ref name, ref input }
                if id == "call-write"
                    && name == "Write"
                    && input.get("file_path").and_then(|v| v.as_str()) == Some("src/new.rs")
                    && input.get("content").and_then(|v| v.as_str()) == Some("fn main() {\n    println!(\"hi\");\n}")
        ));
    }

    #[test]
    fn falls_back_to_apply_patch_for_multi_file_diffs() {
        let patch = "*** Begin Patch\n*** Update File: src/one.rs\n@@\n-old\n+new\n*** Update File: src/two.rs\n@@\n-old2\n+new2\n*** End Patch";
        let path = write_rollout(&[&format!(
            r#"{{"timestamp":"2026-04-01T00:00:00Z","type":"response_item","payload":{{"type":"custom_tool_call","status":"completed","call_id":"call-patch","name":"apply_patch","input":"{}"}}}}"#,
            patch.replace('\n', "\\n").replace('"', "\\\"")
        )]);

        let provider = CodexProvider;
        let session = sample_session(path);
        let messages = provider.load_messages(&session).unwrap();

        assert_eq!(messages.len(), 1);
        assert!(matches!(
            messages[0].content[0],
            ContentBlock::ToolUse { ref name, ref input, .. }
                if name == "ApplyPatch"
                    && input.get("patch").and_then(|v| v.as_str()) == Some(patch)
                    && input.get("files").and_then(|v| v.as_array()).is_some_and(|files| files.len() == 2)
        ));
    }
}
