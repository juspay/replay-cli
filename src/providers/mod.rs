pub mod claude;
pub mod codex;

use anyhow::Result;

use crate::models::{Message, Session};

pub trait SessionProvider {
    /// Discover sessions (metadata only, no full message parsing).
    fn discover(&self) -> Result<Vec<Session>>;

    /// Load full messages for a given session.
    fn load_messages(&self, session: &Session) -> Result<Vec<Message>>;
}

/// Discover sessions from all available providers, sorted newest-first.
pub fn discover_all(providers: &[Box<dyn SessionProvider>]) -> Vec<Session> {
    let mut sessions: Vec<Session> = providers
        .iter()
        .flat_map(|p| p.discover().unwrap_or_default())
        .collect();
    sessions.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    sessions
}
