//! One core-app executor, a sealed public-table baseline, and request cleanup.
use crate::arena::ALLOCATOR;
use core::alloc::Layout;

pub(crate) mod exit_code {
    pub const PREINIT: i32 = 81;
    pub const DOUBLE_INIT: i32 = 82;
    pub const REENTRY: i32 = 83;
    pub const INVARIANT: i32 = 84;
    pub const OOM: i32 = 86;
    pub const WRONG_THREAD: i32 = 88;
    pub const TEST_FAILURE: i32 = 90;
}

#[cold]
#[inline(never)]
pub(crate) fn fail_stop(_code: i32) -> ! {
    // Same immediate Thumb trap as the retained target allocator diagnostic.
    // UNIX exit codes are not target postmortem codes. No VM, GC or unwinding.
    unsafe { core::arch::asm!("udf #0", options(noreturn, nomem, nostack)) }
}

pub(crate) fn require(condition: bool) {
    if !condition {
        fail_stop(exit_code::INVARIANT);
    }
}

#[alloc_error_handler]
fn allocation_error(_: Layout) -> ! {
    fail_stop(exit_code::OOM)
}

#[unsafe(no_mangle)]
pub extern "C" fn ironwood_target_arena_init() {
    ALLOCATOR.init();
}

#[unsafe(no_mangle)]
pub extern "C" fn ironwood_target_arena_check_cold() {
    ALLOCATOR.require_empty();
    require(ALLOCATOR.snapshot().attempts == 0);
}

/// Called by each module entry and every GlobalAlloc allocation/deallocation.
#[unsafe(no_mangle)]
pub extern "C" fn ironwood_bridge_require_executor() {
    unsafe extern "C" {
        fn ironwood_is_executor() -> bool;
    }
    if !unsafe { ironwood_is_executor() } {
        fail_stop(exit_code::WRONG_THREAD);
    }
}

/// C has cancelled the old request before calling this initializer.
#[unsafe(no_mangle)]
pub extern "C" fn ironwood_bridge_prepare() {
    ironwood_bridge_require_executor();
    if ALLOCATOR.public_baseline().is_none() {
        // Do not adopt any earlier allocation, even if it has already been freed.
        require(ALLOCATOR.snapshot().attempts == 0);
    }
    let _ = crate::public_table::initialize();
}

/// No request owners may survive sign/cancel or the final C shutdown gate.
/// Normal target teardown calls this on the bound VM executor, before mp_deinit.
#[unsafe(no_mangle)]
pub extern "C" fn ironwood_bridge_require_idle() {
    ironwood_bridge_require_executor();
    if let Some(baseline) = ALLOCATOR.public_baseline() {
        ALLOCATOR.require_baseline(baseline);
    } else {
        ironwood_target_arena_check_cold();
    }
}
