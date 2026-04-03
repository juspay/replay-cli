#[cfg(feature = "auth")]
mod auth;
mod cli;
mod models;
mod providers;
mod upload;

use anyhow::Result;
use clap::{CommandFactory, Parser};

use cli::{Cli, Command};
use models::LoadedSession;
use providers::claude::ClaudeProvider;
use providers::codex::CodexProvider;
use providers::{discover_all, load_session_messages, SessionProvider};
use upload::Uploader;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => run_list(),
        Some(Command::Upload { session_id }) => run_upload(&session_id),
        Some(Command::Completions { shell }) => {
            clap_complete::generate(shell, &mut Cli::command(), "replay", &mut std::io::stdout());
            Ok(())
        }
        #[cfg(feature = "auth")]
        Some(Command::Login) => auth::login(),
        #[cfg(feature = "auth")]
        Some(Command::Logout) => auth::logout(),
        #[cfg(feature = "auth")]
        Some(Command::Whoami) => auth::whoami(),
    }
}

fn make_providers() -> Vec<Box<dyn SessionProvider>> {
    vec![Box::new(ClaudeProvider), Box::new(CodexProvider)]
}

fn run_list() -> Result<()> {
    let providers = make_providers();
    let sessions = discover_all(&providers);

    if sessions.is_empty() {
        println!("No sessions found.");
        return Ok(());
    }

    println!("Found {} session(s):\n", sessions.len());
    for (i, session) in sessions.iter().enumerate() {
        let title = session
            .title
            .as_deref()
            .or(session.first_user_message.as_deref())
            .unwrap_or("(untitled)");
        let title = truncate(title, 80);
        let agent = format!("{:?}", session.agent);
        let date = session.timestamp.format("%Y-%m-%d %H:%M");
        println!("  {}. [{}] {} ({})", i + 1, agent, title, date);
    }
    println!("\nUse `replay upload <session_id>` to upload a session.");

    Ok(())
}

fn truncate(s: &str, max: usize) -> String {
    // Collapse newlines/whitespace into single spaces for terminal display
    let collapsed: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.len() <= max {
        collapsed
    } else {
        let mut end = max;
        while !collapsed.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &collapsed[..end])
    }
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
        .find(|s| s.id == session_id || s.title.as_deref() == Some(session_id))
        .ok_or_else(|| anyhow::anyhow!("Session '{}' not found", session_id))?;

    let messages = load_session_messages(&providers, session)?;
    let loaded = LoadedSession {
        session: session.clone(),
        messages,
    };
    uploader.upload(&loaded)?;
    Ok(())
}
