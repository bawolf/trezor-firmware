//! Global allocator for the Ironwood signing core on the device.
//!
//! The streaming core allocates a little (Sinsemilla pads into a `Vec<bool>`,
//! `hash_to_curve` boxes a closure, Pasta builds its square-root table on
//! first use; design §7). Those blocks must live in memory the MicroPython
//! collector never touches: a GC-backed allocator freed Pasta's table, which
//! lives behind a Rust static the collector does not scan, and Pasta trapped.
//!
//! So the allocator is a first-fit free list inside a region the Python
//! signing handler owns: a `bytearray` it allocates before the session and
//! keeps referenced until the session is cancelled. Blocks are split on
//! allocation and coalesced with their successors on free, so temporary
//! vectors (Pasta grows four 256-element vectors while building the table)
//! do not exhaust the region.
//!
//! Known limit (design §7, phase-3 report): Pasta's square-root table is a
//! `lazy_static` initialised once per boot, so it stays valid only while the
//! region that holds it is alive. The current handler releases the region
//! after each session; a second session in the same boot installs a fresh
//! region and the table pointer would be stale. Rooting one region for the
//! process lifetime is the open item before hardware use.

use core::alloc::{GlobalAlloc, Layout};
use core::ptr;

static mut REGION_BASE: *mut u8 = ptr::null_mut();
static mut REGION_LEN: usize = 0;
static mut REGION_PEAK: usize = 0;

const HEADER: usize = 16;
const UNIT: usize = 16;

// Block header: total block length in bytes (multiple of UNIT, header
// included), then a free flag. A block's payload starts HEADER bytes after the
// header, 16-byte aligned because every block length is a multiple of UNIT and
// the region base is 16-aligned.
unsafe fn block_len(block: *mut u8) -> usize {
    unsafe { ptr::read(block.cast::<u32>()) as usize }
}
unsafe fn block_free(block: *mut u8) -> bool {
    unsafe { ptr::read(block.add(4).cast::<u32>()) != 0 }
}
unsafe fn set_block(block: *mut u8, len: usize, free: bool) {
    unsafe {
        ptr::write(block.cast::<u32>(), len as u32);
        ptr::write(block.add(4).cast::<u32>(), free as u32);
    }
}

/// Installs the region every later allocation is carved from. A different
/// region is formatted as one free block; installing the same region again
/// keeps every earlier allocation alive. The caller keeps the region alive and
/// unmoved until every block carved from it has been freed.
pub fn install_region(region: &mut [u8]) {
    // SAFETY: single-threaded firmware; the caller keeps the region alive and
    // unmoved.
    unsafe {
        let base = region.as_mut_ptr();
        let misalign = base.addr() % UNIT;
        let base = base.add((UNIT - misalign) % UNIT);
        let len = (region.len() - (UNIT - misalign) % UNIT) / UNIT * UNIT;
        if REGION_BASE != base || REGION_LEN != len {
            REGION_BASE = base;
            REGION_LEN = len;
            REGION_PEAK = 0;
            set_block(base, len, true);
        }
    }
}

/// Highest payload end handed out since the region was installed, in bytes from
/// the region base. Reported to the handler so region sizing rests on
/// measurement.
pub fn region_high_water() -> usize {
    // SAFETY: single-threaded read of the bookkeeping.
    unsafe { REGION_PEAK }
}

struct FreeListAllocator;

// SAFETY: the firmware is single-threaded; every block is carved from the
// installed region, payloads are UNIT-aligned, and only blocks handed out by
// `alloc` are freed.
unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if layout.align() > UNIT {
            return ptr::null_mut();
        }
        let need = (HEADER + layout.size().max(1) + UNIT - 1) / UNIT * UNIT;
        // SAFETY: single-threaded walk over the block chain inside the region.
        unsafe {
            let (base, len) = (REGION_BASE, REGION_LEN);
            if base.is_null() {
                return ptr::null_mut();
            }
            let end = base.add(len);
            let mut block = base;
            while block < end {
                let blen = block_len(block);
                if block_free(block) {
                    // Coalesce with following free blocks.
                    let mut total = blen;
                    let mut next = block.add(total);
                    while next < end && block_free(next) {
                        total += block_len(next);
                        next = block.add(total);
                    }
                    if total != blen {
                        set_block(block, total, true);
                    }
                    if total >= need {
                        if total - need >= UNIT + HEADER {
                            set_block(block.add(need), total - need, true);
                            set_block(block, need, false);
                        } else {
                            set_block(block, total, false);
                        }
                        let payload_end = block.addr() + block_len(block) - base.addr();
                        if payload_end > REGION_PEAK {
                            REGION_PEAK = payload_end;
                        }
                        return block.add(HEADER);
                    }
                    block = block.add(total);
                } else {
                    block = block.add(blen);
                }
            }
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        // SAFETY: `pointer` came from `alloc`, so its header sits HEADER bytes before
        // it.
        unsafe {
            let block = pointer.sub(HEADER);
            set_block(block, block_len(block), true);
        }
    }
}

#[global_allocator]
static IRONWOOD_REGION_ALLOCATOR: FreeListAllocator = FreeListAllocator;

#[alloc_error_handler]
fn allocation_error(_layout: Layout) -> ! {
    rtl::system_exit_fatal("Ironwood allocation failed", file!(), line!())
}
