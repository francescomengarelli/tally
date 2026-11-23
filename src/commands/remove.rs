use anyhow::{Context as _, Result};
use std::fs;
use std::io::{IsTerminal, Write};

use crate::config_edit;
use crate::context::Context;

pub fn run(ctx: &Context, name: &str) -> Result<()> {
    let account = ctx.account(name).cloned();
    config_edit::remove_account(&ctx.paths.config, name)?;
    eprintln!("tally: removed {name} from the config");

    // The main folder is Claude's own; only tally's account folders can go.
    let Some(account) = account.filter(|a| !a.is_main && a.dir.exists()) else {
        return Ok(());
    };
    if !account.dir.starts_with(&ctx.paths.data) {
        eprintln!("tally: i left {} in place", account.dir.display());
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
