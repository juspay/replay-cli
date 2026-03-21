#[cfg(feature = "auth")]
mod auth;
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
use upload::Uploader;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None | Some(Command::Browse) => run_browse(),
        Some(Command::Upload { session_id }) => run_upload(&session_id),
        #[cfg(feature = "auth")]
        Some(Command::Login) => auth::login(),
        #[cfg(feature = "auth")]
        Some(Command::Logout) => auth::logout(),
        #[cfg(feature = "auth")]
        Some(Command::Whoami) => auth::whoami(),
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

    let uploader = Box::new(upload::LazyUploader);
    let app = App::new(sessions, providers, uploader);
    tui::run_tui(app)
}

fn run_upload(session_id: &str) -> Result<()> {
    let token = auth::get_token()?
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run `replay login` first."))?;
    let server_url = auth::config::server_url();
    let uploader = upload::HttpUploader { server_url, token };

    let providers = make_providers();
    let sessions = discover_all(&providers);

    let session = sessions
        .iter()
        .find(|s| s.id == session_id)
        .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

    for p in &providers {
        if let Ok(messages) = p.load_messages(session) {
            let loaded = LoadedSession {
                session: session.clone(),
                messages,
            };
            uploader.upload(&loaded)?;
            return Ok(());
        }
    }

    Err(anyhow::anyhow!(
        "Could not load messages for session '{}'",
        session_id
    ))
}
