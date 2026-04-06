use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser)]
#[command(name = "replay", about = "Browse and upload coding agent sessions")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Subcommand)]
pub enum Command {
    /// List discovered sessions
    List {
        /// Show all sessions globally (default: only current directory)
        #[arg(short, long)]
        all: bool,
    },

    /// Upload session(s) — by ID, or pick interactively
    Upload {
        /// Session ID or title to upload (omit for interactive picker)
        session_id: Option<String>,

        /// Consider all sessions globally (default: only current directory)
        #[arg(short, long)]
        global: bool,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: Shell,
    },

    #[cfg(feature = "auth")]
    /// Authenticate with the Replay server
    Login,

    #[cfg(feature = "auth")]
    /// Remove stored credentials
    Logout,

    #[cfg(feature = "auth")]
    /// Show current authenticated user
    Whoami,
}
