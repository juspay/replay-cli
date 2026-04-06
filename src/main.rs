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
use upload::{UploadOutcome, Uploader};

const BANNER: &str = r#"
██████╗ ███████╗██████╗ ██╗      █████╗ ██╗   ██╗
██╔══██╗██╔════╝██╔══██╗██║     ██╔══██╗╚██╗ ██╔╝
██████╔╝█████╗  ██████╔╝██║     ███████║ ╚████╔╝
██╔══██╗██╔══╝  ██╔═══╝ ██║     ██╔══██║  ╚██╔╝
██║  ██║███████╗██║     ███████╗██║  ██║   ██║
╚═╝  ╚═╝╚══════╝╚═╝     ╚══════╝╚═╝  ╚═╝   ╚═╝   "#;

fn reset_terminal() {
    // Show cursor, disable raw mode, reset attributes
    eprint!("\x1b[?25h\x1b[0m");
    // Best-effort: disable raw mode via stty
    let _ = std::process::Command::new("stty")
        .arg("sane")
        .stdin(std::process::Stdio::inherit())
        .status();
}

fn main() -> Result<()> {
    ctrlc::set_handler(move || {
        reset_terminal();
        std::process::exit(130);
    })
    .expect("failed to set Ctrl-C handler");

    let cli = Cli::parse();

    let result = match cli.command {
        None => run_home(),
        Some(Command::List { all }) => run_list(all),
        Some(Command::Upload { session_id }) => match session_id {
            Some(id) => run_upload(&id),
            None => run_upload_picker(),
        },
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
    };

    reset_terminal();
    result
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
const ITALIC: &str = "\x1b[3m";
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

    // Usage
    println!("  {BOLD}Usage:{RESET}");
    println!("    replay {CYAN}whoami{RESET}                 Show authenticated user");
    println!(
        "    replay {CYAN}upload{RESET}                 Upload sessions for current directory"
    );
    println!("    replay {CYAN}upload{RESET} {DIM}<session-id>{RESET}   Upload a specific session");
    println!();

    Ok(())
}

// ── replay list ────────────────────────────────────────────────────────

fn run_list(show_all: bool) -> Result<()> {
    let providers = make_providers();
    let all_sessions = discover_all(&providers);

    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy();

    let sessions: Vec<_> = if show_all {
        all_sessions
    } else {
        all_sessions
            .into_iter()
            .filter(|s| {
                s.project_path
                    .as_deref()
                    .map(|p| p == cwd_str.as_ref())
                    .unwrap_or(false)
            })
            .collect()
    };

    if sessions.is_empty() {
        if show_all {
            println!("No sessions found.");
        } else {
            println!("No sessions found for {DIM}{cwd_str}{RESET}");
            println!("{DIM}Use `replay list -a` to show all sessions.{RESET}");
        }
        return Ok(());
    }

    if show_all {
        println!("  {BOLD}All sessions{RESET}\n");
    } else {
        println!("  {BOLD}Sessions for{RESET} {DIM}{cwd_str}{RESET}\n");
    }

    let term_width = console::Term::stdout().size().1 as usize;

    // Fixed columns: 2 (indent) + 36 (uuid) + 2 + 8 (agent) + 2 + 16 (date) + 2 = 68 + 5 (right padding)
    let fixed = 73;
    let title_width = if term_width > fixed + 10 {
        term_width - fixed
    } else {
        20 // minimum
    };

    // Group by project_path when --all
    if show_all {
        let mut by_project: std::collections::BTreeMap<String, Vec<&models::Session>> =
            std::collections::BTreeMap::new();
        for s in &sessions {
            let key = s.project_path.as_deref().unwrap_or("(unknown)").to_string();
            by_project.entry(key).or_default().push(s);
        }

        for (project, group) in &by_project {
            println!("\n  {CYAN}{project}{RESET}");
            for s in group {
                print_session_row(s, title_width);
            }
        }
    } else {
        for s in &sessions {
            print_session_row(s, title_width);
        }
    }

    println!(
        "\n  {DIM}{} session(s). Upload with:{RESET} replay upload {CYAN}<id>{RESET}",
        sessions.len()
    );

    Ok(())
}

fn print_session_row(s: &models::Session, title_width: usize) {
    let agent = format!("{:?}", s.agent);
    let date = s.timestamp.format("%Y-%m-%d %H:%M").to_string();
    let title = s
        .title
        .as_deref()
        .or(s.first_user_message.as_deref())
        .unwrap_or("(untitled)");
    let title = truncate(title, title_width);

    println!(
        "  {DIM}{}{RESET}  {:<8} {DIM}{}{RESET}  {BOLD}{}{RESET}",
        s.id, agent, date, title
    );
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

    let title = session
        .title
        .as_deref()
        .or(session.first_user_message.as_deref())
        .unwrap_or("(untitled)");
    let title = truncate(title, 50);

    eprint!("  {title} … ");

    let messages = load_session_messages(&providers, session)?;
    let loaded = LoadedSession {
        session: session.clone(),
        messages,
    };
    match uploader.upload(&loaded) {
        Ok(outcome) => {
            print_upload_outcome(&outcome);
        }
        Err(e) => {
            eprintln!("{RED}failed{RESET}");
            eprintln!("  {DIM}{e}{RESET}");
            return Err(e);
        }
    }
    Ok(())
}

// ── upload output helpers ───────────────────────────────────────────────

fn print_upload_outcome(outcome: &UploadOutcome) {
    match outcome {
        UploadOutcome::Created { url } => {
            eprintln!("{GREEN}uploaded{RESET}");
            eprintln!("  {CYAN}{url}{RESET}");
        }
        UploadOutcome::Updated { url } => {
            eprintln!("{GREEN}updated{RESET}");
            eprintln!("  {CYAN}{url}{RESET}");
        }
    }
}

// ── replay upload (interactive) ─────────────────────────────────────────

fn upload_sessions(
    indices: &[usize],
    sessions: &[models::Session],
    providers: &[Box<dyn SessionProvider>],
    uploader: &upload::HttpUploader,
) -> Result<()> {
    let total = indices.len();
    let mut success = 0;
    let mut failed = 0;
    let mut urls: Vec<String> = Vec::new();

    for (n, &idx) in indices.iter().enumerate() {
        let session = &sessions[idx];
        let title = session
            .title
            .as_deref()
            .or(session.first_user_message.as_deref())
            .unwrap_or("(untitled)");
        let title = truncate(title, 50);

        eprint!("  {DIM}[{}/{}]{RESET} {title} … ", n + 1, total);

        match load_session_messages(providers, session) {
            Ok(messages) => {
                let loaded = LoadedSession {
                    session: session.clone(),
                    messages,
                };
                match uploader.upload(&loaded) {
                    Ok(outcome) => {
                        let (label, url) = match outcome {
                            UploadOutcome::Created { url } => ("uploaded", url),
                            UploadOutcome::Updated { url } => ("updated", url),
                        };
                        eprintln!("{GREEN}{label}{RESET}");
                        urls.push(url);
                        success += 1;
                    }
                    Err(e) => {
                        eprintln!("{RED}failed{RESET} {DIM}— {e}{RESET}");
                        failed += 1;
                    }
                }
            }
            Err(e) => {
                eprintln!("{RED}failed{RESET} {DIM}— {e}{RESET}");
                failed += 1;
            }
        }
    }

    eprintln!();
    if failed == 0 {
        eprintln!("  {GREEN}All {success} session(s) uploaded.{RESET}");
    } else {
        eprintln!("  {success} uploaded, {RED}{failed} failed{RESET}.");
    }

    if !urls.is_empty() {
        eprintln!();
        for url in &urls {
            eprintln!("  {CYAN}{url}{RESET}");
        }
        eprintln!();
        eprintln!("  {DIM}{ITALIC}All sessions are private by default.{RESET}");
    }

    Ok(())
}

fn run_upload_picker() -> Result<()> {
    use dialoguer::MultiSelect;

    let token = auth::get_token()?
        .ok_or_else(|| anyhow::anyhow!("Not logged in. Run `replay login` first."))?;
    let server_url = auth::config::server_url();
    let uploader = upload::HttpUploader { server_url, token };

    let cwd = std::env::current_dir()?;
    let cwd_str = cwd.to_string_lossy();

    let providers = make_providers();
    let all_sessions = discover_all(&providers);

    let sessions: Vec<_> = all_sessions
        .into_iter()
        .filter(|s| {
            s.project_path
                .as_deref()
                .map(|p| p == cwd_str.as_ref())
                .unwrap_or(false)
        })
        .collect();

    if sessions.is_empty() {
        println!("No sessions found for {DIM}{cwd_str}{RESET}");
        return Ok(());
    }

    // Prompt for action
    println!(
        "  Found {BOLD}{}{RESET} session(s) for {DIM}{}{RESET}\n",
        sessions.len(),
        cwd_str
    );
    print!(
        "  Upload {BOLD}[a]{RESET}ll {DIM}({}){RESET} / {BOLD}[s]{RESET}elect / {BOLD}[q]{RESET}uit: ",
        sessions.len()
    );
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let input = input.trim().to_lowercase();

    println!();

    match input.as_str() {
        "a" | "all" | "" => {
            let all: Vec<usize> = (0..sessions.len()).collect();
            upload_sessions(&all, &sessions, &providers, &uploader)?;
        }
        "s" | "select" => {
            let labels: Vec<String> = sessions
                .iter()
                .map(|s| {
                    let title = s
                        .title
                        .as_deref()
                        .or(s.first_user_message.as_deref())
                        .unwrap_or("(untitled)");
                    let title = truncate(title, 60);
                    let agent = format!("{:?}", s.agent);
                    let date = s.timestamp.format("%Y-%m-%d %H:%M");
                    format!("[{}] {} ({})", agent, title, date)
                })
                .collect();

            println!("  {DIM}space{RESET} toggle  {DIM}enter{RESET} confirm\n");

            let selected = MultiSelect::new()
                .items(&labels)
                .defaults(&vec![true; labels.len()])
                .interact()?;

            if selected.is_empty() {
                println!("  Nothing selected.");
                return Ok(());
            }

            println!();
            upload_sessions(&selected, &sessions, &providers, &uploader)?;
        }
        "q" | "quit" | "n" | "no" => {
            println!("  Aborted.");
        }
        _ => {
            println!("  Unknown option. Use {BOLD}a{RESET}, {BOLD}s{RESET}, or {BOLD}q{RESET}.");
        }
    }

    Ok(())
}
