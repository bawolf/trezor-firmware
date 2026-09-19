//! Global allocator for the Ironwood signing core on the device.
//!
//! The streaming core allocates a little (Sinsemilla pads into a `Vec<bool>`,
//! `hash_to_curve` boxes a closure, Pasta builds its square-root table on
//! first use; design §7). Those blocks must live in memory the MicroPython
//! collector never touches: a GC-backed allocator freed Pasta's table, which
//! lives behind a Rust static the collector does not scan, and Pasta trapped.
//!
//! So the allocator is a first-fit free list inside a fixed region reserved for
//! the whole boot: a `.buf`-section static (`REGION`, in AUX2 RAM alongside the
//! other persistent display/wire buffers, NOT in the MicroPython GC heap). It is
//! formatted once, on the first `install_region`, and never freed. Blocks are
//! split on allocation; on free the whole region is swept once (O(n)) to merge every
//! run of adjacent free blocks, so freed per-action scratch is reclaimed
//! regardless of the order frees arrive in (temporary vectors — Pasta grows
//! four 256-element vectors while building the table — do not strand holes).
//!
//! Coalescing correctness: `dealloc` marks the block free and then runs
//! `coalesce_all`, which merges any maximal run of adjacent free blocks into
//! one. This gives both forward AND backward coalescing (the sweep starts at
//! the region base, so a freed block is always merged with a free predecessor
//! as well as a free successor) without a per-block footer, so the layout
//! stays 16 bytes of header only and space efficiency is unchanged. The design
//! is exercised on the host by a stress + 200k-op fuzz harness (see
//! .context/fork-migration/region-allocator-fix-20260918/): zero overlap,
//! undersize, misalignment, or out-of-region writes across the interleaved
//! alloc/free pattern, and a clean null on genuine OOM.
//!
//! Note on the 8-action fault (region-allocator-fix-20260918 report): the
//! free-list arithmetic is memory-safe; the observed Pasta fault is a capacity
//! problem (the 8-action working set can exceed the 96 KB region), which now
//! fails closed through the null path below rather than corrupting a live
//! block. On a malformed chain (a wild write from another subsystem) the walks
//! bail to null instead of spinning, so OOM/corruption both reach the clean
//! `alloc_error_handler` rather than hanging.
//!
//! Cross-session lifetime (2026-09-18, docs/decisions/2026-09-18-cross-session-
//! region-lifetime.md): Pasta's square-root table AND orchard's two
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
//! (`REGION_ROOTED`) and is a no-op thereafter, keeping every earlier allocation.

use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;
use core::ptr;

/// Bytes of the process-lifetime signing region. Sized to the measured
/// single-session working set (~44 KB high-water incl. the ~29.8 KB Pasta table)
/// plus headroom. Reserved from AUX2 RAM, so it shrinks the MicroPython GC heap
/// by this amount for the whole boot (see the decision note's heap analysis).
const REGION_BYTES: usize = 96 * 1024;

/// 16-byte-aligned backing store so the free-list base is `UNIT`-aligned.
#[repr(align(16))]
struct Region(MaybeUninit<[u8; REGION_BYTES]>);

/// The one signing region, reserved for the whole boot in the `.buf` section
/// (AUX2 RAM, outside the GC heap, alongside the display/wire buffers). Never
/// freed, never moved: this is what keeps Pasta's sqrt table and orchard's
/// `OnceBox` caches valid across sessions. Contents are formatted by
/// `install_region` before any use, so leaving it uninitialised (`.buf` is
/// NOLOAD) is fine. The `link_section` is skipped on host/emulator builds, which
/// never compile this module anyway (it is `target_arch = "arm"` only).
#[cfg_attr(not(target_os = "macos"), link_section = ".buf")]
static mut REGION: Region = Region(MaybeUninit::uninit());

static mut REGION_BASE: *mut u8 = ptr::null_mut();
static mut REGION_LEN: usize = 0;
// Boot-monotone maximum payload end ever handed out (never reset after the
// one-time format). Cheap to keep; reported alongside the per-session figure.
static mut REGION_PEAK: usize = 0;
// Peak payload end handed out during the CURRENT session, reset at each
// `mark_session_begin`. This is the figure the measurement sweep reads: on an
// ascending-N single-boot sweep it should stay ~flat because each session
// reuses the same rooted table + caches, which the boot-monotone `REGION_PEAK`
// (can only grow) cannot show.
static mut REGION_SESSION_PEAK: usize = 0;
// Bytes already in use in the region when the current session began (sampled by
// `mark_session_begin` before `begin` allocates). After session 1 this is the
// persistent set (Pasta table + the two orchard OnceBox caches) and must stay
// constant across a sweep; any upward drift is a cross-session leak.
static mut REGION_IN_USE_AT_BEGIN: usize = 0;
// Set once the region is formatted this boot; `install_region` is then a no-op
// so the Pasta table + orchard OnceBox caches carved into it survive every later
// session (the whole point of the process-lifetime region).
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

/// True if a header describes a well-formed block that stays inside `[base, end)`.
/// A malformed header (zero length, off the UNIT grid, or running past the region
/// end) can only arise from a wild write by another subsystem; the walks below stop
/// on it rather than looping forever or stepping outside the region.
unsafe fn block_ok(block: *mut u8, blen: usize, end: *mut u8) -> bool {
    blen != 0 && blen % UNIT == 0 && unsafe { block.add(blen) } <= end
}

/// Sweep the whole region once, merging every maximal run of adjacent free blocks
/// into a single free block. O(n) in the number of blocks. Called on every free so
/// the free list is always fully coalesced (no two adjacent free blocks), which makes
/// backward coalescing fall out of the forward sweep and keeps first-fit allocation
/// order-independent.
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
/// and it is never freed: the first call formats it as one free block, and every
/// later `session_begin` reuses it with the Pasta table + orchard `OnceBox`
/// caches (and the coalesced free list) still intact. That is what lets a second
/// sign in the same boot succeed where the old per-session region left those
/// statics dangling.
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
        REGION_PEAK = 0;
        REGION_SESSION_PEAK = 0;
        REGION_IN_USE_AT_BEGIN = 0;
        REGION_ROOTED = true;
        set_block(raw, REGION_BYTES, true);
    }
}

/// Highest payload end handed out since the region was installed (boot-monotone,
/// never reset), in bytes from the region base.
pub fn region_high_water() -> usize {
    // SAFETY: single-threaded read of the bookkeeping.
    unsafe { REGION_PEAK }
}

/// Highest payload end handed out during the CURRENT session (reset at each
/// `mark_session_begin`), in bytes from the region base. This is the per-session
/// figure the measurement sweep reads.
pub fn region_session_high_water() -> usize {
    // SAFETY: single-threaded read of the bookkeeping.
    unsafe { REGION_SESSION_PEAK }
}

/// Bytes already allocated in the region when the current session began, sampled
/// by `mark_session_begin` before `begin` allocates. Constant across a sweep in
/// steady state (the persistent Pasta table + orchard OnceBox caches); any
/// upward drift is a cross-session leak.
pub fn region_in_use_at_begin() -> usize {
    // SAFETY: single-threaded read of the bookkeeping.
    unsafe { REGION_IN_USE_AT_BEGIN }
}

/// Sum of every in-use block's total length (header included) by one O(n) walk.
pub fn region_in_use_bytes() -> usize {
    // SAFETY: single-threaded walk over the block chain inside the region.
    unsafe {
        let (base, len) = (REGION_BASE, REGION_LEN);
        if base.is_null() {
            return 0;
        }
        let end = base.add(len);
        let mut block = base;
        let mut total = 0;
        while block < end {
            let blen = block_len(block);
            if !block_ok(block, blen, end) {
                break;
            }
            if !block_free(block) {
                total += blen;
            }
            block = block.add(blen);
        }
        total
    }
}

/// Marks the start of a signing session for measurement: resets the per-session
/// peak and samples the in-use bytes BEFORE the session allocates, so a
/// cross-session leak is detectable. Call after `install_region`, before `begin`.
pub fn mark_session_begin() {
    // SAFETY: single-threaded write of the bookkeeping.
    unsafe {
        REGION_SESSION_PEAK = 0;
        REGION_IN_USE_AT_BEGIN = region_in_use_bytes();
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
                    let payload_end = block.addr() + block_len(block) - base.addr();
                    if payload_end > REGION_PEAK {
                        REGION_PEAK = payload_end;
                    }
                    if payload_end > REGION_SESSION_PEAK {
                        REGION_SESSION_PEAK = payload_end;
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
            // MUST-FIX #1 (Fable review, 2026-09-19): the region now lives for the
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
