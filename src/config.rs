use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SHARED: [&str; 11] = [
    "CLAUDE.md",
    "settings.json",
    "keybindings.json",
    "skills",
    "agents",
    "commands",
    "plugins",
    "projects",
    "plans",
    "history.jsonl",
    "file-history",
];

pub struct Paths {
    pub home: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    pub data: PathBuf,
}

impl Paths {
    pub fn from_home(home: &Path, config: Option<PathBuf>) -> Paths {
        Paths {
            home: home.to_path_buf(),
            config: config.unwrap_or_else(|| home.join(".config/tally/config.toml")),
            cache: home.join(".cache/tally"),
            data: home.join(".local/share/tally"),
        }
    }

    /// The folder Claude uses when `CLAUDE_CONFIG_DIR` is unset.
    pub fn main_dir(&self) -> PathBuf {
        self.home.join(".claude")
    }
}

#[derive(Deserialize)]
#[serde(default)]
pub struct Config {
    pub unit: Option<String>,
    pub five_hour: f64,
    pub min_session: f64,
    pub shared: Vec<String>,
    pub account: BTreeMap<String, AccountEntry>,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            unit: None,
            five_hour: 0.18,
            min_session: 3.0,
            shared: SHARED.iter().map(|s| s.to_string()).collect(),
            account: BTreeMap::new(),
        }
    }
}

#[derive(Deserialize)]
pub struct AccountEntry {
    pub dir: Option<String>,
    pub plan: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Account {
    pub name: String,
    pub dir: PathBuf,
    pub plan: Option<f64>,
    pub is_main: bool,
}

/// Read the config; a missing file is the default config.
pub fn load(paths: &Paths) -> Result<Config> {
    let contents = match std::fs::read_to_string(&paths.config) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
        Err(e) => {
            return Err(e).with_context(|| format!("i couldn't read {}", paths.config.display()));
        }
    };
    toml::from_str(&contents)
        .with_context(|| format!("i couldn't parse {}", paths.config.display()))
}

/// Expand a leading `~` or `$HOME` and drop trailing slashes, so spellings
/// compare equal.
fn expand(dir: &str, home: &Path) -> PathBuf {
    let rest = dir
        .strip_prefix('~')
        .or_else(|| dir.strip_prefix("$HOME"))
        .filter(|rest| rest.is_empty() || rest.starts_with('/'));
    let dir = match rest {
        Some("") => home.to_path_buf(),
        Some(rest) => home.join(rest.trim_start_matches('/')),
        None => PathBuf::from(dir),
    };
    let text = dir.to_string_lossy();
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() {
        dir
    } else {
        PathBuf::from(trimmed)
    }
}

impl Config {
    /// The declared accounts; with none declared, `~/.claude` as `main`.
    pub fn accounts(&self, paths: &Paths) -> Vec<Account> {
        if self.account.is_empty() {
            return vec![Account {
                name: "main".into(),
                dir: paths.main_dir(),
                plan: None,
                is_main: true,
            }];
        }
        self.account
            .iter()
            .map(|(name, entry)| {
                let dir = match &entry.dir {
                    Some(d) => expand(d, &paths.home),
                    None => paths.data.join(name),
                };
                Account {
                    name: name.clone(),
                    is_main: dir == paths.main_dir(),
                    dir,
                    plan: entry.plan,
                }
            })
            .collect()
    }

    /// The account whose plan reads as 100%: `unit`, else main, else the first.
    pub fn unit<'a>(&self, accounts: &'a [Account]) -> Result<&'a Account> {
        match &self.unit {
            Some(name) => accounts
                .iter()
                .find(|a| &a.name == name)
                .with_context(|| format!("i couldn't find the unit account \"{name}\"")),
            None => match accounts.iter().find(|a| a.is_main).or(accounts.first()) {
                Some(a) => Ok(a),
                None => bail!("i have no accounts"),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(toml: Option<&str>) -> (tempfile::TempDir, Paths) {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().to_path_buf();
        let paths = Paths::from_home(&home, None);
        if let Some(t) = toml {
            std::fs::create_dir_all(paths.config.parent().unwrap()).unwrap();
            std::fs::write(&paths.config, t).unwrap();
        }
        (tmp, paths)
    }

    #[test]
    fn missing_file_gives_single_main_account() {
        let (_t, paths) = setup(None);
        let cfg = load(&paths).unwrap();
        let accounts = cfg.accounts(&paths);
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].name, "main");
        assert_eq!(accounts[0].dir, paths.home.join(".claude"));
        assert!(accounts[0].is_main);
        assert_eq!(accounts[0].plan, None);
        assert_eq!(cfg.unit(&accounts).unwrap().name, "main");
    }

    #[test]
    fn defaults_fill_omitted_fields() {
        let (_t, paths) = setup(Some(""));
        let cfg = load(&paths).unwrap();
        assert_eq!(cfg.five_hour, 0.18);
        assert_eq!(cfg.min_session, 3.0);
        assert_eq!(cfg.shared.len(), 11);
        assert_eq!(cfg.shared[0], "CLAUDE.md");
        assert_eq!(cfg.shared[10], "file-history");
    }

    #[test]
    fn account_dir_defaults_to_data_dir() {
        let (_t, paths) = setup(Some("[account.personal]\n"));
        let cfg = load(&paths).unwrap();
        let accounts = cfg.accounts(&paths);
        assert_eq!(accounts.len(), 1);
        assert_eq!(
            accounts[0].dir,
            paths.home.join(".local/share/tally/personal")
        );
        assert!(!accounts[0].is_main);
    }

    #[test]
    fn main_detected_for_any_spelling() {
        let (_t, paths) = setup(None);
        let home = paths.home.display().to_string();
        for dir in ["~/.claude", "~/.claude/", &format!("{home}/.claude")] {
            let toml = format!("[account.work]\ndir = \"{dir}\"\nplan = 6.25\n");
            std::fs::create_dir_all(paths.config.parent().unwrap()).unwrap();
            std::fs::write(&paths.config, toml).unwrap();
            let accounts = load(&paths).unwrap().accounts(&paths);
            assert!(accounts[0].is_main, "{dir}");
            assert_eq!(accounts[0].plan, Some(6.25));
        }
    }

    #[test]
    fn dollar_home_expands_like_tilde() {
        let (_t, paths) = setup(Some(
            "[account.a]\ndir = \"$HOME/.claude\"\n[account.b]\ndir = \"$HOME\"\n[account.c]\ndir = \"$HOMEWORK/x\"\n",
        ));
        let accounts = load(&paths).unwrap().accounts(&paths);
        assert_eq!(accounts[0].dir, paths.home.join(".claude"));
        assert!(accounts[0].is_main);
        assert_eq!(accounts[1].dir, paths.home);
        assert_eq!(accounts[2].dir, PathBuf::from("$HOMEWORK/x"));
    }

    #[test]
    fn unit_defaults_to_main_then_errors_on_unknown() {
        let (_t, paths) = setup(Some("[account.a]\n[account.work]\ndir = \"~/.claude\"\n"));
        let cfg = load(&paths).unwrap();
        let accounts = cfg.accounts(&paths);
        assert_eq!(cfg.unit(&accounts).unwrap().name, "work");

        let (_t, paths) = setup(Some("unit = \"nope\"\n[account.a]\n"));
        let cfg = load(&paths).unwrap();
        let accounts = cfg.accounts(&paths);
        let err = cfg.unit(&accounts).unwrap_err();
        assert!(err.to_string().contains("nope"));
    }
}
