use anyhow::{Context as _, Result, bail};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::process::Command;

use super::claude_bin;
use crate::config::Account;
use crate::context::Context;
use crate::links;

pub fn run(ctx: &Context, name: &str) -> Result<()> {
    let Some(account) = ctx.account(name) else {
        let names: Vec<&str> = ctx.accounts.iter().map(|a| a.name.as_str()).collect();
        bail!(
            "i don't know the account '{name}': the accounts are {}",
            names.join(", ")
        );
    };
    login(ctx, account)
}

/// Prepare the account's folder, then run `claude auth login` in it.
pub fn login(ctx: &Context, account: &Account) -> Result<()> {
    let mut cmd = Command::new(claude_bin());
    cmd.args(["auth", "login"]);
    if account.is_main {
        cmd.env_remove("CLAUDE_CONFIG_DIR");
    } else {
        for warning in links::reconcile(&account.dir, &ctx.paths.main_dir(), &ctx.config.shared)? {
            eprintln!("tally: {warning}");
        }
        seed_claude_json(ctx, account)?;
        cmd.env("CLAUDE_CONFIG_DIR", &account.dir);
    }
    let status = cmd
        .env("TALLY_ACCOUNT", &account.name)
        .status()
        .context("i couldn't start claude")?;
    if !status.success() {
        bail!("claude login failed for {}", account.name);
    }
    Ok(())
}

/// A new account starts with the main one's MCP servers and theme.
/// An existing `.claude.json` is never touched.
fn seed_claude_json(ctx: &Context, account: &Account) -> Result<()> {
    let target = account.dir.join(".claude.json");
    if target.exists() {
        return Ok(());
    }
    let Ok(text) = fs::read_to_string(ctx.paths.home.join(".claude.json")) else {
        return Ok(());
    };
    let Ok(Value::Object(source)) = serde_json::from_str(&text) else {
        return Ok(());
    };
    let seed: Map<String, Value> = ["mcpServers", "theme"]
        .iter()
        .filter_map(|k| source.get(*k).map(|v| (k.to_string(), v.clone())))
        .collect();
    if seed.is_empty() {
        return Ok(());
    }
    // private: MCP servers can carry tokens in their env
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&target);
    let mut file = match file {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(()),
        Err(e) => {
            return Err(e).with_context(|| format!("i couldn't write {}", target.display()));
        }
    };
    file.write_all(Value::Object(seed).to_string().as_bytes())
        .with_context(|| format!("i couldn't write {}", target.display()))
}
