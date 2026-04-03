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
    /// Upload a session by ID
    Upload {
        /// Session ID or title to upload
        session_id: String,
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
