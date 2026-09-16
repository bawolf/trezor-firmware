#![no_std]
#![no_main]
#![cfg_attr(
    feature = "ironwood_target_native_compile_only",
    feature(alloc_error_handler)
)]

// Callable synthetic module for link measurement; requires a trusted caller.
// sign internally approves the matching token. No boot invocation or wire handler;
// this native-only slice supplies no trusted Python route.
#[cfg(feature = "ironwood_target_native_compile_only")]
pub mod arena;
#[cfg(feature = "ironwood_target_native_compile_only")]
mod arena_startup;
#[cfg(feature = "ironwood_target_native_compile_only")]
pub mod public_table;
#[cfg(feature = "ironwood_target_native_compile_only")]
use arena_startup::{exit_code, fail_stop, require};
#[cfg(feature = "ironwood_target_native_compile_only")]
use ironwood_emulator_bridge as _;

#[cfg(all(
    feature = "ironwood_target_native_compile_only",
    feature = "production"
))]
compile_error!("synthetic target integration is forbidden in production");

// force pull in Rust generated symbols (incl. the panic handler)
use sys as _;
use trezor_lib as _;
