use anyhow::{Context as _, Result};
use std::path::PathBuf;

use crate::config::{self, Account, Config, Paths};
use crate::config_edit::validate_name;

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
        let accounts = config.accounts(&paths);
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
