mod api;
mod cli;
mod commands;
mod config;
mod config_edit;
mod context;
mod credentials;
mod format;
mod links;
mod maths;
mod pick;
mod usage;

use anyhow::Result;
use clap::Parser;
use cli::{Cli, Commands};
use context::Context;

fn main() {
    let cli = Cli::parse();
    // Shell startup, completions and the status line must never fail, whatever is wrong.
    if let Some(Commands::Notice | Commands::Statusline | Commands::Accounts) = cli.command {
        if let Ok(ctx) = Context::load(cli.config) {
            match cli.command {
                Some(Commands::Notice) => commands::notice::run(&ctx),
                Some(Commands::Accounts) => {
                    for a in &ctx.accounts {
                        println!("{}", a.name);
                    }
                }
                _ => commands::statusline::run(&ctx),
            }
        }
        return;
    }
    if let Err(e) = real_main(cli) {
        eprintln!("tally: {e:#}");
        std::process::exit(1);
    }
}

fn real_main(cli: Cli) -> Result<()> {
    // need no config: they must work even when the config is broken
    match cli.command {
        Some(Commands::SelfUpgrade) => return commands::self_upgrade::run(),
        Some(Commands::Init { shell }) => {
            commands::init::run(shell);
            return Ok(());
        }
        _ => {}
    }
    let ctx = Context::load(cli.config)?;
    match cli.command {
        Some(Commands::Run { account, args }) => {
            commands::run::run(&ctx, account.as_deref(), &args)
        }
        Some(Commands::Pick { why }) => commands::pick::run(&ctx, why),
        Some(Commands::Add { name, plan }) => commands::add::run(&ctx, &name, plan),
        Some(Commands::Remove { name }) => commands::remove::run(&ctx, &name),
        Some(Commands::Login { account }) => commands::login::run(&ctx, &account),
        Some(Commands::Refresh) => {
            commands::refresh::run(&ctx);
            Ok(())
        }
        None => commands::summary::run(&ctx),
        // handled above
        Some(
            Commands::Notice
            | Commands::Statusline
            | Commands::Accounts
            | Commands::SelfUpgrade
            | Commands::Init { .. },
        ) => Ok(()),
    }
}
