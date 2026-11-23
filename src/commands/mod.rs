pub mod add;
pub mod init;
pub mod login;
pub mod pick;
pub mod remove;
pub mod run;

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
    #[allow(dead_code)] // Task 10 (summary)
    pub plan: f64,
    /// False when the plan was a guess (unknown tier).
    #[allow(dead_code)] // Task 10 (summary)
    pub plan_known: bool,
    /// Only for accounts with data.
    pub metrics: Option<Metrics>,
}

pub struct Snapshot {
    pub rows: Vec<Row>,
    #[allow(dead_code)] // Task 10 (summary, notice)
    pub totals: Totals,
    pub pick: Option<Pick>,
}

/// The one place that turns statuses into metrics, totals and a pick.
pub fn snapshot(ctx: &Context, max_age: SignedDuration, net: &dyn Net) -> Snapshot {
    let statuses = usage::gather(&ctx.accounts, &ctx.paths.cache, max_age, net);
    let at = Timestamp::now();

    // Even a cached status needs the login's tier for the plan.
    let plans: Vec<(f64, bool)> = ctx
        .accounts
        .iter()
        .map(|a| match a.plan {
            Some(p) => (p, true),
            None => {
                let tier = credentials::read(&a.dir)
                    .ok()
                    .flatten()
                    .and_then(|o| o.rate_limit_tier);
                maths::tier_multiple(tier.as_deref())
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
    let cands: Vec<Candidate> = rows
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
