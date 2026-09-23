//! Emulator-only global allocator for the Ironwood crates: the device image
//! carves from a Python-owned region (`allocator.rs`); the unix
//! emulator delegates to the C allocator, so the region the handler passes is
//! accepted and ignored.

use core::alloc::{GlobalAlloc, Layout};

unsafe extern "C" {
    fn malloc(size: usize) -> *mut u8;
    fn free(pointer: *mut u8);
}

/// Same entry point as the device allocator so the binding is target-agnostic.
/// The emulator delegates to C `malloc`, and its Rust statics live in the
/// never-collected process heap, so there is no region to install.
pub fn install_region() {}

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
