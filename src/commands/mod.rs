pub mod add;
pub mod init;
pub mod login;
pub mod notice;
pub mod pick;
pub mod refresh;
pub mod remove;
pub mod run;
pub mod statusline;
pub mod summary;

use std::ffi::OsString;

use jiff::{SignedDuration, Timestamp};

use crate::config::Account;
use crate::context::Context;
use crate::credentials;
use crate::maths::{self, Metrics, Totals};
use crate::pick::{self as picking, Candidate, Pick};
use crate::usage::{self, Net, Status};

/// The claude binary: `$TALLY_CLAUDE`, else `claude` on PATH.
pub fn claude_bin() -> OsString {
    std::env::var_os("TALLY_CLAUDE").unwrap_or_else(|| "claude".into())
}

pub struct Row {
    pub account: Account,
    pub status: Status,
    /// Weekly multiple of Pro.
    pub plan: f64,
    /// False when the plan was a guess (unknown tier).
    pub plan_known: bool,
    /// Only for accounts with data.
    pub metrics: Option<Metrics>,
}

pub struct Snapshot {
    pub rows: Vec<Row>,
    pub totals: Totals,
    pub pick: Option<Pick>,
}

/// The one place that turns statuses into metrics, totals and a pick.
pub fn snapshot(ctx: &Context, max_age: SignedDuration, net: &dyn Net) -> Snapshot {
    let statuses = usage::gather(&ctx.accounts, &ctx.paths.cache, max_age, net);
    let at = Timestamp::now();

    // Even a cached status needs the login's tier for the plan.
    let logins: Vec<Option<credentials::Oauth>> = ctx
        .accounts
        .iter()
        .map(|a| credentials::read(&a.dir).ok().flatten())
        .collect();
    let plans: Vec<(f64, bool)> = ctx
        .accounts
        .iter()
        .zip(&logins)
        .map(|(a, login)| match a.plan {
            Some(p) => (p, true),
            None => maths::tier_multiple(login.as_ref().and_then(|o| o.rate_limit_tier.as_deref())),
        })
        .collect();
    // no login, no account: a leftover cache doesn't make it usable
    let statuses: Vec<Status> = statuses
        .into_iter()
        .zip(&logins)
        .map(|(s, login)| {
            if login.is_none() {
                Status::NotLoggedIn
            } else {
                s
            }
        })
        .collect();
    let unit = ctx.config.unit(&ctx.accounts).ok().map(|u| u.name.clone());
    let unit_plan = ctx
        .accounts
        .iter()
        .zip(&plans)
        .find(|(a, _)| Some(&a.name) == unit.as_ref())
        .map_or(1.0, |(_, p)| p.0);

    let rows: Vec<Row> = ctx
        .accounts
        .iter()
        .zip(statuses)
        .zip(plans)
        .map(|((account, status), (plan, plan_known))| {
            let metrics = match &status {
                Status::Ready { usage, .. } => Some(maths::metrics(
                    usage,
                    plan / unit_plan,
                    ctx.config.five_hour,
                    at,
                )),
                _ => None,
            };
            Row {
                account: account.clone(),
                status,
                plan,
                plan_known,
                metrics,
            }
        })
        .collect();

    let all: Vec<Metrics> = rows.iter().filter_map(|r| r.metrics.clone()).collect();
    let out = |r: &Row| matches!(r.status, Status::NotLoggedIn | Status::LoginExpired);
    // accounts without a login are never picked, unless none has one
    let usable: Vec<&Row> = match rows.iter().any(|r| !out(r)) {
        true => rows.iter().filter(|r| !out(r)).collect(),
        false => rows.iter().collect(),
    };
    let cands: Vec<Candidate> = usable
        .iter()
        .map(|r| Candidate {
            name: &r.account.name,
            metrics: r.metrics.clone(),
            is_unit: Some(&r.account.name) == unit.as_ref(),
        })
        .collect();
    let pick = picking::pick(&cands, ctx.config.min_session);
    Snapshot {
        totals: maths::totals(&all),
        rows,
        pick,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiError, Tokens, Usage};
    use crate::config::{Config, Paths};

    struct Offline;

    impl Net for Offline {
        fn usage(&self, _: &str) -> Result<Usage, ApiError> {
            Err(ApiError::Other(anyhow::anyhow!("offline")))
        }
        fn refresh(&self, _: &str) -> Result<Tokens, ApiError> {
            Err(ApiError::Other(anyhow::anyhow!("offline")))
        }
    }

    #[test]
    fn offline_picks_the_logged_in_account_over_a_logged_out_unit() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = Paths::from_home(tmp.path(), None);
        let config: Config = toml::from_str(
            "unit = \"acme\"\n[account.acme]\ndir = \"~/.claude\"\n[account.personal]\n",
        )
        .unwrap();
        let accounts = config.accounts(&paths);
        let dir = &accounts[1].dir;
        std::fs::create_dir_all(dir).unwrap();
        let login = r#"{"claudeAiOauth":{"accessToken":"t","expiresAt":99999999999999}}"#;
        std::fs::write(dir.join(".credentials.json"), login).unwrap();
        let ctx = Context {
            paths,
            config,
            accounts,
        };
        let snap = snapshot(&ctx, SignedDuration::from_mins(1), &Offline);
        assert_eq!(snap.pick.unwrap().name, "personal");
    }
}
