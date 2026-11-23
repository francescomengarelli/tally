use anyhow::Result;
use jiff::SignedDuration;

use super::snapshot;
use crate::context::Context;
use crate::pick::Rule;
use crate::usage::{Http, Status};

pub fn run(ctx: &Context, why: bool) -> Result<()> {
    let snap = snapshot(ctx, SignedDuration::from_mins(1), &Http);
    let pick = snap.pick.expect("at least one account");
    if !why {
        println!("{}", pick.name);
        return Ok(());
    }
    let width = snap
        .rows
        .iter()
        .map(|r| r.account.name.len())
        .max()
        .unwrap_or(0);
    for row in &snap.rows {
        let name = &row.account.name;
        match &row.metrics {
            Some(m) => println!(
                "{name:width$}  now {:.0}%  week {:.0}%  resets in {:.1}d  {:.0}%/day",
                m.now, m.week, m.days_to_reset, m.day
            ),
            None => println!("{name:width$}  {}", missing(&row.status, name)),
        }
    }
    let rule = match pick.rule {
        Rule::MostUrgent => "most urgent: the most week to spend per day, with room for a session",
        Rule::NoneEligible => "none has room for a session: the one with the most room",
        Rule::NoData => "no data: the unit account",
    };
    println!("picked {}: {rule}", pick.name);
    Ok(())
}

fn missing(status: &Status, name: &str) -> String {
    match status {
        Status::NotLoggedIn => format!("not logged in → tally login {name}"),
        Status::LoginExpired => format!("login expired → tally login {name}"),
        _ => "unavailable".into(),
    }
}
