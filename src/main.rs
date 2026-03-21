mod cli;
mod models;
mod providers;
mod tui;
mod upload;

use anyhow::Result;
use clap::Parser;

use cli::{Cli, Command};
use models::LoadedSession;
use providers::claude::ClaudeProvider;
use providers::codex::CodexProvider;
use providers::{discover_all, SessionProvider};
use tui::app::App;
use upload::{DummyUploader, Uploader};

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None | Some(Command::Browse) => run_browse(),
        Some(Command::Upload { session_id }) => run_upload(&session_id),
    }
}

fn make_providers() -> Vec<Box<dyn SessionProvider>> {
    vec![
        Box::new(ClaudeProvider),
        Box::new(CodexProvider),
    ]
}

fn run_browse() -> Result<()> {
    let providers = make_providers();
    let sessions = discover_all(&providers);

    if sessions.is_empty() {
        eprintln!("No sessions found.");
        return Ok(());
    }

    let uploader = Box::new(DummyUploader);
    let app = App::new(sessions, providers, uploader);
    tui::run_tui(app)
}

fn run_upload(session_id: &str) -> Result<()> {
    let providers = make_providers();
    let sessions = discover_all(&providers);

    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

    // Load messages
    for p in &providers {
        if let Ok(messages) = p.load_messages(session) {
            let loaded = LoadedSession {
                session: session.clone(),
                messages,
            };
            let uploader = DummyUploader;
            uploader.upload(&loaded)?;
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "Could not load messages for session '{}'",
        session_id
    ))
}
