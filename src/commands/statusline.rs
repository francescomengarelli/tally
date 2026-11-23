use std::process::{Command, Stdio};

use anyhow::anyhow;
use jiff::{SignedDuration, Timestamp};

use super::{Snapshot, snapshot};
use crate::api::{ApiError, Tokens, Usage};
use crate::context::Context;
use crate::format;
use crate::usage::{Net, Status};

/// The status line never touches the network; `refresh` does that.
struct Cached;

impl Net for Cached {
    fn usage(&self, _: &str) -> Result<Usage, ApiError> {
        Err(ApiError::Other(anyhow!("cache only")))
    }
    fn refresh(&self, _: &str) -> Result<Tokens, ApiError> {
        Err(ApiError::Other(anyhow!("cache only")))
    }
}

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

/// Does any account that could be fetched have no cache, or one older than a minute?
fn stale(snap: &Snapshot) -> bool {
    let at = Timestamp::now();
    snap.rows.iter().any(|row| match &row.status {
        Status::Ready { fetched_at, .. } => {
            at.duration_since(*fetched_at) > SignedDuration::from_mins(1)
        }
        Status::NotLoggedIn => false,
        _ => true,
    })
}

/// Run `tally refresh` on its own, without waiting for it.
fn spawn_refresh(ctx: &Context) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(exe)
        .arg("--config")
        .arg(&ctx.paths.config)
        .arg("refresh")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

/// Never fails: Claude Code would show an error in place of the line.
pub fn run(ctx: &Context) {
    let snap = snapshot(ctx, SignedDuration::MAX, &Cached);
    if stale(&snap) {
        spawn_refresh(ctx);
    }
    let any = snap
        .rows
        .iter()
        .any(|r| matches!(r.status, Status::Ready { .. }));
    if let (true, Some(name)) = (any, account_name(ctx)) {
        println!("{}", format::statusline(&name, &snap));
    }
}
