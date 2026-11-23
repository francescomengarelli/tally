use anyhow::{Context as _, Result, bail};
use std::fs;
use std::io::{IsTerminal, Write};

use crate::config::Account;
use crate::config_edit;
use crate::context::Context;

pub fn run(ctx: &Context, name: &str) -> Result<()> {
    if ctx.config.unit.as_deref() == Some(name) {
        bail!(
            "{name} is the unit account; change unit in {} first",
            ctx.paths.config.display()
        );
    }
    let account = ctx.account(name).cloned();
    config_edit::remove_account(&ctx.paths.config, name)?;
    eprintln!("tally: removed {name} from the config");

    // The main folder is Claude's own; only tally's account folders can go.
    let Some(account) = account.filter(|a| !a.is_main && a.dir.exists()) else {
        return Ok(());
    };
    let others: Vec<&Account> = ctx.accounts.iter().filter(|a| a.name != name).collect();
    if let Err(why) = deletable(ctx, &account, &others) {
        eprintln!("tally: i left {} in place: {why}", account.dir.display());
        return Ok(());
    }
    if !std::io::stdin().is_terminal() {
        eprintln!(
            "tally: i left {} in place (not a terminal, so i didn't ask)",
            account.dir.display()
        );
        return Ok(());
    }
    eprint!("delete {} and its login? [y/N] ", account.dir.display());
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if matches!(answer.trim(), "y" | "Y" | "yes") {
        fs::remove_dir_all(&account.dir)
            .with_context(|| format!("i couldn't delete {}", account.dir.display()))?;
        eprintln!("tally: deleted {}", account.dir.display());
    }
    Ok(())
}

/// Whether the folder is safe to delete: strictly inside tally's data
/// folder once symlinks and `..` are resolved, not Claude's own folder or
/// anything holding it, and not shared with or holding another account.
fn deletable(ctx: &Context, account: &Account, others: &[&Account]) -> Result<(), String> {
    let dir = fs::canonicalize(&account.dir).map_err(|e| format!("i couldn't resolve it: {e}"))?;
    let data = fs::canonicalize(&ctx.paths.data)
        .map_err(|e| format!("i couldn't resolve {}: {e}", ctx.paths.data.display()))?;
    if dir == data || !dir.starts_with(&data) {
        return Err(format!("it isn't inside {}", data.display()));
    }
    if let Ok(main) = fs::canonicalize(ctx.paths.main_dir())
        && main.starts_with(&dir)
    {
        return Err("it is or holds the main claude folder".into());
    }
    for other in others {
        if let Ok(o) = fs::canonicalize(&other.dir)
            && o.starts_with(&dir)
        {
            return Err(format!("the account {} uses it", other.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, Paths};

    fn ctx(toml: &str) -> (tempfile::TempDir, Context) {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_home(tmp.path(), None);
        fs::create_dir_all(tmp.path().join(".claude")).unwrap();
        fs::create_dir_all(&paths.data).unwrap();
        let config: Config = toml::from_str(toml).unwrap();
        let accounts = config.accounts(&paths);
        (
            tmp,
            Context {
                paths,
                config,
                accounts,
            },
        )
    }

    fn deletable_for(ctx: &Context, name: &str) -> Result<(), String> {
        let account = ctx.account(name).unwrap();
        let others: Vec<_> = ctx.accounts.iter().filter(|a| a.name != name).collect();
        deletable(ctx, account, &others)
    }

    #[test]
    fn a_normal_account_folder_is_deletable() {
        let (_t, ctx) = ctx("[account.a]\n");
        fs::create_dir_all(&ctx.accounts[0].dir).unwrap();
        assert!(deletable_for(&ctx, "a").is_ok());
    }

    #[test]
    fn dotdot_into_the_main_folder_is_refused() {
        let (_t, ctx) = ctx("[account.a]\ndir = \"~/.local/share/tally/../../../.claude\"\n");
        assert!(deletable_for(&ctx, "a").is_err());
        let (_t, ctx) = ctx_home_escape();
        assert!(deletable_for(&ctx, "a").is_err());
    }

    fn ctx_home_escape() -> (tempfile::TempDir, Context) {
        ctx("[account.a]\ndir = \"~/.local/share/tally/../../..\"\n")
    }

    #[test]
    fn the_data_folder_itself_is_refused() {
        let (_t, ctx) = ctx("[account.a]\ndir = \"~/.local/share/tally\"\n");
        assert!(deletable_for(&ctx, "a").is_err());
    }

    #[test]
    fn a_folder_another_account_uses_is_refused() {
        let (_t, ctx) = ctx("[account.a]\n[account.b]\ndir = \"~/.local/share/tally/a\"\n");
        fs::create_dir_all(&ctx.accounts[0].dir).unwrap();
        assert!(deletable_for(&ctx, "a").is_err());
    }
}
