//! Global allocator for the Ironwood signing core on the device.
//!
//! The streaming core allocates a little (Sinsemilla pads into a `Vec<bool>`,
//! `hash_to_curve` boxes a closure, Pasta builds its square-root table on
//! first use; docs/common/zcash-ironwood-signing.md §7). None of it may live in
//! memory the MicroPython collector can free: Pasta's table sits behind a Rust
//! static the collector does not scan, and a GC-backed allocator freed it
//! underneath Pasta.
//!
//! Those allocations divide in two, and so do the arenas (`arena.rs`, which
//! holds the free-list mechanics and the routing rule):
//!
//! - the **rooted tier**, `REGION`: a `REGION_BYTES` static in the
//!   `.zcash_region` section, which the model's linker script places in AUX1
//!   RAM (T3T1 and T3B1; T3W1 has one bank and puts everything there). It is
//!   outside the MicroPython GC heap and outside `.buf`, so the heap keeps its
//!   full stock size. Formatted once per boot, never freed, never moved: this
//!   is what holds the persistent set -- Pasta's `SqrtTables<Fp>` and orchard's
//!   two `OnceBox<CommitDomain>` caches -- valid across every session.
//! - the **scratch tier**: a MicroPython `bytearray` the Zcash workflow owns,
//!   installed for the length of one signing session and released in the
//!   workflow's `finally`. A session's working set is borrowed from the GC heap
//!   for exactly as long as it is needed, so every other coin's workflow runs
//!   at the full heap.
//!
//! Residue. `dealloc` zeroes each freed payload and `release_scratch` zeroes
//! the whole scratch before those bytes go back to the collector, which will
//! reuse them for arbitrary Python objects. `.zcash_region` is NOLOAD, so
//! nothing clears the rooted tier at boot; a fatal mid-session exit runs no
//! destructors and leaves whatever was live until the same offsets are
//! allocated again. That is the exposure every coin's stack residue has, and
//! the same answer: what stays is key-derived scratch, never the seed.

use core::alloc::{GlobalAlloc, Layout};
use core::mem::MaybeUninit;

use super::arena::{Arenas, UNIT};

/// Bytes of the boot-rooted tier. Sized to the `prewarm` transient peak, which
/// is what Pasta's square-root table costs while it is being built: the build
/// grows four 256-element `Vec<Fp>` and holds all four live before shrinking
/// the last, so it peaks well above the ~29.8 KB table that survives it. The
/// `ironwood/tests/rooted_tier*.rs` binaries run this arena over the real
/// trace, in every cold order, and assert it refuses nothing.
const REGION_BYTES: usize = 40 * 1024;

/// Smallest scratch tier a session may be given, and the size
/// `apps.zcash.sign_pczt` allocates. The two must move together.
///
/// Sized by `ironwood/tests/scratch_tier.rs`, which runs this arena under a
/// whole sign -- the boxed request included -- and requires every admitted
/// shape to stay within three quarters of the tier; that quarter is the
/// margin. At 24 KiB a 2-action sign was refused on the Safe 5: the host model
/// then in use left out the boxed request and the key derivation around it and
/// came up 432 B short. The tier was packed at the refusal, not fragmented --
/// in-place `realloc` is what keeps it that way. The tier is borrowed from the
/// GC heap only while
/// a Zcash session is live: on T3T1 that leaves 240,368 - 49,152 = 191,216 B of
/// heap for the Zcash workflow itself, and every other flow keeps all of it.
pub const SCRATCH_BYTES: usize = 48 * 1024;

/// 16-byte-aligned backing store so the free-list base is `UNIT`-aligned.
#[repr(align(16))]
struct Region(MaybeUninit<[u8; REGION_BYTES]>);

/// The rooted tier, reserved for the whole boot in `.zcash_region`. Never
/// freed, never moved: this is what keeps Pasta's sqrt table and orchard's
/// `OnceBox` caches valid across sessions. Contents are formatted by
/// `install_region` before any use, so leaving it uninitialised (the section is
/// NOLOAD) is fine. The `link_section` is skipped on host/emulator builds,
/// which never compile this module anyway (it is `target_arch = "arm"` only).
#[cfg_attr(not(target_os = "macos"), link_section = ".zcash_region")]
static mut REGION: Region = Region(MaybeUninit::uninit());

static mut ARENAS: Arenas = Arenas::new();

// Highest bytes each tier has ever held. Debuglink-only: the device gate reads
// them through `trezorironwood.debug_region_info()` and nothing in production
// does, so production carries neither the counters nor the comparison.
#[cfg(feature = "debuglink")]
static mut PERSIST_PEAK: usize = 0;
#[cfg(feature = "debuglink")]
static mut SCRATCH_PEAK: usize = 0;

fn arenas() -> &'static mut Arenas {
    // SAFETY: the firmware is single-threaded and every caller runs to
    // completion before another can touch the allocator.
    unsafe { &mut *core::ptr::addr_of_mut!(ARENAS) }
}

/// Records the tiers' high-water marks after an allocation changed them.
#[cfg(feature = "debuglink")]
fn mark_peaks() {
    let (persist, scratch) = arenas().in_use();
    // SAFETY: single-threaded update of the bookkeeping.
    unsafe {
        if persist > PERSIST_PEAK {
            PERSIST_PEAK = persist;
        }
        if scratch > SCRATCH_PEAK {
            SCRATCH_PEAK = scratch;
        }
    }
}

/// Roots the persistent tier on the first call and is a no-op thereafter. The
/// tier is the fixed `.zcash_region` static, so its base never moves and it is
/// never freed: the first call formats it as one free block, and every later
/// `session_begin` reuses it with the Pasta table, the orchard `OnceBox` caches
/// and the coalesced free list still intact.
pub fn install_region() {
    let base = core::ptr::addr_of_mut!(REGION).cast::<u8>();
    // SAFETY: `REGION` is a boot-lifetime static, `#[repr(align(16))]`
    // guarantees the alignment, and `REGION_BYTES` is a multiple of `UNIT`.
    unsafe { arenas().root(base, REGION_BYTES) };
}

/// Fatal if a scratch tier is still installed.
///
/// `session_begin` calls this before anything else. A live scratch at that
/// point means the workflow that installed it never reached the `finally`
/// that releases it -- and the only way that happens is a generator that was
/// dropped rather than closed, because the vendored MicroPython has no
/// generator finaliser (`vendor/micropython/py/objgenerator.c`), so a
/// collected generator never runs its `finally` at all. The span is then
/// backed by a `bytearray` nothing holds a reference to, which the collector
/// may already have handed to something else; releasing it would zero
/// `SCRATCH_BYTES` of live heap, and freeing through it would rewrite block
/// headers over whatever now lives there. There is no recovery from that, so
/// refuse to start instead. No reachable path in `apps.zcash.sign_pczt` gets
/// here -- this is the assertion that keeps it that way.
pub fn forbid_installed_scratch() {
    if arenas().scratch_installed() {
        rtl::system_exit_fatal("Ironwood scratch not released", file!(), line!());
    }
}

/// Lends `[base, base + len)` -- a MicroPython `bytearray` the caller keeps
/// referenced -- to the allocator as this session's scratch tier. False means
/// the buffer was refused (already one installed, or under `SCRATCH_BYTES`);
/// the caller turns that into a `ValueError` rather than signing out of the
/// wrong tier.
///
/// # Safety
///
/// `base` must stay allocated, unmoved and untouched by anything else until
/// `release_scratch`. MicroPython's collector does not move objects, so the
/// caller's own reference is enough to guarantee this.
pub unsafe fn install_scratch(base: *mut u8, len: usize) -> bool {
    // SAFETY: the caller owns the buffer for the whole session.
    let installed = unsafe { arenas().install_scratch(base, len, SCRATCH_BYTES) };
    #[cfg(feature = "debuglink")]
    if installed {
        // SAFETY: single-threaded reset of the per-session bookkeeping.
        unsafe {
            SCRATCH_PEAK = 0;
        }
    }
    installed
}

/// Ends the session's claim on the scratch tier, wipes it and routes allocation
/// back to the rooted tier. A block still in use is a Rust static that was
/// first filled during the session and would dangle the moment the `bytearray`
/// is collected -- the inverse of the cross-session bug the rooted tier fixes,
/// and fatal rather than undefined.
pub fn release_scratch() {
    if arenas().release_scratch().is_err() {
        rtl::system_exit_fatal("Ironwood scratch retained", file!(), line!());
    }
}

/// In-use and peak bytes of both tiers: `(persist_in_use, persist_peak,
/// scratch_in_use, scratch_peak)`. The device gate reads this between two signs
/// in one boot; `persist_in_use` must not drift and `scratch_in_use` must be
/// zero at every release. The in-use figures come from the block chains, so
/// they also cross-check the counters.
#[cfg(feature = "debuglink")]
pub fn region_info() -> (usize, usize, usize, usize) {
    let (persist, scratch) = arenas().audit();
    // SAFETY: single-threaded read of the bookkeeping.
    unsafe { (persist, PERSIST_PEAK, scratch, SCRATCH_PEAK) }
}

/// A tier that cannot serve a request is fatal, and says so here: the release
/// profile's `panic = "immediate-abort"` compiles `handle_alloc_error` to a
/// bare `udf`, so a null returned to `alloc` never reaches
/// `allocation_error` below and shows only as a UsageFault at a `RawVec` PC.
fn served(payload: *mut u8) -> *mut u8 {
    if payload.is_null() {
        rtl::system_exit_fatal("Ironwood allocation failed", file!(), line!());
    }
    #[cfg(feature = "debuglink")]
    mark_peaks();
    payload
}

struct FreeListAllocator;

// SAFETY: the firmware is single-threaded; every block is carved from one of
// the two installed tiers, payloads are UNIT-aligned, and only blocks handed
// out by `alloc` are resized or freed.
unsafe impl GlobalAlloc for FreeListAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the tiers installed above are valid for the call.
        served(unsafe { arenas().alloc(layout.size(), layout.align()) })
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `pointer` came from `alloc` with `layout` and is still live.
        served(unsafe { arenas().realloc(pointer, layout.size(), new_size, layout.align()) })
    }

    unsafe fn dealloc(&self, pointer: *mut u8, _layout: Layout) {
        // SAFETY: `pointer` came from `alloc` and has not been freed.
        unsafe { arenas().dealloc(pointer) };
    }
}

#[global_allocator]
static IRONWOOD_REGION_ALLOCATOR: FreeListAllocator = FreeListAllocator;

#[alloc_error_handler]
fn allocation_error(_layout: Layout) -> ! {
    rtl::system_exit_fatal("Ironwood allocation failed", file!(), line!())
}

const _: () = assert!(REGION_BYTES % UNIT == 0);
// The rooted tier's base is `&REGION`, and `Arenas::root` requires it on the
// arena's grid: every payload it hands out is `base + k * UNIT + HEADER`, so
// anything less than `UNIT` here makes every allocation unaligned by the same
// offset -- which on this target is a UsageFault the moment a `u64` or a
// `pallas::Base` limb lands on it, not a slow path. `#[repr(align(16))]` on
// `Region` is what provides it; this is the assertion that it still does.
const _: () = assert!(core::mem::align_of::<Region>() >= UNIT);
// The scratch is a MicroPython `bytearray` of exactly `SCRATCH_BYTES`, and
// `install_scratch` aligns its base up to `UNIT` before measuring it, so the
// tier is the whole buffer only while the two grids agree. The other half of
// that coupling -- that MicroPython really does place item data on a 16-byte
// boundary -- is `_Static_assert`ed against `MICROPY_BYTES_PER_GC_BLOCK` in
// `upymod/rustmods.c`, where the module is registered.
const _: () = assert!(SCRATCH_BYTES % UNIT == 0);
