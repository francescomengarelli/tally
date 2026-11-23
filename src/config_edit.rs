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
    let (text, existed) = read(path)?;
    let mut doc = parse(path, &text)?;
    if accounts(&mut doc, path)?.contains_key(name) {
        bail!("account '{name}' is already in {}", path.display());
    }
    let mut table = Table::new();
    if let Some(plan) = plan {
        table["plan"] = value(plan);
    }
    accounts(&mut doc, path)?.insert(name, Item::Table(table));
    let mut out = doc.to_string();
    if !existed {
        // the implicit main account goes first, set apart from the new one
        out = format!("{MAIN_ONLY}\n{out}");
    }
    write(path, &out)
}

/// Remove `[account.<name>]` from the config.
pub fn remove_account(path: &Path, name: &str) -> Result<()> {
    validate_name(name)?;
    let (text, _) = read(path)?;
    let mut doc = parse(path, &text)?;
    if accounts(&mut doc, path)?.remove(name).is_none() {
        bail!("there is no account '{name}' in {}", path.display());
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
