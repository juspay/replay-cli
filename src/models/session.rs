use chrono::{DateTime, Utc};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub text: String,
    pub timestamp: DateTime<Utc>,
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
