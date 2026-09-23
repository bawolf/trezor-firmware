//! The device allocator's own arena code, run as this process's allocator over
//! a `REGION_BYTES` rooted tier and a `SCRATCH_BYTES` scratch tier, through a
//! boot: one cold order of the calls that fill the rooted tier, then signing
//! sessions. Each `rooted_tier*.rs` binary is one cold order.
//!
//! `region_budget.rs` prices allocations with the arena's block arithmetic but
//! ignores fragmentation. This runs the real first-fit free list (`arena.rs`,
//! included by path) instead. On the device a refused request is a fatal exit:
//! before `realloc` resized in place, Pasta's square-root table build
//! fragmented the 40 KB tier until an 8 KB growth was refused, and the first
//! `ZcashGetAddress` after boot stopped on a UsageFault. Every cold order
//! failed that way, and the table is built once per process as it is once per
//! boot, so each order needs a binary of its own.
//!
//! The sessions are the device's two-tier invariants with fragmentation in
//! them: two signs in one boot, then the two widest shapes the wire admits (32
//! shielded actions; one action and 31 transparent outputs), each in a freshly
//! installed scratch that must refuse nothing, must be empty when it is
//! released -- the device's fatal tripwire -- and must leave the rooted tier
//! exactly as it found it.
//!
//! The binaries require `computed-generators` (`Cargo.toml`): the device links
//! the computed Sinsemilla, whose domain warm allocates differently from the
//! precomputed table's, and fragmentation depends on the exact sequence.

#[allow(dead_code)]
#[path = "../../../rust/src/ironwood/arena.rs"]
mod arena;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::ptr;
use std::sync::atomic::{AtomicUsize, Ordering};

use arena::Arenas;
use ironwood::MAX_ACTIONS;
use ironwood::receive::{Network, derive_external_receiver, derive_full_viewing_key};

use crate::common::*;

#[cfg(not(feature = "computed-generators"))]
compile_error!("the device arena tests trace the device's Sinsemilla: add `computed-generators`");

/// `allocator::REGION_BYTES`.
const REGION_BYTES: usize = 40 * 1024;
/// `allocator::SCRATCH_BYTES`, the `bytearray` `sign_pczt` lends each session.
const SCRATCH_BYTES: usize = 24 * 1024;

#[repr(align(16))]
struct Region(UnsafeCell<[u8; REGION_BYTES]>);
// SAFETY: only the traced thread touches the region.
unsafe impl Sync for Region {}
static REGION: Region = Region(UnsafeCell::new([0; REGION_BYTES]));

#[repr(align(16))]
struct Scratch(UnsafeCell<[u8; SCRATCH_BYTES]>);
// SAFETY: only the traced thread touches the scratch.
unsafe impl Sync for Scratch {}
static SCRATCH: Scratch = Scratch(UnsafeCell::new([0; SCRATCH_BYTES]));

struct Shared(UnsafeCell<Arenas>);
// SAFETY: only the traced thread allocates from the arenas.
unsafe impl Sync for Shared {}
static ARENAS: Shared = Shared(UnsafeCell::new(Arenas::new()));

/// Requests the arena refused, and the size of the first one.
static REFUSED: AtomicUsize = AtomicUsize::new(0);
static FIRST_REFUSED: AtomicUsize = AtomicUsize::new(0);
static MISALIGNED: AtomicUsize = AtomicUsize::new(0);
/// High-water mark of the installed scratch, reset at each install.
static SCRATCH_PEAK: AtomicUsize = AtomicUsize::new(0);

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

/// Whether `pointer` is arena memory -- either tier -- rather than `System`'s.
fn in_arena(pointer: *mut u8) -> bool {
    let within = |base: *mut u8, len: usize| pointer >= base && pointer < base.wrapping_add(len);
    within(REGION.0.get().cast(), REGION_BYTES) || within(SCRATCH.0.get().cast(), SCRATCH_BYTES)
}

/// Checks what the arena returned, counting each refusal once. A refused
/// request is then served from `System`, so the trace runs to the end and
/// reports every one.
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
    SCRATCH_PEAK.fetch_max(arenas().in_use().1, Ordering::Relaxed);
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
        let moved = if tracing() && in_arena(pointer) {
            // The arena's own `realloc` already tried in place and then a
            // fresh block. If it refused, `served` has counted that once, and
            // asking the arena again through `self.alloc` would count the same
            // request twice: go to `System` directly.
            let resized =
                unsafe { arenas().realloc(pointer, layout.size(), new_size, layout.align()) };
            if let Some(resized) = served(resized, new_layout) {
                return resized;
            }
            unsafe { System.alloc(new_layout) }
        } else {
            // A `System` block, or an untraced call: it moves, as
            // `GlobalAlloc`'s default does -- into the arena if tracing.
            unsafe { self.alloc(new_layout) }
        };
        if !moved.is_null() {
            // SAFETY: both blocks are live and distinct; the old one is freed
            // by whichever allocator it belongs to.
            unsafe {
                ptr::copy_nonoverlapping(pointer, moved, layout.size().min(new_size));
                self.dealloc(pointer, layout);
            }
        }
        moved
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if in_arena(pointer) {
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

/// A call that fills the rooted tier outside a session, as the handlers make
/// it: `ZcashGetAddress`, `ZcashGetViewingKey`, and the `prewarm` every
/// `session_begin` runs before it installs a scratch.
#[derive(Clone, Copy, Debug)]
pub enum Cold {
    Receive,
    ViewingKey,
    Prewarm,
}

/// Runs `order` against a freshly rooted tier, then every session shape, and
/// asserts nothing was refused, nothing misaligned, the scratch was empty at
/// every release and the rooted tier never moved.
pub fn boot(order: &[Cold]) {
    // SAFETY: the region is a 16-byte aligned static, used only through here.
    unsafe { arenas().root(REGION.0.get().cast(), REGION_BYTES) };

    let seed = [7u8; 64];
    let mut index = [0u8; 11];
    index[0] = 3;
    let mut key = [0u8; 96];
    for step in order {
        match step {
            Cold::Receive => {
                traced(|| derive_external_receiver(&seed, Network::Testnet, 0, index)).unwrap();
            }
            Cold::ViewingKey => {
                traced(|| derive_full_viewing_key(&seed, Network::Testnet, 0, &mut key)).unwrap();
            }
            Cold::Prewarm => traced(ironwood::prewarm),
        }
    }

    let (persistent, _) = arenas().audit();
    println!(
        "{order:?}: rooted tier {REGION_BYTES} B, {persistent} B persistent, {} requests refused",
        REFUSED.load(Ordering::Relaxed)
    );

    // The fixtures are built untraced, after the rooted tier is filled: the
    // host's own Orchard builder is not device code, and building a fixture
    // first would fill the statics above from `System` instead of the tier.
    let small = fixture();
    let wide_shielded = build_actions(MAX_ACTIONS);
    let wide_transparent = build_wide_deshield();

    // Sessions, each in a freshly installed scratch as `session_begin` does.
    // ZIP-317 charges `5_000 * actions`, so the widest shielded bundle needs a
    // `maximum_fee` that admits its 160_000.
    let scratch = SCRATCH.0.get().cast::<u8>();
    for (label, bytes, seed, maximum_fee) in [
        ("sign 1", &small, 1, 100_000),
        ("sign 2", &small, 2, 100_000),
        ("32 actions", &wide_shielded, 3, 5_000 * MAX_ACTIONS as u64),
        ("1 + 31 transparent", &wide_transparent, 4, 100_000),
    ] {
        // SAFETY: the scratch static outlives the session and is only
        // reached through the arena until `release_scratch`.
        assert!(unsafe { arenas().install_scratch(scratch, SCRATCH_BYTES, SCRATCH_BYTES) });
        SCRATCH_PEAK.store(0, Ordering::Relaxed);
        let refused = REFUSED.load(Ordering::Relaxed);
        traced(|| sign_streamed(bytes, seed, maximum_fee));
        println!(
            "scratch tier {SCRATCH_BYTES} B, {label:<18}: peak {} B, {} refused",
            SCRATCH_PEAK.load(Ordering::Relaxed),
            REFUSED.load(Ordering::Relaxed) - refused
        );
        // The device's tripwire: a block still in the scratch at release is a
        // static filled mid-session, and the device exits on it.
        assert_eq!(
            arenas().release_scratch(),
            Ok(()),
            "{label}: the scratch was not empty at release"
        );
        assert_eq!(
            arenas().audit().0,
            persistent,
            "{label}: the session moved the rooted tier"
        );
    }

    assert_eq!(
        MISALIGNED.load(Ordering::Relaxed),
        0,
        "a payload missed its alignment"
    );
    assert_eq!(
        REFUSED.load(Ordering::Relaxed),
        0,
        "the arenas refused {} requests, the first for {} B; each is a fatal exit on the device",
        REFUSED.load(Ordering::Relaxed),
        FIRST_REFUSED.load(Ordering::Relaxed)
    );
}
