use std::io::{IsTerminal, Read};

use jiff::{SignedDuration, Timestamp};

use super::{Snapshot, build, refresh_in_background};
use crate::context::Context;
use crate::format::{self, Hint};
use crate::pick;
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

/// The session id from the JSON Claude Code pipes in, if it looks like one.
/// It ends up on screen, so nothing but letters, digits and dashes.
fn session_id() -> Option<String> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut body = String::new();
    stdin.read_to_string(&mut body).ok()?;
    let v: serde_json::Value = serde_json::from_str(&body).ok()?;
    let id = v.get("session_id")?.as_str()?;
    let ok = !id.is_empty()
        && id.len() <= 64
        && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    ok.then(|| id.to_string())
}

/// What the picked account offers over the one in use, if it is worth a switch.
fn switch(snap: &Snapshot, account: &str) -> Option<(String, pick::Switch)> {
    let to = &snap.pick.as_ref()?.name;
    if to == account {
        return None;
    }
    let metrics = |name: &str| {
        snap.rows
            .iter()
            .find(|r| r.account.name == name)?
            .metrics
            .as_ref()
    };
    let s = pick::switch(metrics(account)?, metrics(to)?, snap.min_session)?;
    Some((to.clone(), s))
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
    let (true, Some(name)) = (any, account_name(ctx)) else {
        return;
    };
    let session = session_id();
    let hint = switch(&snap, &name);
    let hint = hint.as_ref().map(|(to, switch)| Hint {
        to,
        session: session.as_deref(),
        switch: *switch,
    });
    println!(
        "{}",
        format::statusline(&name, &snap, hint.as_ref(), format::colour(true))
    );
}
