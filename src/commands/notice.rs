use std::io::IsTerminal;

use jiff::SignedDuration;

use super::snapshot;
use crate::context::Context;
use crate::format;
use crate::usage::Http;

/// Shell startup must never fail: no data, nothing printed.
pub fn run(ctx: &Context) {
    let snap = snapshot(ctx, SignedDuration::from_mins(5), &Http);
    if let Some(line) = format::notice(&snap, std::io::stdout().is_terminal()) {
        println!("{line}");
    }
}
