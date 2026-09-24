#![cfg(feature = "test")]
//! Cold order: `ZcashSignPczt` first, so `prewarm` builds Pasta's table and
//! orchard's caches before anything else has touched the tier. Then an
//! address, a viewing key and a warm receive. See `device_arena`.

mod common;
mod device_arena;

use device_arena::Cold::*;

#[test]
fn the_tiers_hold_a_boot_that_starts_with_a_sign() {
    device_arena::boot(&[Prewarm, Receive, ViewingKey, Receive]);
}
