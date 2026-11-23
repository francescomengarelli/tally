use anyhow::{Context, Result, bail};
use std::fs;
use std::path::Path;
use toml_edit::{DocumentMut, Item, Table, value};

/// The file a machine with no config gets: the implicit main account.
const MAIN_ONLY: &str = "[account.main]\ndir = \"~/.claude\"\n";

/// Account names: `^[a-z0-9][a-z0-9-]*$`.
pub fn validate_name(name: &str) -> Result<()> {
    let mut chars = name.chars();
    let first_ok = chars
        .next()
        .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
    if !first_ok || !chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
        bail!("'{name}' isn't a valid account name: use lowercase letters, digits and dashes");
    }
    Ok(())
}

/// Add `[account.<name>]` to the config, keeping everything else as it is.
pub fn add_account(path: &Path, name: &str, plan: Option<f64>) -> Result<()> {
    validate_name(name)?;
    let (text, _) = read(path)?;
    let mut doc = parse(path, &text)?;
    let trailing = doc.trailing().as_str().unwrap_or("").to_string();
    let accounts = accounts(&mut doc, path)?;
    if accounts.contains_key(name) {
        bail!("account '{name}' is already in {}", path.display());
    }
    // With no account declared, main is implicit: keep it explicit now
    let first = accounts.is_empty();
    if first {
        let mut main = Table::new();
        main["dir"] = value("~/.claude");
        // a comment-only file keeps its comments above the new tables
        let gap = if text.trim().is_empty() { "" } else { "\n" };
        main.decor_mut().set_prefix(format!("{trailing}{gap}"));
        accounts.insert("main", Item::Table(main));
    }
    let mut table = Table::new();
    if let Some(plan) = plan {
        table["plan"] = value(plan);
    }
    if first {
        table.decor_mut().set_prefix("\n");
    }
    accounts.insert(name, Item::Table(table));
    if first {
        doc.set_trailing("");
    }
    let out = doc.to_string();
    write(path, &out)
}

/// Remove `[account.<name>]` from the config.
pub fn remove_account(path: &Path, name: &str) -> Result<()> {
    validate_name(name)?;
    let (text, _) = read(path)?;
    let mut doc = parse(path, &text)?;
    let accounts = accounts(&mut doc, path)?;
    let Some(Item::Table(gone)) = accounts.remove(name) else {
        bail!("there is no account '{name}' in {}", path.display());
    };
    // its leading comments belong to the file, not to it: hand them on
    let prefix = gone
        .decor()
        .prefix()
        .and_then(|p| p.as_str())
        .unwrap_or("")
        .to_string();
    let after = gone.position();
    let next = accounts
        .iter_mut()
        .filter_map(|(_, item)| item.as_table_mut())
        .filter(|t| t.position() > after)
        .min_by_key(|t| t.position());
    match next {
        Some(next) => {
            let old = next.decor().prefix().and_then(|p| p.as_str()).unwrap_or("");
            let joined = format!("{prefix}{old}");
            next.decor_mut().set_prefix(joined);
        }
        None => {
            let trailing = format!("{}{prefix}", doc.trailing().as_str().unwrap_or(""));
            doc.set_trailing(trailing);
        }
    }
    write(path, &doc.to_string())
}

/// The file's text, and whether it existed. A missing file reads as the
/// implicit main account only (and is parsed without it, see `add_account`).
fn read(path: &Path) -> Result<(String, bool)> {
    match fs::read_to_string(path) {
        Ok(text) => Ok((text, true)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((String::new(), false)),
        Err(e) => Err(e).with_context(|| format!("i couldn't read {}", path.display())),
    }
}

fn parse(path: &Path, text: &str) -> Result<DocumentMut> {
    text.parse()
        .with_context(|| format!("i couldn't read {}", path.display()))
}

/// The `account` table, created (implicit, so children render as
/// `[account.<name>]`) when absent.
fn accounts<'a>(doc: &'a mut DocumentMut, path: &Path) -> Result<&'a mut Table> {
    let item = doc.entry("account").or_insert_with(|| {
        let mut t = Table::new();
        t.set_implicit(true);
        Item::Table(t)
    });
    item.as_table_mut().with_context(|| {
        format!(
            "i couldn't read {}: 'account' is not a table",
            path.display()
        )
    })
}

/// Write through `path` itself, so a symlink stays a symlink.
fn write(path: &Path, text: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("i couldn't create {}", parent.display()))?;
    }
    fs::write(path, text).with_context(|| format!("i couldn't write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# my tally\nunit = \"main\"\n\n[account.main]\ndir = \"~/.claude\" # home\n\n[account.work]\ndir = \"~/w\"\nplan = 5.0\n\n# tail comment\n";

    fn file(text: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("config.toml");
        fs::write(&p, text).unwrap();
        (d, p)
    }

    #[test]
    fn rejects_bad_names() {
        for bad in ["My Account", "../x", "-a", ""] {
            assert!(validate_name(bad).is_err(), "{bad}");
        }
        for ok in ["personal", "work-2"] {
            assert!(validate_name(ok).is_ok(), "{ok}");
        }
        let e = validate_name("A").unwrap_err().to_string();
        assert_eq!(
            e,
            "'A' isn't a valid account name: use lowercase letters, digits and dashes"
        );
    }

    #[test]
    fn add_keeps_comments_and_order() {
        let (_d, p) = file(SAMPLE);
        add_account(&p, "personal", None).unwrap();
        let out = fs::read_to_string(&p).unwrap();
        let head = SAMPLE.split("# tail comment").next().unwrap().trim_end();
        assert!(out.starts_with(head), "{out}");
        assert!(out.contains("# tail comment"), "{out}");
        assert!(out.contains("[account.personal]"), "{out}");
        assert!(!out.contains("[account]\n"), "{out}");
        let personal = out.find("[account.personal]").unwrap();
        assert!(personal > out.find("[account.work]").unwrap());
    }

    #[test]
    fn add_writes_plan() {
        let (_d, p) = file(SAMPLE);
        add_account(&p, "personal", Some(10.0)).unwrap();
        let out = fs::read_to_string(&p).unwrap();
        assert!(out.contains("[account.personal]\nplan = 10.0"), "{out}");
    }

    #[test]
    fn add_to_missing_file_keeps_main() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("nested/tally/config.toml");
        add_account(&p, "personal", None).unwrap();
        let out = fs::read_to_string(&p).unwrap();
        assert_eq!(
            out,
            "[account.main]\ndir = \"~/.claude\"\n\n[account.personal]\n"
        );
        let doc: toml::Table = out.parse().unwrap();
        assert_eq!(doc["account"]["main"]["dir"].as_str(), Some("~/.claude"));
        assert!(doc["account"].as_table().unwrap().contains_key("personal"));
    }

    #[test]
    fn add_through_symlink() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("dotfiles-config.toml");
        fs::write(&target, SAMPLE).unwrap();
        let link = d.path().join("config.toml");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        add_account(&link, "personal", None).unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert!(
            fs::read_to_string(&target)
                .unwrap()
                .contains("[account.personal]")
        );
    }

    #[test]
    fn add_existing_errors() {
        let (_d, p) = file(SAMPLE);
        let e = add_account(&p, "work", None).unwrap_err().to_string();
        assert_eq!(e, format!("account 'work' is already in {}", p.display()));
        assert_eq!(fs::read_to_string(&p).unwrap(), SAMPLE);
    }

    fn names(p: &Path) -> Vec<String> {
        let home = p.parent().unwrap();
        let paths = crate::config::Paths::from_home(home, Some(p.to_path_buf()));
        let cfg = crate::config::load(&paths).unwrap();
        cfg.accounts(&paths).into_iter().map(|a| a.name).collect()
    }

    #[test]
    fn add_to_empty_or_settings_only_file_keeps_main() {
        for text in ["", "# nothing yet\n", "unit = \"main\"\nfive_hour = 0.2\n"] {
            let (_d, p) = file(text);
            add_account(&p, "personal", None).unwrap();
            assert_eq!(names(&p), ["main", "personal"], "{text:?}");
            let out = fs::read_to_string(&p).unwrap();
            assert!(out.starts_with(text), "{out}");
            let sep = if text.is_empty() { "" } else { "\n" };
            let want = format!("{sep}[account.main]\ndir = \"~/.claude\"\n\n[account.personal]");
            assert!(out.contains(&want), "{out}");
        }
    }

    #[test]
    fn remove_first_keeps_header_comments() {
        let (_d, p) = file(
            "# tally: my accounts\n# see docs\n\n[account.work]\ndir = \"~/w\"\n\n[account.main]\ndir = \"~/.claude\"\n",
        );
        remove_account(&p, "work").unwrap();
        let out = fs::read_to_string(&p).unwrap();
        assert!(
            out.starts_with("# tally: my accounts\n# see docs\n"),
            "{out}"
        );
        assert!(!out.contains("work"), "{out}");
        let (_d, p) = file("# top\n[account.work]\ndir = \"~/w\"\n");
        remove_account(&p, "work").unwrap();
        assert!(fs::read_to_string(&p).unwrap().contains("# top"));
    }

    #[test]
    fn add_then_remove_round_trips_through_load() {
        let (_d, p) = file(SAMPLE);
        add_account(&p, "personal", Some(10.0)).unwrap();
        let paths = crate::config::Paths::from_home(p.parent().unwrap(), Some(p.clone()));
        let load = || crate::config::load(&paths).unwrap().accounts(&paths);
        let got: Vec<_> = load().into_iter().map(|a| (a.name, a.plan)).collect();
        assert_eq!(
            got,
            [
                ("main".into(), None),
                ("personal".into(), Some(10.0)),
                ("work".into(), Some(5.0)),
            ]
        );
        remove_account(&p, "personal").unwrap();
        let after = load();
        let got: Vec<_> = after.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(got, ["main", "work"]);
        assert!(after[0].dir.ends_with(".claude") && after[1].dir.ends_with("w"));
    }

    #[test]
    fn remove_drops_section() {
        let (_d, p) = file(SAMPLE);
        remove_account(&p, "work").unwrap();
        let out = fs::read_to_string(&p).unwrap();
        assert!(!out.contains("work"), "{out}");
        assert!(
            out.contains("[account.main]") && out.contains("# my tally"),
            "{out}"
        );
        let e = remove_account(&p, "work").unwrap_err().to_string();
        assert_eq!(e, format!("there is no account 'work' in {}", p.display()));
    }
}
