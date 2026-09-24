//! Emulator-only global allocator for the Ironwood crates: the device image
//! carves from two arenas (`allocator.rs`); the unix emulator delegates to the
//! C allocator, so the tiers the handler installs are accepted and ignored.
//!
//! This is why no emulator run prices either tier. The arenas are modelled on
//! the host instead: `ironwood/tests/rooted_tier.rs` and `scratch_tier.rs` run
//! the device's own arena code under the real allocation traces.

use core::alloc::{GlobalAlloc, Layout};

unsafe extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(pointer: *mut u8);
}

/// Mirrors the device constant so the handler's buffer check is the same on
/// both targets.
pub const SCRATCH_BYTES: usize = 48 * 1024;

/// Same entry points as the device allocator so the bindings are
/// target-agnostic. The emulator's Rust statics live in the never-collected
/// process heap, so there is no tier to root, lend back or wipe.
pub fn install_region() {}

/// Nothing to forbid: the emulator installs no tier, so there is never one
/// left over from a workflow that skipped its `finally`. The device's
/// counterpart is fatal, and the mechanism behind it is unit-tested on the
/// host in `ironwood::arena`.
pub fn forbid_installed_scratch() {}

/// # Safety
///
/// Signature parity with the device allocator; the buffer is not retained.
pub unsafe fn install_scratch(base: *mut u8, len: usize) -> bool {
    !base.is_null() && len >= SCRATCH_BYTES
}

pub fn release_scratch() {}

struct SystemAllocator;

// SAFETY: the emulator is single-threaded and the C allocator returns 16-byte
// aligned blocks, which covers every alignment the Ironwood crates request.
unsafe impl GlobalAlloc for SystemAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > 16 {
            return core::ptr::null_mut();
        }
        unsafe { malloc(layout.size()) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        unsafe { free(pointer) }
    }
}

#[global_allocator]
static IRONWOOD_UNIX_ALLOCATOR: SystemAllocator = SystemAllocator;
