#![cfg(feature = "test")]
//! Cold order: `ZcashGetAddress` first, which is what faulted on the first
//! Safe 5 boot -- the receiver derivation builds Pasta's table cold. Then a
//! viewing key, a signing session's `prewarm`, and a warm receive. See
//! `device_arena` for what each order asserts.

mod common;
mod device_arena;

use device_arena::Cold::*;

#[test]
fn the_tiers_hold_a_boot_that_starts_with_an_address() {
    device_arena::boot(&[Receive, ViewingKey, Prewarm, Receive]);
}
