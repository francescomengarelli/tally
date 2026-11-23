use jiff::{SignedDuration, Timestamp};

use super::{build, refresh_in_background};
use crate::context::Context;
use crate::format;
use crate::usage::{self, Status};

/// `TALLY_ACCOUNT`, else the account whose folder is `CLAUDE_CONFIG_DIR`, else main.
fn account_name(ctx: &Context) -> Option<String> {
    if let Some(name) = std::env::var("TALLY_ACCOUNT")
        .ok()
        .filter(|n| !n.is_empty())
    {
        return Some(name);
    }
    let dir = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|d| !d.is_empty());
    let found = dir.and_then(|d| {
        ctx.accounts
            .iter()
            .find(|a| a.dir == std::path::Path::new(&d))
    });
    found
        .or_else(|| ctx.accounts.iter().find(|a| a.is_main))
        .map(|a| a.name.clone())
}

/// Never fails: Claude Code would show an error in place of the line.
/// Reads the cache only; a background refresh keeps it fresh.
/// Claude Code renders the colours, so they are on unless `NO_COLOR` is set.
pub fn run(ctx: &Context) {
    let at = Timestamp::now();
    let statuses = ctx
        .accounts
        .iter()
        .map(|a| usage::from_cache(a, &ctx.paths.cache, SignedDuration::from_mins(1), at))
        .collect();
    let snap = build(ctx, statuses);
    refresh_in_background(ctx, &snap);
    let any = snap
        .rows
        .iter()
        .any(|r| matches!(r.status, Status::Ready { .. }));
    if let (true, Some(name)) = (any, account_name(ctx)) {
        println!("{}", format::statusline(&name, &snap, format::colour(true)));
    }
}
