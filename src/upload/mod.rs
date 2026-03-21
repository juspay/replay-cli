use anyhow::Result;

use crate::models::LoadedSession;

pub trait Uploader {
    fn upload(&self, session: &LoadedSession) -> Result<()>;
}

pub struct DummyUploader;

impl Uploader for DummyUploader {
    fn upload(&self, session: &LoadedSession) -> Result<()> {
        eprintln!(
            "UPLOAD {} {} ({} messages)",
            session.session.agent.label(),
            session.session.id,
            session.messages.len(),
        );
        eprintln!("  path: {:?}", session.session.source_path);
        if let Some(ref proj) = session.session.project_path {
            eprintln!("  project: {}", proj);
        }
        for msg in &session.messages {
            let prefix = match msg.role {
                crate::models::Role::User => "USER",
                crate::models::Role::Assistant => "ASST",
            };
            let preview: String = msg.text.chars().take(80).collect();
            let ellipsis = if msg.text.len() > 80 { "..." } else { "" };
            eprintln!("  [{prefix}] {preview}{ellipsis}");
        }
        Ok(())
    }
}
