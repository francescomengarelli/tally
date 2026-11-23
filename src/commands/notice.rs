use std::io::IsTerminal;

use jiff::SignedDuration;

use super::{refresh_in_background, snapshot};
use crate::context::Context;
use crate::format;
use crate::usage::{Http, NoRenew};

/// Shell startup must never fail or wait on a lock: no data, nothing printed.
/// Expired tokens are left to a background refresh.
pub fn run(ctx: &Context) {
    let snap = snapshot(ctx, SignedDuration::from_mins(5), &NoRenew(&Http));
    refresh_in_background(ctx, &snap);
    if let Some(line) = format::notice(&snap, format::colour(std::io::stdout().is_terminal())) {
        println!("{line}");
    }
}
