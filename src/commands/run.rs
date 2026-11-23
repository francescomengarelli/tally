use anyhow::{Result, bail};
use jiff::{SignedDuration, Timestamp, tz::TimeZone};
use std::os::unix::process::CommandExt;
use std::process::Command;

use super::{claude_bin, snapshot};
use crate::context::Context;
use crate::links;
use crate::pick::Rule;
use crate::usage::Http;

pub fn run(ctx: &Context, account: Option<&str>, args: &[String]) -> Result<()> {
    let name = match account {
        Some(name) => {
            if ctx.account(name).is_none() {
                let names: Vec<&str> = ctx.accounts.iter().map(|a| a.name.as_str()).collect();
                bail!(
                    "i don't know the account '{name}': the accounts are {}",
                    names.join(", ")
                );
            }
            name.to_string()
        }
        None => pick_one(ctx),
    };
    let account = ctx.account(&name).expect("checked above");

    let mut cmd = Command::new(claude_bin());
    cmd.args(args).env("TALLY_ACCOUNT", &account.name);
    if account.is_main {
        // a value inherited from a parent claude must not leak in
        cmd.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        for warning in links::reconcile(&account.dir, &ctx.paths.main_dir(), &ctx.config.shared)? {
            eprintln!("tally: {warning}");
        }
        cmd.env("CLAUDE_CONFIG_DIR", &account.dir);
    }
    let err = cmd.exec();
    bail!("i couldn't start claude: {err}")
}

/// The picked account's name, saying so on stderr.
fn pick_one(ctx: &Context) -> String {
    let snap = snapshot(ctx, SignedDuration::from_mins(1), &Http);
    // an account list is never empty, so there is always a pick
    let pick = snap.pick.expect("at least one account");
    eprintln!("tally: {}", pick.name);
    if pick.rule == Rule::NoneEligible {
        let reset = snap
            .rows
            .iter()
            .find(|r| r.account.name == pick.name)
            .and_then(|r| r.metrics.as_ref())
            .and_then(|m| m.binding_reset);
        match reset {
            Some(at) => eprintln!(
                "tally: all accounts are near a limit; {} frees up at {}",
                pick.name,
                clock(at)
            ),
            None => eprintln!("tally: all accounts are near a limit"),
        }
    }
    pick.name
}

fn clock(at: Timestamp) -> String {
    at.to_zoned(TimeZone::system())
        .strftime("%H:%M")
        .to_string()
}
