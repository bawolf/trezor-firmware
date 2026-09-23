#![cfg(feature = "test")]
//! The device allocator's own arena code, run as this process's allocator over
//! a `REGION_BYTES` rooted tier, under everything that fills the tier cold.
//!
//! `region_budget.rs` prices allocations with the arena's block arithmetic but
//! ignores fragmentation. This test runs the real first-fit free list
//! (`arena.rs`, included by path) instead. On the device a refused request is
//! a fatal exit: before `realloc` resized in place, Pasta's square-root table
//! build fragmented the 40 KB tier until an 8 KB growth was refused, and the
//! first `ZcashGetAddress` after boot stopped on a UsageFault.
//!
//! One test in its own binary: Pasta's square-root table and orchard's
//! `OnceBox` caches are process-global and build once, as they build once per
//! boot, so the cold path is only observable the first time. Build with
//! `computed-generators`, as the firmware does, to trace the device's
//! Sinsemilla.

#[allow(dead_code)]
#[path = "../../rust/src/ironwood/arena.rs"]
mod arena;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use arena::Arenas;
use ironwood::receive::{Network, derive_external_receiver, derive_full_viewing_key};

/// `allocator::REGION_BYTES`.
const REGION_BYTES: usize = 40 * 1024;

#[repr(align(16))]
struct Region(UnsafeCell<[u8; REGION_BYTES]>);
// SAFETY: only the traced thread touches the region.
unsafe impl Sync for Region {}
static REGION: Region = Region(UnsafeCell::new([0; REGION_BYTES]));

struct Shared(UnsafeCell<Arenas>);
// SAFETY: only the traced thread allocates from the arenas.
unsafe impl Sync for Shared {}
static ARENAS: Shared = Shared(UnsafeCell::new(Arenas::new()));

/// Requests the arena refused, and the size of the first one.
static REFUSED: AtomicUsize = AtomicUsize::new(0);
static FIRST_REFUSED: AtomicUsize = AtomicUsize::new(0);
static MISALIGNED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static TRACING: Cell<bool> = const { Cell::new(false) };
}

fn tracing() -> bool {
    TRACING.try_with(Cell::get).unwrap_or(false)
}

fn arenas() -> &'static mut Arenas {
    // SAFETY: only the traced thread reaches the arenas, one call at a time.
    unsafe { &mut *ARENAS.0.get() }
}

fn in_region(pointer: *mut u8) -> bool {
    let base = REGION.0.get().cast::<u8>();
    pointer >= base && pointer < base.wrapping_add(REGION_BYTES)
}

/// Checks what the arena returned. A refusal is counted and served from
/// `System` instead, so the trace runs to the end and reports every one.
fn served(pointer: *mut u8, layout: Layout) -> Option<*mut u8> {
    if pointer.is_null() {
        if REFUSED.fetch_add(1, Ordering::Relaxed) == 0 {
            FIRST_REFUSED.store(layout.size(), Ordering::Relaxed);
        }
        return None;
    }
    if pointer as usize % layout.align() != 0 {
        MISALIGNED.fetch_add(1, Ordering::Relaxed);
    }
    Some(pointer)
}

struct DeviceArena;

// SAFETY: the traced thread is served by the arena exactly as the device is;
// every other allocation, and a refused one, goes to `System`, and pointers
// are routed by address so neither allocator frees or resizes the other's.
unsafe impl GlobalAlloc for DeviceArena {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if tracing() {
            let pointer = unsafe { arenas().alloc(layout.size(), layout.align()) };
            if let Some(pointer) = served(pointer, layout) {
                return pointer;
            }
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_layout = unsafe { Layout::from_size_align_unchecked(new_size, layout.align()) };
        if tracing() && in_region(pointer) {
            let resized =
                unsafe { arenas().realloc(pointer, layout.size(), new_size, layout.align()) };
            if let Some(resized) = served(resized, new_layout) {
                return resized;
            }
        }
        // Everything else moves, as `GlobalAlloc`'s default does.
        unsafe {
            let moved = self.alloc(new_layout);
            if !moved.is_null() {
                ptr::copy_nonoverlapping(pointer, moved, layout.size().min(new_size));
                self.dealloc(pointer, layout);
            }
            moved
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if in_region(pointer) {
            unsafe { arenas().dealloc(pointer) }
        } else {
            unsafe { System.dealloc(pointer, layout) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: DeviceArena = DeviceArena;

fn traced<T>(body: impl FnOnce() -> T) -> T {
    TRACING.with(|flag| flag.set(true));
    let value = body();
    TRACING.with(|flag| flag.set(false));
    value
}

#[test]
fn the_rooted_tier_holds_a_cold_receive_and_everything_after_it() {
    // SAFETY: the region is a 16-byte aligned static, used only through here.
    unsafe { arenas().root(REGION.0.get().cast(), REGION_BYTES) };

    let seed = [7u8; 64];
    let mut index = [0u8; 11];
    index[0] = 3;
    let mut key = [0u8; 96];

    // `ZcashGetAddress` as the first Zcash call after boot, which is what
    // faulted: the receiver derivation builds Pasta's table cold. Then the
    // viewing key, a signing session's `prewarm` (orchard's `OnceBox`
    // caches), and a warm receive, all in the same tier.
    traced(|| derive_external_receiver(&seed, Network::Testnet, 0, index)).unwrap();
    traced(|| derive_full_viewing_key(&seed, Network::Testnet, 0, &mut key)).unwrap();
    traced(ironwood::prewarm);
    traced(|| derive_external_receiver(&seed, Network::Testnet, 0, index)).unwrap();

    let (persistent, _) = arenas().audit();
    println!(
        "rooted tier {REGION_BYTES} B: {persistent} B persistent, {} requests refused",
        REFUSED.load(Ordering::Relaxed)
    );
    assert_eq!(
        MISALIGNED.load(Ordering::Relaxed),
        0,
        "a payload missed its alignment"
    );
    assert_eq!(
        REFUSED.load(Ordering::Relaxed),
        0,
        "the rooted tier refused {} requests, the first for {} B; each is a fatal exit on the device",
        REFUSED.load(Ordering::Relaxed),
        FIRST_REFUSED.load(Ordering::Relaxed)
    );
}
