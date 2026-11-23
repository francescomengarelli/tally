use anyhow::{Result, bail};

use super::login::login;
use crate::config_edit;
use crate::context::Context;

pub fn run(ctx: &Context, name: &str, plan: Option<f64>) -> Result<()> {
    config_edit::validate_name(name)?;
    if plan.is_some_and(|p| !p.is_finite() || p <= 0.0) {
        bail!("the plan must be a positive number");
    }
    config_edit::add_account(&ctx.paths.config, name, plan)?;
    eprintln!("tally: added {name}");

    // the config changed: log in from what is on disk now
    let ctx = Context::load(Some(ctx.paths.config.clone()))?;
    let account = ctx.account(name).expect("just added");
    login(&ctx, account)
}
