//! Shared entries: symlinks from an account folder into the main one.

use std::collections::HashSet;
use std::fs;
use std::io::ErrorKind;
use std::os::unix::fs::symlink;
use std::path::Path;

use anyhow::{Context, Result, bail};

/// Each account's own login and state: never shared.
const PER_ACCOUNT: [&str; 2] = [".credentials.json", ".claude.json"];

/// Make `account_dir` link every `shared` entry that exists in `main_dir`.
/// Returns warnings. Only symlinks tally made (pointing at
/// `main_dir/<their own name>`) are ever removed; everything else is left.
pub fn reconcile(account_dir: &Path, main_dir: &Path, shared: &[String]) -> Result<Vec<String>> {
    fs::create_dir_all(account_dir)
        .with_context(|| format!("i couldn't create {}", account_dir.display()))?;

    // Linking a folder into itself would replace main's own links with loops.
    if let Ok(main) = fs::canonicalize(main_dir) {
        let account = fs::canonicalize(account_dir)
            .with_context(|| format!("i couldn't resolve {}", account_dir.display()))?;
        if account.starts_with(&main) {
            bail!(
                "i won't link {} to itself: it is the main folder",
                account_dir.display()
            );
        }
    }

    let mut warnings = Vec::new();
    let mut listed = HashSet::new();
    for entry in shared {
        if entry.is_empty() || entry == "." || entry == ".." || entry.contains('/') {
            warnings.push(format!(
                "i skipped '{entry}' in shared: it isn't a plain name"
            ));
            continue;
        }
        if PER_ACCOUNT.contains(&entry.as_str()) {
            warnings.push(format!(
                "i skipped '{entry}' in shared: it belongs to each account"
            ));
            continue;
        }
        listed.insert(entry.as_str());

        let target = main_dir.join(entry);
        let link = account_dir.join(entry);
        let in_main = match fs::metadata(&target) {
            Ok(_) => true,
            Err(e) if e.kind() == ErrorKind::NotFound => false,
            Err(e) => {
                return Err(e).with_context(|| format!("i couldn't check {}", target.display()));
            }
        };
        match link.symlink_metadata() {
            Ok(meta) if meta.is_symlink() => {
                let own = fs::read_link(&link).is_ok_and(|t| t == target);
                if own && in_main {
                    continue;
                }
                if own {
                    // the entry is gone from main: the link is dangling
                    remove(&link)?;
                } else if in_main {
                    if tally_shaped(&link, entry) {
                        remove(&link)?;
                        make(&target, &link)?;
                    } else {
                        warnings.push(format!(
                            "{entry} in {} is a link you made, so i left it",
                            account_dir.display()
                        ));
                    }
                }
            }
            Ok(_) => {
                if in_main {
                    warnings.push(format!(
                        "{entry} in {} is a real file, so i left it",
                        account_dir.display()
                    ));
                }
            }
            Err(e) if e.kind() == ErrorKind::NotFound => {
                if in_main {
                    make(&target, &link)?;
                }
            }
            Err(e) => {
                return Err(e).with_context(|| format!("i couldn't look at {}", link.display()));
            }
        }
    }

    // Our own links for entries no longer listed.
    let dir = fs::read_dir(account_dir)
        .with_context(|| format!("i couldn't read {}", account_dir.display()))?;
    for item in dir {
        let item = item.with_context(|| format!("i couldn't read {}", account_dir.display()))?;
        let name = item.file_name();
        if name
            .to_str()
            .is_some_and(|n| listed.contains(n) || PER_ACCOUNT.contains(&n))
        {
            continue;
        }
        let path = item.path();
        let is_link = path.symlink_metadata().is_ok_and(|m| m.is_symlink());
        if is_link && fs::read_link(&path).is_ok_and(|t| t == main_dir.join(&name)) {
            remove(&path)?;
        }
    }
    Ok(warnings)
}

/// Does the link point at `<something>/.claude/<entry>`: the shape tally
/// makes, so a link left over from a moved home is ours to fix.
fn tally_shaped(link: &Path, entry: &str) -> bool {
    let Ok(target) = fs::read_link(link) else {
        return false;
    };
    let mut parts = target.components().rev();
    parts.next().is_some_and(|c| c.as_os_str() == entry)
        && parts.next().is_some_and(|c| c.as_os_str() == ".claude")
}

fn make(target: &Path, link: &Path) -> Result<()> {
    symlink(target, link)
        .with_context(|| format!("i couldn't link {} to {}", link.display(), target.display()))
}

fn remove(link: &Path) -> Result<()> {
    fs::remove_file(link).with_context(|| format!("i couldn't remove the link {}", link.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    struct Dirs {
        _tmp: tempfile::TempDir,
        main: std::path::PathBuf,
        account: std::path::PathBuf,
    }

    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let main = tmp.path().join("main");
        let account = tmp.path().join("account");
        fs::create_dir(&main).unwrap();
        Dirs {
            _tmp: tmp,
            main,
            account,
        }
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn creates_links_for_existing_entries() {
        let d = dirs();
        fs::write(d.main.join("settings.json"), "{}").unwrap();
        let w = reconcile(&d.account, &d.main, &names(&["settings.json", "commands"])).unwrap();
        assert!(w.is_empty());
        assert_eq!(
            fs::read_link(d.account.join("settings.json")).unwrap(),
            d.main.join("settings.json")
        );
        assert!(d.account.join("commands").symlink_metadata().is_err());
    }

    #[test]
    fn fixes_link_left_by_a_moved_home() {
        let d = dirs();
        fs::write(d.main.join("settings.json"), "{}").unwrap();
        fs::create_dir(&d.account).unwrap();
        symlink(
            "/old-home/.claude/settings.json",
            d.account.join("settings.json"),
        )
        .unwrap();
        let w = reconcile(&d.account, &d.main, &names(&["settings.json"])).unwrap();
        assert!(w.is_empty());
        assert_eq!(
            fs::read_link(d.account.join("settings.json")).unwrap(),
            d.main.join("settings.json")
        );
    }

    #[test]
    fn leaves_real_file_and_warns() {
        let d = dirs();
        fs::write(d.main.join("settings.json"), "{}").unwrap();
        fs::create_dir(&d.account).unwrap();
        fs::write(d.account.join("settings.json"), "mine").unwrap();
        let w = reconcile(&d.account, &d.main, &names(&["settings.json"])).unwrap();
        assert_eq!(
            w,
            vec![format!(
                "settings.json in {} is a real file, so i left it",
                d.account.display()
            )]
        );
        assert_eq!(
            fs::read_to_string(d.account.join("settings.json")).unwrap(),
            "mine"
        );
    }

    #[test]
    fn removes_own_unlisted_link() {
        let d = dirs();
        fs::create_dir(d.main.join("plans")).unwrap();
        fs::create_dir(d.main.join("skills")).unwrap();
        reconcile(&d.account, &d.main, &names(&["plans", "skills"])).unwrap();
        reconcile(&d.account, &d.main, &names(&["skills"])).unwrap();
        assert!(d.account.join("plans").symlink_metadata().is_err());
        assert!(d.account.join("skills").symlink_metadata().is_ok());
    }

    #[test]
    fn keeps_foreign_links_and_files() {
        let d = dirs();
        fs::write(d.main.join("other"), "x").unwrap();
        fs::create_dir(&d.account).unwrap();
        fs::write(d.account.join(".credentials.json"), "secret").unwrap();
        symlink("/elsewhere", d.account.join("mine")).unwrap();
        // points into main but under another name: not an own link
        symlink(d.main.join("other"), d.account.join("alias")).unwrap();
        let w = reconcile(&d.account, &d.main, &names(&["settings.json"])).unwrap();
        assert!(w.is_empty());
        assert_eq!(
            fs::read_to_string(d.account.join(".credentials.json")).unwrap(),
            "secret"
        );
        assert_eq!(
            fs::read_link(d.account.join("mine")).unwrap(),
            Path::new("/elsewhere")
        );
        assert!(d.account.join("alias").symlink_metadata().is_ok());
    }

    #[test]
    fn leaves_a_link_the_user_made_and_says_so() {
        let d = dirs();
        fs::write(d.main.join("settings.json"), "{}").unwrap();
        fs::create_dir(&d.account).unwrap();
        for target in ["/nowhere", "/elsewhere/settings.json", "/x/.claude/other"] {
            let link = d.account.join("settings.json");
            let _ = fs::remove_file(&link);
            symlink(target, &link).unwrap();
            let w = reconcile(&d.account, &d.main, &names(&["settings.json"])).unwrap();
            assert_eq!(
                w,
                vec![format!(
                    "settings.json in {} is a link you made, so i left it",
                    d.account.display()
                )]
            );
            assert_eq!(fs::read_link(&link).unwrap(), Path::new(target));
        }
    }

    #[test]
    fn idempotent() {
        let d = dirs();
        fs::write(d.main.join("settings.json"), "{}").unwrap();
        fs::create_dir(d.main.join("skills")).unwrap();
        let list = names(&["settings.json", "skills", "commands"]);
        reconcile(&d.account, &d.main, &list).unwrap();
        let before = fs::read_link(d.account.join("skills")).unwrap();
        let w = reconcile(&d.account, &d.main, &list).unwrap();
        assert!(w.is_empty());
        assert_eq!(fs::read_link(d.account.join("skills")).unwrap(), before);
    }

    #[test]
    fn removes_dangling_own_link_for_entry_gone_from_main() {
        let d = dirs();
        fs::write(d.main.join("plans"), "x").unwrap();
        reconcile(&d.account, &d.main, &names(&["plans"])).unwrap();
        fs::remove_file(d.main.join("plans")).unwrap();
        let w = reconcile(&d.account, &d.main, &names(&["plans"])).unwrap();
        assert!(w.is_empty());
        assert!(d.account.join("plans").symlink_metadata().is_err());
    }

    #[test]
    fn keeps_wrong_link_to_entry_absent_from_main() {
        let d = dirs();
        fs::create_dir(&d.account).unwrap();
        symlink("/elsewhere", d.account.join("plans")).unwrap();
        reconcile(&d.account, &d.main, &names(&["plans"])).unwrap();
        assert_eq!(
            fs::read_link(d.account.join("plans")).unwrap(),
            Path::new("/elsewhere")
        );
    }

    #[test]
    fn skips_names_that_are_not_plain() {
        let d = dirs();
        fs::write(d.main.join("x"), "x").unwrap();
        let list = names(&["a/b", "..", ".", "", "../x"]);
        let w = reconcile(&d.account, &d.main, &list).unwrap();
        assert_eq!(w.len(), 5);
        assert_eq!(w[0], "i skipped 'a/b' in shared: it isn't a plain name");
        assert_eq!(fs::read_dir(&d.account).unwrap().count(), 0);
    }

    #[test]
    fn real_folder_where_link_belongs_is_left() {
        let d = dirs();
        fs::create_dir(d.main.join("skills")).unwrap();
        fs::create_dir_all(d.account.join("skills")).unwrap();
        fs::write(d.account.join("skills/keep"), "x").unwrap();
        let w = reconcile(&d.account, &d.main, &names(&["skills"])).unwrap();
        assert_eq!(w.len(), 1);
        assert!(d.account.join("skills/keep").exists());
    }

    #[test]
    fn refuses_the_main_folder_itself() {
        let d = dirs();
        symlink("/elsewhere", d.main.join("settings.json")).unwrap();
        let list = names(&["settings.json"]);
        let e = reconcile(&d.main, &d.main, &list).unwrap_err();
        assert_eq!(
            e.to_string(),
            format!(
                "i won't link {} to itself: it is the main folder",
                d.main.display()
            )
        );
        let via = d.main.parent().unwrap().join("via");
        symlink(&d.main, &via).unwrap();
        assert!(reconcile(&via, &d.main, &list).is_err());
        assert_eq!(
            fs::read_link(d.main.join("settings.json")).unwrap(),
            Path::new("/elsewhere")
        );
    }

    #[test]
    fn refuses_a_folder_inside_main() {
        let d = dirs();
        let inner = d.main.join("inner");
        assert!(reconcile(&inner, &d.main, &names(&["x"])).is_err());
    }

    #[test]
    fn skips_per_account_files() {
        let d = dirs();
        fs::write(d.main.join(".credentials.json"), "x").unwrap();
        fs::write(d.main.join(".claude.json"), "x").unwrap();
        fs::create_dir(&d.account).unwrap();
        symlink("/elsewhere", d.account.join(".credentials.json")).unwrap();
        let w = reconcile(
            &d.account,
            &d.main,
            &names(&[".credentials.json", ".claude.json"]),
        )
        .unwrap();
        assert_eq!(
            w,
            vec![
                "i skipped '.credentials.json' in shared: it belongs to each account",
                "i skipped '.claude.json' in shared: it belongs to each account",
            ]
        );
        assert_eq!(
            fs::read_link(d.account.join(".credentials.json")).unwrap(),
            Path::new("/elsewhere")
        );
        assert!(d.account.join(".claude.json").symlink_metadata().is_err());
    }

    #[test]
    fn unreadable_main_removes_nothing() {
        use std::os::unix::fs::PermissionsExt;
        let d = dirs();
        fs::write(d.main.join("plans"), "x").unwrap();
        let list = names(&["plans"]);
        reconcile(&d.account, &d.main, &list).unwrap();
        fs::set_permissions(&d.main, fs::Permissions::from_mode(0o000)).unwrap();
        let r = reconcile(&d.account, &d.main, &list);
        fs::set_permissions(&d.main, fs::Permissions::from_mode(0o755)).unwrap();
        if let Err(e) = r {
            assert!(e.to_string().starts_with("i couldn't check"));
        } else {
            // running as root: permissions don't apply
        }
        assert!(d.account.join("plans").symlink_metadata().is_ok());
    }
}
