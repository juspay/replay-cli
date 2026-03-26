pub mod claude;
pub mod codex;

use anyhow::{anyhow, Result};

use crate::models::{Agent, Message, Session};

pub trait SessionProvider {
    /// Agent family this provider owns.
    fn agent(&self) -> Agent;

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

/// Load messages using the provider that owns the session's agent.
pub fn load_session_messages(
    providers: &[Box<dyn SessionProvider>],
    session: &Session,
) -> Result<Vec<Message>> {
    let provider = providers
        .iter()
        .find(|provider| provider.agent() == session.agent)
        .ok_or_else(|| anyhow!("No provider registered for {} sessions", session.agent))?;

    let messages = provider.load_messages(session)?;
    if messages.is_empty() {
        return Err(anyhow!(
            "Session '{}' has no exportable messages",
            session.id
        ));
    }

    Ok(messages)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::path::PathBuf;
    use std::rc::Rc;

    use chrono::Utc;

    use super::*;
    use crate::models::{ContentBlock, Role};

    #[derive(Clone)]
    enum Behavior {
        Messages(Vec<Message>),
        Error(&'static str),
    }

    struct DummyProvider {
        agent: Agent,
        behavior: Behavior,
        calls: Rc<Cell<usize>>,
    }

    impl SessionProvider for DummyProvider {
        fn agent(&self) -> Agent {
            self.agent
        }

        fn discover(&self) -> Result<Vec<Session>> {
            Ok(Vec::new())
        }

        fn load_messages(&self, _session: &Session) -> Result<Vec<Message>> {
            self.calls.set(self.calls.get() + 1);
            match &self.behavior {
                Behavior::Messages(messages) => Ok(messages.clone()),
                Behavior::Error(message) => Err(anyhow!(*message)),
            }
        }
    }

    fn sample_session(agent: Agent) -> Session {
        Session {
            id: format!("{agent}-session"),
            agent,
            timestamp: Utc::now(),
            project_path: None,
            git_branch: None,
            first_user_message: None,
            source_path: PathBuf::from("rollout.jsonl"),
            title: None,
            cli_version: None,
            model: None,
        }
    }

    fn sample_message(role: Role, text: &str) -> Message {
        Message {
            role,
            content: vec![ContentBlock::Text {
                text: text.to_string(),
            }],
            timestamp: Utc::now(),
            model: None,
            stop_reason: None,
            usage: None,
        }
    }

    #[test]
    fn loads_messages_from_matching_provider_only() {
        let claude_calls = Rc::new(Cell::new(0));
        let codex_calls = Rc::new(Cell::new(0));
        let providers: Vec<Box<dyn SessionProvider>> = vec![
            Box::new(DummyProvider {
                agent: Agent::Claude,
                behavior: Behavior::Messages(Vec::new()),
                calls: claude_calls.clone(),
            }),
            Box::new(DummyProvider {
                agent: Agent::Codex,
                behavior: Behavior::Messages(vec![sample_message(Role::Assistant, "done")]),
                calls: codex_calls.clone(),
            }),
        ];

        let messages = load_session_messages(&providers, &sample_session(Agent::Codex)).unwrap();

        assert_eq!(messages.len(), 1);
        assert_eq!(claude_calls.get(), 0);
        assert_eq!(codex_calls.get(), 1);
    }

    #[test]
    fn rejects_sessions_with_no_exportable_messages() {
        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(DummyProvider {
            agent: Agent::Codex,
            behavior: Behavior::Messages(Vec::new()),
            calls: Rc::new(Cell::new(0)),
        })];

        let err = load_session_messages(&providers, &sample_session(Agent::Codex)).unwrap_err();

        assert!(err.to_string().contains("has no exportable messages"));
    }

    #[test]
    fn surfaces_provider_errors_from_matching_provider() {
        let providers: Vec<Box<dyn SessionProvider>> = vec![Box::new(DummyProvider {
            agent: Agent::Codex,
            behavior: Behavior::Error("bad rollout"),
            calls: Rc::new(Cell::new(0)),
        })];

        let err = load_session_messages(&providers, &sample_session(Agent::Codex)).unwrap_err();

        assert!(err.to_string().contains("bad rollout"));
    }
}
