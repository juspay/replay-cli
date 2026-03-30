use chrono::{DateTime, Utc};
use serde::Serialize;
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Claude,
    Codex,
}

impl Agent {
    pub fn label(&self) -> &'static str {
        match self {
            Agent::Claude => "[CLAUDE]",
            Agent::Codex => "[CODEX]",
        }
    }
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Agent::Claude => write!(f, "claude"),
            Agent::Codex => write!(f, "codex"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

/// A single content block within a message.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "thinking")]
    Thinking { thinking: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        /// Preserved as-is from the API: can be a JSON string or an array of
        /// sub-blocks (text, image, etc.).
        content: serde_json::Value,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
    #[serde(rename = "image")]
    Image {
        source: ImageSource,
    },
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageSource {
    /// e.g. "base64"
    #[serde(rename = "type")]
    pub source_type: String,
    /// e.g. "image/png"
    pub media_type: String,
    /// The base64-encoded image data.
    pub data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TokenUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_creation_input_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_read_input_tokens: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: Vec<ContentBlock>,
    pub timestamp: DateTime<Utc>,
    /// Model used (assistant messages only).
    pub model: Option<String>,
    /// Why the turn ended (assistant messages only).
    pub stop_reason: Option<String>,
    /// Token usage (assistant messages only).
    pub usage: Option<TokenUsage>,
}

impl Message {
    /// Extract display-friendly text from content blocks.
    /// Concatenates all Text blocks, ignoring thinking/tool_use/tool_result.
    pub fn display_text(&self) -> String {
        let texts: Vec<&str> = self
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        texts.join("\n")
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent: Agent,
    pub timestamp: DateTime<Utc>,
    pub project_path: Option<String>,
    pub git_branch: Option<String>,
    pub first_user_message: Option<String>,
    pub source_path: PathBuf,
    /// Session title (from custom-title or agent-name).
    pub title: Option<String>,
    /// CLI version that created this session.
    pub cli_version: Option<String>,
    /// Primary model used in this session.
    pub model: Option<String>,
}

#[derive(Debug, Clone)]
pub struct LoadedSession {
    pub session: Session,
    pub messages: Vec<Message>,
}

/// Format a timestamp as a human-readable relative time.
pub fn relative_time(ts: &DateTime<Utc>) -> String {
    let now = Utc::now();
    let delta = now.signed_duration_since(*ts);

    if delta.num_minutes() < 1 {
        "just now".to_string()
    } else if delta.num_minutes() < 60 {
        format!("{}m ago", delta.num_minutes())
    } else if delta.num_hours() < 24 {
        format!("{}h ago", delta.num_hours())
    } else if delta.num_days() < 7 {
        format!("{}d ago", delta.num_days())
    } else if delta.num_weeks() < 5 {
        format!("{}w ago", delta.num_weeks())
    } else {
        ts.format("%Y-%m-%d").to_string()
    }
}
