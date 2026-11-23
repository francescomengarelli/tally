use std::fs;

use jiff::SignedDuration;

use super::{snapshot, touch_stamp};
use crate::context::Context;
use crate::usage::Http;

/// Fetch everything again and let the cache keep it; prints nothing.
/// One at a time: if another refresh is running, this one has nothing to do.
pub fn run(ctx: &Context) {
    let _ = fs::create_dir_all(&ctx.paths.cache);
    let Ok(lock) = fs::File::create(ctx.paths.cache.join("refresh.lock")) else {
        return;
    };
    if lock.try_lock().is_err() {
        return;
    }
    touch_stamp(&ctx.paths.cache);
    snapshot(ctx, SignedDuration::ZERO, &Http);
}
