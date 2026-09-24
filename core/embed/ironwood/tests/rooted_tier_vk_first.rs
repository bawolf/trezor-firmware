#![cfg(feature = "test")]
//! Cold order: `ZcashGetViewingKey` first, which builds Pasta's table through
//! the viewing-key derivation. Then an address, `prewarm` and a warm receive.
//! See `device_arena`.

mod common;
mod device_arena;

use device_arena::Cold::*;

#[test]
fn the_tiers_hold_a_boot_that_starts_with_a_viewing_key() {
    device_arena::boot(&[ViewingKey, Receive, Prewarm, Receive]);
}
