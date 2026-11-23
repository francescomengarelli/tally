mod api;
mod cli;
mod commands;
mod config;
mod config_edit;
mod context;
mod credentials;
mod links;
mod maths;
mod pick;
mod usage;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use context::Context;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let ctx = Context::load(cli.config)?;
    match cli.command {
        Some(Commands::Run { account, args }) => {
            commands::run::run(&ctx, account.as_deref(), &args)
        }
        Some(Commands::Pick { why }) => commands::pick::run(&ctx, why),
        Some(Commands::Add { name, plan }) => commands::add::run(&ctx, &name, plan),
        Some(Commands::Remove { name }) => commands::remove::run(&ctx, &name),
        Some(Commands::Login { account }) => commands::login::run(&ctx, &account),
        Some(Commands::Init { shell }) => commands::init::run(&ctx, shell),
        // Task 10: the summary, notice, statusline and refresh.
        None | Some(Commands::Notice | Commands::Statusline | Commands::Refresh) => {
            anyhow::bail!("not built yet")
        }
    }
}
