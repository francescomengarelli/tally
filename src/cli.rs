use clap::{Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "tally", about = "claude on several accounts, and how much is left", version = env!("CARGO_PKG_VERSION"))]
pub struct Cli {
    /// Path to a custom config file (default: ~/.config/tally/config.toml)
    #[arg(long, global = true)]
    pub config: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Run claude as an account, or as the one tally picks.
    ///
    /// Everything after `--` goes to claude untouched.
    ///
    /// Example: `tally run personal -- --resume`
    Run {
        /// The account to run as (default: the picked one).
        account: Option<String>,
        /// Arguments for claude.
        #[arg(last = true)]
        args: Vec<String>,
    },
    /// Print the account tally would pick.
    Pick {
        /// Show each account's numbers and the rule that decided.
        #[arg(long)]
        why: bool,
    },
    /// Add an account to the config, then log it in.
    Add {
        /// Lowercase letters, digits and dashes.
        name: String,
        /// The plan as a multiple of Pro (e.g. 5), when the login can't tell.
        #[arg(long)]
        plan: Option<f64>,
    },
    /// Remove an account from the config; ask before deleting its folder.
    Remove { name: String },
    /// Log an account in (creates its folder if needed).
    Login { account: String },
    /// One line for shell startup.
    Notice,
    /// One line for Claude Code's status line.
    Statusline,
    /// Print shell functions for `| source`.
    Init { shell: Shell },
    /// Refresh the cache in the background.
    #[command(hide = true)]
    Refresh,
}

#[derive(Clone, Copy, ValueEnum)]
pub enum Shell {
    Fish,
}
