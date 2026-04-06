#[cfg(feature = "auth")]
mod auth;
mod cli;
mod models;
mod providers;
mod upload;

use anyhow::Result;
use clap::{CommandFactory, Parser};
use std::io::{self, Write};

use cli::{Cli, Command};
#[allow(unused_imports)]
use models::LoadedSession;
use providers::claude::ClaudeProvider;
use providers::codex::CodexProvider;
use providers::{discover_all, load_session_messages, SessionProvider};
#[allow(unused_imports)]
use upload::Uploader;

const BANNER: &str = r#"
██████╗ ███████╗██████╗ ██╗      █████╗ ██╗   ██╗
██╔══██╗██╔════╝██╔══██╗██║     ██╔══██╗╚██╗ ██╔╝
██████╔╝█████╗  ██████╔╝██║     ███████║ ╚████╔╝
██╔══██╗██╔══╝  ██╔═══╝ ██║     ██╔══██║  ╚██╔╝
██║  ██║███████╗██║     ███████╗██║  ██║   ██║
╚═╝  ╚═╝╚══════╝╚═╝     ╚══════╝╚═╝  ╚═╝   ╚═╝   "#;

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        None => run_home(),
        Some(Command::List) => run_list(),
        // TODO: re-enable with picker UI
        // Some(Command::Upload { session_id }) => match session_id {
        //     Some(id) => run_upload(&id),
        //     None => run_upload_cwd(),
        // },
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

// ── bare `replay` ──────────────────────────────────────────────────────

// ANSI color helpers
const RESET: &str = "\x1b[0m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const GREEN: &str = "\x1b[32m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";

fn run_home() -> Result<()> {
    // Banner
    println!("{BOLD}{CYAN}{BANNER}{RESET}");
    println!();

    // Login status
    #[cfg(feature = "auth")]
    {
        match auth::get_user_info() {
            Ok(Some(user)) => {
                println!(
                    "  {GREEN}●{RESET} Logged in as {BOLD}{}{RESET} {DIM}({}){RESET}",
                    user.name, user.email
                );
            }
            Ok(None) => {
                // \x1b[s = save cursor, print status line
                print!("  {RED}●{RESET} Not logged in.  ");
                print!("Log in now? {DIM}[Y/n]{RESET} ");
                io::stdout().flush()?;

                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                let input = input.trim().to_lowercase();

                if input.is_empty() || input == "y" || input == "yes" {
                    auth::login()?;
                    // login() prints 3 lines to stderr + 1 prompt line = 4 lines to clear
                    // Move up 4 lines and clear each
                    for _ in 0..4 {
                        eprint!("\x1b[A\x1b[2K");
                    }
                    if let Ok(Some(user)) = auth::get_user_info() {
                        eprintln!(
                            "  {GREEN}●{RESET} Logged in as {BOLD}{}{RESET} {DIM}({}){RESET}",
                            user.name, user.email
                        );
                    } else {
                        eprintln!("  {GREEN}●{RESET} Logged in.");
                    }
                } else {
                    // Clear the prompt line
                    eprint!("\x1b[A\x1b[2K");
                    eprintln!("  {RED}●{RESET} Not logged in.");
                }
            }
            Err(_) => {
                println!("  {DIM}Could not check login status.{RESET}");
            }
        }
    }

    #[cfg(not(feature = "auth"))]
    {
        println!("  {DIM}Auth not enabled (build with --features auth).{RESET}");
    }

    println!();

    // tldr-style usage
    println!("  Show the current authenticated user:");
    println!();
    println!("    replay {CYAN}whoami{RESET}");
    println!();
    println!("  Upload all sessions for the current directory:");
    println!();
    println!("    replay {CYAN}upload{RESET}");
    println!();
    println!("  Upload a specific session by ID:");
    println!();
    println!("    replay {CYAN}upload{RESET} {RED}<session-id>{RESET}");

    Ok(())
}

// ── replay list ────────────────────────────────────────────────────────

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

// ── replay upload <id> ─────────────────────────────────────────────────

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

// ── replay upload (all sessions for cwd) ───────────────────────────────

fn run_upload_cwd() -> Result<()> {
    let token = auth::get_token()?
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run `replay login` first."))?;
    let server_url = auth::config::server_url();
    let uploader = upload::HttpUploader { server_url, token };

    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy();

    let providers = make_providers();
    let all_sessions = discover_all(&providers);

    // Filter to sessions whose project_path matches the current directory
    let sessions: Vec<_> = all_sessions
        .iter()
        .filter(|s| {
            s.project_path
                .as_deref()
                .map(|p| p == cwd_str.as_ref())
                .unwrap_or(false)
        })
        .collect();

    if sessions.is_empty() {
        println!("No sessions found for {}", cwd_str);
        return Ok(());
    }

    println!(
        "Found {} session(s) for {}:\n",
        sessions.len(),
        cwd_str
    );
    for (i, session) in sessions.iter().enumerate() {
        let title = session
            .title
            .as_deref()
            .or(session.first_user_message.as_deref())
            .unwrap_or("(untitled)");
        let title = truncate(title, 70);
        let agent = format!("{:?}", session.agent);
        let date = session.timestamp.format("%Y-%m-%d %H:%M");
        println!("  {}. [{}] {} ({})", i + 1, agent, title, date);
    }

    println!();
    print!("Upload all {} session(s)? [Y/n] ", sessions.len());
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    if !(input.is_empty() || input == "y" || input == "yes") {
        println!("Aborted.");
        return Ok(());
    }

    println!();
    let mut success = 0;
    let mut failed = 0;

    for session in &sessions {
        let title = session
            .title
            .as_deref()
            .or(session.first_user_message.as_deref())
            .unwrap_or("(untitled)");
        let title = truncate(title, 50);

        match load_session_messages(&providers, session) {
            Ok(messages) => {
                let loaded = LoadedSession {
                    session: (*session).clone(),
                    messages,
                };
                match uploader.upload(&loaded) {
                    Ok(()) => {
                        success += 1;
                    }
                    Err(e) => {
                        eprintln!("  ✗ {} — {}", title, e);
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("  ✗ {} — failed to load: {}", title, e);
                failed += 1;
            }
        }
    }

    println!();
    println!(
        "Done. {} uploaded, {} failed.",
        success, failed
    );

    Ok(())
}
