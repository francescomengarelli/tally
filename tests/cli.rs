//! End to end: the real binary, a temp HOME and a fake claude. No network:
//! every account that is asked for usage has a fresh cache file.

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

const CONFIG: &str = "unit = \"acme\"\n\n[account.acme]\ndir = \"~/.claude\"\nplan = 5.0\n\n[account.personal]\n";

struct Env {
    home: tempfile::TempDir,
}

impl Env {
    fn new(config: &str) -> Env {
        let home = tempfile::tempdir().unwrap();
        let env = Env { home };
        if !config.is_empty() {
            let path = env.path(".config/tally/config.toml");
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, config).unwrap();
        }
        fs::create_dir_all(env.path(".claude")).unwrap();
        env
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.home.path().join(rel)
    }

    fn tally(&self) -> Command {
        let mut cmd = Command::cargo_bin("tally").unwrap();
        let fake = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/fake-claude");
        cmd.env("HOME", self.home.path())
            .env("TALLY_CLAUDE", fake)
            .env_remove("CLAUDE_CONFIG_DIR")
            .env_remove("TALLY_ACCOUNT");
        cmd
    }

    /// A fresh cache entry and a login: `five` and `week` percent used, week resets in 3 days.
    fn cache(&self, name: &str, five: f64, week: f64) {
        self.cache_only(name, five, week);
        let dir = if name == "acme" {
            self.path(".claude")
        } else {
            self.path(&format!(".local/share/tally/{name}"))
        };
        fs::create_dir_all(&dir).unwrap();
        let login = r#"{"claudeAiOauth":{"accessToken":"t","refreshToken":"r","expiresAt":99999999999999}}"#;
        fs::write(dir.join(".credentials.json"), login).unwrap();
    }

    /// A fresh cache entry with no login behind it.
    fn cache_only(&self, name: &str, five: f64, week: f64) {
        // an hour ahead, so a slow run never counts it as old and refreshes
        let now = jiff::Timestamp::now();
        let fetched = jiff::Timestamp::from_second(now.as_second() + 3600).unwrap();
        let in_hours = |h: i64| jiff::Timestamp::from_second(now.as_second() + h * 3600).unwrap();
        let body = serde_json::json!({
            "fetched_at": fetched,
            "usage": {
                "five_hour": {"utilization": five, "resets_at": in_hours(3)},
                "seven_day": {"utilization": week, "resets_at": in_hours(72)},
            },
        });
        let path = self.path(&format!(".cache/tally/{name}.json"));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body.to_string()).unwrap();
    }
}

#[test]
fn run_named_account_sets_env_and_args() {
    let env = Env::new(CONFIG);
    fs::write(env.path(".claude/settings.json"), "{}").unwrap();
    let dir = env.path(".local/share/tally/personal");
    env.tally()
        .args(["run", "personal", "--", "-c", "x"])
        .assert()
        .success()
        .stdout(format!(
            "CLAUDE_CONFIG_DIR={}\nTALLY_ACCOUNT=personal\n-c\nx\n",
            dir.display()
        ));
    let link = dir.join("settings.json");
    assert!(link.symlink_metadata().unwrap().is_symlink());
    assert_eq!(
        fs::read_link(link).unwrap(),
        env.path(".claude/settings.json")
    );
}

#[test]
fn run_main_has_no_config_dir() {
    let env = Env::new(CONFIG);
    env.tally()
        .env("CLAUDE_CONFIG_DIR", "/leaked")
        .args(["run", "acme", "--", "hi"])
        .assert()
        .success()
        .stdout("CLAUDE_CONFIG_DIR=\nTALLY_ACCOUNT=acme\nhi\n");
}

#[test]
fn run_unknown_account_errors() {
    let env = Env::new(CONFIG);
    env.tally()
        .args(["run", "nope"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("nope").and(predicate::str::contains("acme, personal")));
}

#[test]
fn run_auto_uses_pick() {
    let env = Env::new(CONFIG);
    // acme is nearly out of its 5-hour window, personal is untouched.
    env.cache("acme", 90.0, 90.0);
    env.cache("personal", 0.0, 0.0);
    env.tally()
        .args(["run", "--", "go"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TALLY_ACCOUNT=personal\ngo\n"))
        .stderr(predicate::str::contains("tally: personal"));
}

#[test]
fn run_warns_when_all_are_near_a_limit() {
    let env = Env::new(CONFIG);
    env.cache("acme", 99.0, 99.0);
    env.cache("personal", 99.0, 99.0);
    env.tally()
        .args(["run"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "tally: all accounts are near a limit; ",
        ))
        .stderr(predicate::str::contains(" frees up at "));
}

#[test]
fn pick_prints_the_name() {
    let env = Env::new(CONFIG);
    env.cache("acme", 90.0, 90.0);
    env.cache("personal", 0.0, 0.0);
    env.tally()
        .arg("pick")
        .assert()
        .success()
        .stdout("personal\n");
}

#[test]
fn pick_why_names_rule() {
    let env = Env::new(CONFIG);
    env.cache("acme", 90.0, 90.0);
    env.cache("personal", 0.0, 0.0);
    env.tally()
        .args(["pick", "--why"])
        .assert()
        .success()
        .stdout(
            predicate::str::is_match(r"(?m)^acme +now 2%  week 10%  resets in 3\.0d  3%/day$")
                .unwrap()
                .and(predicate::str::contains("personal  "))
                .and(predicate::str::contains("picked personal: most urgent")),
        );
}

#[test]
fn login_seeds_claude_json() {
    let env = Env::new(CONFIG);
    fs::write(
        env.path(".claude.json"),
        r#"{"mcpServers":{"runpod":{"url":"x"}},"theme":"dark","userID":"secret"}"#,
    )
    .unwrap();
    env.tally()
        .args(["login", "personal"])
        .assert()
        .success()
        .stdout(predicate::str::contains("auth\nlogin\n"));
    let seeded: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(env.path(".local/share/tally/personal/.claude.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        seeded,
        serde_json::json!({"mcpServers":{"runpod":{"url":"x"}},"theme":"dark"})
    );
}

#[test]
fn login_keeps_an_existing_claude_json() {
    let env = Env::new(CONFIG);
    fs::write(env.path(".claude.json"), r#"{"theme":"dark"}"#).unwrap();
    let dir = env.path(".local/share/tally/personal");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(".claude.json"), r#"{"theme":"light"}"#).unwrap();
    env.tally().args(["login", "personal"]).assert().success();
    assert_eq!(
        fs::read_to_string(dir.join(".claude.json")).unwrap(),
        r#"{"theme":"light"}"#
    );
}

#[test]
fn login_main_seeds_nothing() {
    let env = Env::new(CONFIG);
    fs::write(env.path(".claude.json"), r#"{"theme":"dark"}"#).unwrap();
    env.tally()
        .args(["login", "acme"])
        .assert()
        .success()
        .stdout(predicate::str::contains("CLAUDE_CONFIG_DIR=\n"));
    assert!(!env.path(".claude/.claude.json").exists());
}

#[test]
fn add_writes_config_then_logs_in() {
    let env = Env::new(CONFIG);
    env.tally()
        .args(["add", "work", "--plan", "6.25"])
        .assert()
        .success()
        .stdout(
            predicate::str::contains("TALLY_ACCOUNT=")
                .and(predicate::str::contains("auth\nlogin\n")),
        );
    let config = fs::read_to_string(env.path(".config/tally/config.toml")).unwrap();
    assert!(config.contains("[account.work]\nplan = 6.25"), "{config}");
    assert!(env.path(".local/share/tally/work").is_dir());
}

#[test]
fn add_rejects_bad_name() {
    let env = Env::new("");
    env.tally()
        .args(["add", "Bad Name"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("isn't a valid account name"));
    assert!(!env.path(".config/tally/config.toml").exists());
}

#[test]
fn add_rejects_bad_plan() {
    let env = Env::new("");
    env.tally()
        .args(["add", "work", "--plan", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the plan must be a positive number",
        ));
    assert!(!env.path(".config/tally/config.toml").exists());
}

#[test]
fn remove_off_a_tty_keeps_the_folder() {
    let env = Env::new(CONFIG);
    let dir = env.path(".local/share/tally/personal");
    fs::create_dir_all(&dir).unwrap();
    env.tally()
        .args(["remove", "personal"])
        .assert()
        .success()
        .stderr(predicate::str::contains("not a terminal"));
    assert!(dir.is_dir());
    let config = fs::read_to_string(env.path(".config/tally/config.toml")).unwrap();
    assert!(!config.contains("personal"), "{config}");
}

#[test]
fn remove_never_touches_the_main_folder() {
    let env = Env::new(&CONFIG.replace("unit = \"acme\"", "unit = \"personal\""));
    env.tally().args(["remove", "acme"]).assert().success();
    assert!(env.path(".claude").is_dir());
}

#[test]
fn bad_account_name_in_config_is_named() {
    let env = Env::new("[account.\"Bad One\"]\n");
    env.tally()
        .arg("pick")
        .assert()
        .failure()
        .stderr(predicate::str::contains("Bad One"));
}

#[test]
fn init_fish_lists_accounts() {
    let env = Env::new(CONFIG);
    env.tally().args(["init", "fish"]).assert().success().stdout(
        "function cl --wraps claude --description 'claude on the account tally picks'\n    tally run -- $argv\nend\n\
function cl-acme --wraps claude --description 'claude as acme'\n    tally run acme -- $argv\nend\n\
function cl-personal --wraps claude --description 'claude as personal'\n    tally run personal -- $argv\nend\n",
    );
}

#[test]
fn fresh_cache_without_a_login_is_not_picked() {
    let env = Env::new(CONFIG);
    env.cache("acme", 90.0, 90.0);
    env.cache_only("personal", 0.0, 0.0);
    env.tally()
        .arg("pick")
        .assert()
        .success()
        .stdout("acme\n");
}

#[test]
fn run_named_logged_out_warns_and_still_runs() {
    let env = Env::new(CONFIG);
    env.tally()
        .args(["run", "personal", "--", "hi"])
        .assert()
        .success()
        .stdout(predicate::str::contains("TALLY_ACCOUNT=personal\nhi\n"))
        .stderr(predicate::str::contains(
            "tally: personal is not logged in → tally login personal",
        ));
}

#[test]
fn errors_are_terse() {
    let env = Env::new(CONFIG);
    env.tally().args(["run", "nope"]).assert().failure().stderr(
        predicate::str::starts_with("tally: ").and(predicate::str::contains("Error:").not()),
    );
}

#[test]
fn plan_in_config_must_be_positive() {
    let env = Env::new("[account.a]\nplan = 0\n");
    env.tally()
        .arg("pick")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "the plan for a must be a positive number",
        ));
}

#[test]
fn add_refuses_the_implicit_main() {
    let env = Env::new("");
    env.tally()
        .args(["add", "main"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("account 'main' is already in"));
    assert!(!env.path(".config/tally/config.toml").exists());
}

#[test]
fn remove_refuses_the_unit_account() {
    let env = Env::new(CONFIG);
    env.tally()
        .args(["remove", "acme"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "acme is the unit account; change unit in ",
        ));
    assert_eq!(
        fs::read_to_string(env.path(".config/tally/config.toml")).unwrap(),
        CONFIG
    );
}

#[test]
fn remove_a_non_unit_account_leaves_pick_working() {
    let env = Env::new(CONFIG);
    env.cache("acme", 10.0, 10.0);
    env.tally().args(["remove", "personal"]).assert().success();
    env.tally()
        .arg("pick")
        .assert()
        .success()
        .stdout("acme\n");
}

#[test]
fn notice_offline_prints_nothing_and_succeeds() {
    let env = Env::new(CONFIG);
    env.tally()
        .arg("notice")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn notice_with_a_broken_config_prints_nothing_and_succeeds() {
    let env = Env::new("this is not toml [");
    env.tally()
        .arg("notice")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn notice_prints_the_totals_and_the_pick() {
    let env = Env::new(CONFIG);
    env.cache("acme", 0.0, 0.0);
    env.cache("personal", 0.0, 0.0);
    env.tally()
        .arg("notice")
        .assert()
        .success()
        .stdout(predicate::str::contains(" claude: now ").and(predicate::str::contains("→  cl (")));
}

#[test]
fn statusline_reads_env_account() {
    let env = Env::new(CONFIG);
    env.cache("acme", 0.0, 0.0);
    env.cache("personal", 0.0, 0.0);
    env.tally()
        .env("TALLY_ACCOUNT", "personal")
        .arg("statusline")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("personal · now "));
}

#[test]
fn statusline_reads_config_dir_account() {
    let env = Env::new(CONFIG);
    env.cache("acme", 0.0, 0.0);
    env.cache("personal", 0.0, 0.0);
    env.tally()
        .env("CLAUDE_CONFIG_DIR", env.path(".local/share/tally/personal"))
        .arg("statusline")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("personal · now "));
    env.tally()
        .arg("statusline")
        .assert()
        .success()
        .stdout(predicate::str::starts_with("acme · now "));
}

#[test]
fn statusline_without_data_prints_nothing_and_succeeds() {
    let env = Env::new(CONFIG);
    env.tally()
        .arg("statusline")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
}

#[test]
fn summary_prints_a_table_with_a_total() {
    let env = Env::new(CONFIG);
    env.cache("acme", 10.0, 20.0);
    env.cache("personal", 0.0, 0.0);
    env.tally().assert().success().stdout(
        predicate::str::starts_with("account")
            .and(predicate::str::contains("acme"))
            .and(predicate::str::contains("total")),
    );
}

#[test]
fn refresh_exits_quietly_when_another_refresh_holds_the_lock() {
    let env = Env::new(CONFIG);
    let cache = env.path(".cache/tally");
    fs::create_dir_all(&cache).unwrap();
    let held = fs::File::create(cache.join("refresh.lock")).unwrap();
    held.lock().unwrap();
    env.tally()
        .arg("refresh")
        .assert()
        .success()
        .stdout(predicate::str::is_empty());
    assert!(!cache.join("refresh.stamp").exists());
    drop(held);
    env.tally().arg("refresh").assert().success();
    assert!(cache.join("refresh.stamp").exists());
}
