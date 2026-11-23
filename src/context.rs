use anyhow::{Context as _, Result};
use std::path::{Path, PathBuf};

use crate::config::{self, Account, Config, Paths};
use crate::config_edit::validate_name;

/// Every account needs a folder of its own: absolute, not home or a parent
/// of the main folder, and not shared with another account (two accounts on
/// one login would refresh the same rotating token at once).
fn check_dirs(paths: &Paths, accounts: &[Account]) -> Result<()> {
    let main = paths.main_dir();
    for a in accounts {
        if !a.dir.is_absolute()
            || a.dir == paths.home
            || (a.dir != main && main.starts_with(&a.dir))
        {
            anyhow::bail!(
                "the dir for {} must be an absolute folder of its own",
                a.name
            );
        }
    }
    // A folder that doesn't exist yet compares by its spelling.
    let real = |dir: &Path| std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    for (i, a) in accounts.iter().enumerate() {
        for b in &accounts[i + 1..] {
            if real(&a.dir) == real(&b.dir) {
                anyhow::bail!("{} and {} use the same folder", a.name, b.name);
            }
        }
    }
    Ok(())
}

pub struct Context {
    pub paths: Paths,
    pub config: Config,
    pub accounts: Vec<Account>,
}

impl Context {
    pub fn load(config: Option<PathBuf>) -> Result<Context> {
        let home = dirs::home_dir().context("i couldn't find your home folder")?;
        let paths = Paths::from_home(&home, config);
        let config = config::load(&paths)?;
        for name in config.account.keys() {
            validate_name(name).with_context(|| {
                format!(
                    "i couldn't use the account '{name}' in {}",
                    paths.config.display()
                )
            })?;
        }
        for (name, entry) in &config.account {
            if entry.plan.is_some_and(|p| !p.is_finite() || p <= 0.0) {
                anyhow::bail!("the plan for {name} must be a positive number");
            }
        }
        let accounts = config.accounts(&paths);
        check_dirs(&paths, &accounts)?;
        config.unit(&accounts)?;
        Ok(Context {
            paths,
            config,
            accounts,
        })
    }

    pub fn account(&self, name: &str) -> Option<&Account> {
        self.accounts.iter().find(|a| a.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn accounts(home: &Path, toml: &str) -> (Paths, Vec<Account>) {
        let paths = Paths::from_home(home, None);
        let config: Config = toml::from_str(toml).unwrap();
        let accounts = config.accounts(&paths);
        (paths, accounts)
    }

    fn check(toml: &str) -> Result<()> {
        let tmp = tempfile::tempdir().unwrap();
        let (paths, accounts) = accounts(tmp.path(), toml);
        check_dirs(&paths, &accounts)
    }

    #[test]
    fn plain_accounts_pass() {
        check("[account.a]\ndir = \"~/.claude\"\n[account.b]\n[account.c]\ndir = \"~/other\"\n")
            .unwrap();
        check("").unwrap();
    }

    #[test]
    fn relative_home_and_parent_dirs_are_refused() {
        for dir in ["relative/x", "~", "$HOME", "$HOME/", "/", "../x"] {
            let e = check(&format!("[account.a]\ndir = \"{dir}\"\n")).unwrap_err();
            assert_eq!(
                e.to_string(),
                "the dir for a must be an absolute folder of its own",
                "{dir}"
            );
        }
    }

    #[test]
    fn the_same_folder_twice_is_refused() {
        let e = check("[account.a]\ndir = \"~/.claude\"\n[account.b]\ndir = \"~/.claude/\"\n")
            .unwrap_err();
        assert_eq!(e.to_string(), "a and b use the same folder");
        let e = check("[account.a]\ndir = \"$HOME/x\"\n[account.b]\ndir = \"~/x\"\n").unwrap_err();
        assert_eq!(e.to_string(), "a and b use the same folder");
    }

    #[test]
    fn a_symlink_to_another_account_is_the_same_folder() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(tmp.path().join("real")).unwrap();
        std::os::unix::fs::symlink(tmp.path().join("real"), tmp.path().join("alias")).unwrap();
        let (paths, accounts) = accounts(
            tmp.path(),
            "[account.a]\ndir = \"~/real\"\n[account.b]\ndir = \"~/alias\"\n",
        );
        assert!(check_dirs(&paths, &accounts).is_err());
    }
}
