//! Global allocator for the Ironwood signing core on the device.
//!
//! The streaming core allocates a little (Sinsemilla pads into a `Vec<bool>`,
//! `hash_to_curve` boxes a closure, Pasta builds its square-root table on
//! first use; docs/common/zcash-ironwood-signing.md §7). Those blocks must
//! live in memory the MicroPython
//! collector never touches: a GC-backed allocator freed Pasta's table, which
//! lives behind a Rust static the collector does not scan, and Pasta trapped.
//!
//! So the allocator is a first-fit free list inside a fixed region reserved for
//! the whole boot: a `.buf`-section static (`REGION`, placed wherever the
//! model's linker script puts `.buf` -- AUX1_RAM on T3T1 and T3W1, AUX2_RAM on
//! T3B1 -- alongside the other persistent display/wire buffers, and NOT in the
//! MicroPython GC heap). It is
//! formatted once, on the first `install_region`, and never freed. Blocks are
//! split on allocation; on free the whole region is swept once (O(n)) to merge
//! every run of adjacent free blocks, so freed per-action scratch is reclaimed
//! regardless of the order frees arrive in (temporary vectors — Pasta grows
//! four 256-element vectors while building the table — do not strand holes).
//!
//! Coalescing correctness: `dealloc` marks the block free and then runs
//! `coalesce_all`, which merges any maximal run of adjacent free blocks into
//! one. This gives both forward AND backward coalescing (the sweep starts at
//! the region base, so a freed block is always merged with a free predecessor
//! as well as a free successor) without a per-block footer, so the layout
//! stays 16 bytes of header only and space efficiency is unchanged. The design
//! is exercised on the host by a stress + 200k-op fuzz harness: zero overlap,
//! undersize, misalignment, or out-of-region writes across the interleaved
//! alloc/free pattern, and a clean null on genuine OOM.
//!
//! Residue after a fatal reset: `.buf` is NOLOAD, so nothing zeroes this
//! region at boot. The `dealloc` wipe below covers every orderly path -- each
//! freed block's payload is zeroed before it returns to the free list -- but a
//! fatal mid-session exit (`alloc_error_handler` -> `system_exit_fatal`, or
//! any other reset) runs no destructors, so whatever was live stays in RAM
//! until the same offsets are allocated again. It is the same exposure every
//! coin's stack residues have, and the same answer: what is left is
//! key-derived scratch, not the seed, which never enters this region.
//!
//! Note on the 8-action fault: the free-list arithmetic is memory-safe; the
//! observed Pasta fault is a capacity problem (the 8-action working set can
//! exceed the 96 KB region), which now fails closed through the null path below
//! rather than corrupting a live block. On a malformed chain (a wild write from
//! another subsystem) the walks bail to null instead of spinning, so
//! OOM/corruption both reach the clean `alloc_error_handler` rather than
//! hanging.
//!
//! Cross-session lifetime: Pasta's square-root table AND orchard's two
//! `OnceBox<CommitDomain>` caches are built once per boot behind Rust `static`s
//! the collector never scans, and they allocate through THIS region. They
//! therefore stay valid only while the region that first held them is alive. An
//! earlier design gave each session a fresh Python `bytearray` region freed at
//! session end, so those statics dangled and a second sign in the same boot
//! faulted (same base, re-zeroed content) or read freed memory (different base)
//! — the one remaining signing blocker. The fix makes the region a `.buf`
//! static reserved for the whole boot: its base never moves and it is never
//! freed, so the table and caches stay valid across every session by
//! construction, the pre-warm is a one-time boot cost, and the region no longer
//! competes with (or churns) the GC heap. `install_region` formats it once
//! (`REGION_ROOTED`) and is a no-op thereafter, keeping every earlier
//! allocation.

use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::ptr;

/// Bytes of the process-lifetime signing region. Sized to the measured
/// single-session working set (~44 KB high-water incl. the ~29.8 KB Pasta
/// table) plus headroom. Reserved from AUX2 RAM, so it shrinks the MicroPython
/// GC heap by this amount for the whole boot (see the decision note's heap
/// analysis).
const REGION_BYTES: usize = 96 * 1024;

/// 16-byte-aligned backing store so the free-list base is `UNIT`-aligned.
#[repr(align(16))]
struct Region(MaybeUninit<[u8; REGION_BYTES]>);

/// The one signing region, reserved for the whole boot in the `.buf` section
/// (AUX2 RAM, outside the GC heap, alongside the display/wire buffers). Never
/// freed, never moved: this is what keeps Pasta's sqrt table and orchard's
/// `OnceBox` caches valid across sessions. Contents are formatted by
/// `install_region` before any use, so leaving it uninitialised (`.buf` is
/// NOLOAD) is fine. The `link_section` is skipped on host/emulator builds,
/// which never compile this module anyway (it is `target_arch = "arm"` only).
#[cfg_attr(not(target_os = "macos"), link_section = ".buf")]
static mut REGION: Region = Region(MaybeUninit::uninit());

static mut REGION_BASE: *mut u8 = ptr::null_mut();
static mut REGION_LEN: usize = 0;
// Set once the region is formatted this boot; `install_region` is then a no-op
// so the Pasta table + orchard OnceBox caches carved into it survive every
// later session (the whole point of the process-lifetime region).
static mut REGION_ROOTED: bool = false;

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

/// True if a header describes a well-formed block that stays inside `[base,
/// end)`. A malformed header (zero length, off the UNIT grid, or running past
/// the region end) can only arise from a wild write by another subsystem; the
/// walks below stop on it rather than looping forever or stepping outside the
/// region.
unsafe fn block_ok(block: *mut u8, blen: usize, end: *mut u8) -> bool {
    blen != 0 && blen % UNIT == 0 && unsafe { block.add(blen) } <= end
}

/// Sweep the whole region once, merging every maximal run of adjacent free
/// blocks into a single free block. O(n) in the number of blocks. Called on
/// every free so the free list is always fully coalesced (no two adjacent free
/// blocks), which makes backward coalescing fall out of the forward sweep and
/// keeps first-fit allocation order-independent.
unsafe fn coalesce_all(base: *mut u8, len: usize) {
    unsafe {
        let end = base.add(len);
        let mut block = base;
        while block < end {
            let blen = block_len(block);
            if !block_ok(block, blen, end) {
                break;
            }
            if block_free(block) {
                let mut total = blen;
                let mut next = block.add(total);
                while next < end && block_free(next) {
                    let nlen = block_len(next);
                    if !block_ok(next, nlen, end) {
                        break;
                    }
                    total += nlen;
                    next = block.add(total);
                }
                if total != blen {
                    set_block(block, total, true);
                }
                block = block.add(total);
            } else {
                block = block.add(blen);
            }
        }
    }
}

/// Roots the process-lifetime region on the first call and is a no-op
/// thereafter. The region is the fixed `.buf` static, so its base never moves
/// and it is never freed: the first call formats it as one free block, and
/// every later `session_begin` reuses it with the Pasta table + orchard
/// `OnceBox` caches (and the coalesced free list) still intact. That is what
/// lets a second sign in the same boot succeed where the old per-session region
/// left those statics dangling.
pub fn install_region() {
    // SAFETY: single-threaded firmware; `REGION` is a boot-lifetime static.
    unsafe {
        if REGION_ROOTED {
            // Already formatted this boot. Do NOT re-format: the Pasta sqrt
            // table and orchard OnceBox caches live in here and are reached
            // through Rust statics the GC never scans.
            return;
        }
        let raw = ptr::addr_of_mut!(REGION).cast::<u8>();
        // `#[repr(align(16))]` guarantees 16-byte alignment; REGION_BYTES is a
        // multiple of UNIT, so no head/tail rounding is needed.
        REGION_BASE = raw;
        REGION_LEN = REGION_BYTES;
        REGION_ROOTED = true;
        set_block(raw, REGION_BYTES, true);
    }
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
            // The free list is kept fully coalesced by `dealloc`, so a single
            // first-fit pass finds the largest available hole for `need`.
            let mut block = base;
            while block < end {
                let blen = block_len(block);
                // Fail closed on a malformed chain rather than walking off the region:
                // return null so `alloc_error_handler` runs a clean fatal exit.
                if !block_ok(block, blen, end) {
                    return ptr::null_mut();
                }
                if block_free(block) && blen >= need {
                    if blen - need >= UNIT + HEADER {
                        set_block(block.add(need), blen - need, true);
                        set_block(block, need, false);
                    } else {
                        set_block(block, blen, false);
                    }
                    return block.add(HEADER);
                }
                block = block.add(blen);
            }
            ptr::null_mut()
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        // SAFETY: `pointer` came from `alloc`, so its header sits HEADER bytes before
        // it. Marking the block free and sweeping the region merges it with any free
        // neighbours (predecessor and successor), so no freed scratch is stranded.
        unsafe {
            let block = pointer.sub(HEADER);
            let blen = block_len(block);
            // The region now lives for the
            // whole boot, so freed secret-class scratch (e.g. the sinsemilla
            // `padded: Vec<bool>` holding ak||nk / nullifier-key bits) is no longer
            // recycled — and overwritten — by GC churn; it would linger in `.buf`
            // until the same offsets happen to be reused. Zero the payload span
            // before marking the block free. The compiler fence keeps this store
            // from being elided as dead ahead of the block returning to the free
            // list. Cost is a memset of the freed size, negligible next to
            // Sinsemilla. Header bookkeeping is untouched.
            let payload = pointer;
            let payload_len = blen - HEADER;
            ptr::write_bytes(payload, 0, payload_len);
            core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
            set_block(block, blen, true);
            let (base, len) = (REGION_BASE, REGION_LEN);
            if !base.is_null() {
                coalesce_all(base, len);
            }
        }
    }
}

#[global_allocator]
static IRONWOOD_REGION_ALLOCATOR: FreeListAllocator = FreeListAllocator;

#[alloc_error_handler]
fn allocation_error(_layout: Layout) -> ! {
    rtl::system_exit_fatal("Ironwood allocation failed", file!(), line!())
}
