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
    List,

    // TODO: re-enable with picker UI
    // /// Upload session(s) — by ID, or all sessions for the current directory
    // Upload {
    //     /// Session ID or title to upload (omit to upload all sessions for cwd)
    //     session_id: Option<String>,
    // },

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
