use jiff::SignedDuration;

use super::snapshot;
use crate::context::Context;
use crate::usage::Http;

/// Fetch everything again and let the cache keep it; prints nothing.
pub fn run(ctx: &Context) {
    snapshot(ctx, SignedDuration::ZERO, &Http);
}
