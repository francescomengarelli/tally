use std::io::IsTerminal;

use anyhow::Result;
use jiff::SignedDuration;

use super::snapshot;
use crate::context::Context;
use crate::format;
use crate::usage::Http;

pub fn run(ctx: &Context) -> Result<()> {
    let snap = snapshot(ctx, SignedDuration::from_mins(1), &Http);
    println!(
        "{}",
        format::table(&snap, format::colour(std::io::stdout().is_terminal()))
    );
    Ok(())
}
