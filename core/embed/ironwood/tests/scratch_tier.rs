#![cfg(feature = "test")]
//! The device allocator's own arena code, run as this process's allocator
//! under a whole `ZcashSignPczt` as the firmware makes it: `session_begin`,
//! every chunk through `session_feed`, `session_approve`, `session_sign` and
//! `session_cancel`, over a `SCRATCH_BYTES` scratch tier.
//!
//! `region_budget.rs` sums block sizes and so cannot see fragmentation, and
//! it traced a bare `Session` rather than what `signing.rs` builds around one.
//! It priced a 2-action session at 24,368 B of a 24,576 B tier; on the Safe 5
//! the same session stopped on "Ironwood allocation failed". This test runs the
//! real first-fit free list instead (`arena.rs`, included by path) and reports
//! the furthest byte any session reached: first-fit makes the same choices in
//! any tier at least that long, so that figure is the smallest tier the session
//! runs in, fragmentation included.
//!
//! Host sizes are the upper bound of the device's: a `Layout` here has 8-byte
//! pointers and `usize`s where the device has 4, and every other field is the
//! same size. Build with `computed-generators`, as the firmware does, to trace
//! the device's Sinsemilla.
//!
//! One test in its own binary, for the reason `rooted_tier.rs` gives: the
//! persistent set is process-global, as it is boot-global on the device.

mod common;

#[allow(dead_code)]
#[path = "../../rust/src/ironwood/arena.rs"]
mod arena;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::{Cell, UnsafeCell};
use std::fmt::Write;
use std::ptr;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use arena::{Arenas, HEADER};
use common::*;
use ironwood::{Event, Hedged, MAX_ACTIONS, MAX_TRANSPARENT_OUTPUTS, Network, Review, Session};
use orchard::keys::FullViewingKey;
use rand_core::OsRng;

/// `allocator::REGION_BYTES`.
const ROOTED_BYTES: usize = 40 * 1024;
/// `allocator::SCRATCH_BYTES` and `sign_pczt.SCRATCH_BYTES`.
const SCRATCH_BYTES: usize = 48 * 1024;
/// The furthest a session may reach into the scratch tier: 75 %, so a quarter
/// of the tier is margin for what the host trace does not see.
const SCRATCH_CEILING: usize = SCRATCH_BYTES / 4 * 3;
/// `sign_pczt.CHUNK_BYTES`: what one `ZcashPcztAck` carries.
const CHUNK_BYTES: usize = 1024;

#[repr(align(16))]
struct Tier<const N: usize>(UnsafeCell<[u8; N]>);
// SAFETY: only the traced thread touches the tiers.
unsafe impl<const N: usize> Sync for Tier<N> {}

impl<const N: usize> Tier<N> {
    fn base(&self) -> *mut u8 {
        self.0.get().cast()
    }
    fn holds(&self, pointer: *mut u8) -> bool {
        pointer >= self.base() && pointer < self.base().wrapping_add(N)
    }
}

static ROOTED: Tier<ROOTED_BYTES> = Tier(UnsafeCell::new([0; ROOTED_BYTES]));
static SCRATCH: Tier<SCRATCH_BYTES> = Tier(UnsafeCell::new([0; SCRATCH_BYTES]));

struct Shared(UnsafeCell<Arenas>);
// SAFETY: only the traced thread allocates from the arenas.
unsafe impl Sync for Shared {}
static ARENAS: Shared = Shared(UnsafeCell::new(Arenas::new()));

/// Per session: the furthest scratch byte handed out, the most scratch bytes
/// in use at once, and the refusals, each a fatal exit on the device.
static REACH: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static REFUSED: AtomicUsize = AtomicUsize::new(0);
/// The first refused request and the scratch tier's blocks at that moment.
static FIRST_REFUSAL: Mutex<Option<String>> = Mutex::new(None);

thread_local! {
    static TRACING: Cell<bool> = const { Cell::new(false) };
}

fn tracing() -> bool {
    TRACING.try_with(Cell::get).unwrap_or(false)
}

fn set_tracing(on: bool) {
    TRACING.with(|flag| flag.set(on));
}

fn arenas() -> &'static mut Arenas {
    // SAFETY: only the traced thread reaches the arenas, one call at a time.
    unsafe { &mut *ARENAS.0.get() }
}

/// Length of the block whose header is at `block`.
fn block_len(block: *mut u8) -> usize {
    // SAFETY: `block` is a header inside a tier this test formatted.
    unsafe { ptr::read(block.cast::<u32>()) as usize }
}

/// The scratch tier block by block, as `offset+len` with `*` for free.
fn scratch_blocks() -> String {
    let mut blocks = String::new();
    let mut offset = 0;
    while offset < SCRATCH_BYTES {
        // SAFETY: the walk follows headers `arena.rs` wrote, inside the tier.
        let block = unsafe { SCRATCH.base().add(offset) };
        let len = block_len(block);
        if len == 0 {
            break;
        }
        // SAFETY: as above; the free flag follows the length.
        let free = unsafe { ptr::read(block.add(4).cast::<u32>()) } != 0;
        let _ = write!(blocks, " {offset}+{len}{}", if free { "*" } else { "" });
        offset += len;
    }
    blocks
}

/// The frames of the current backtrace outside `std` and this harness.
fn caller() -> String {
    std::backtrace::Backtrace::force_capture()
        .to_string()
        .lines()
        .filter(|line| line.trim_start().starts_with(|c: char| c.is_ascii_digit()))
        .filter(|line| {
            !line.contains(" std::")
                && !line.contains(" core::")
                && !line.contains("scratch_tier::")
        })
        .take(12)
        .collect::<Vec<_>>()
        .join("\n")
}

/// Records what the arena did with `request`. A refusal is counted, and the
/// first one described, then served from `System` so the trace runs on.
fn served(pointer: *mut u8, request: usize) -> Option<*mut u8> {
    if pointer.is_null() {
        if REFUSED.fetch_add(1, Ordering::Relaxed) == 0 {
            set_tracing(false);
            let (_, in_use) = arenas().in_use();
            *FIRST_REFUSAL.lock().unwrap() = Some(format!(
                "refused {request} B (block {}) with {in_use} B in use; blocks:{}\n{}",
                arena::block_size(request),
                scratch_blocks(),
                caller()
            ));
            set_tracing(true);
        }
        return None;
    }
    if SCRATCH.holds(pointer) {
        // SAFETY: the header sits HEADER bytes before the payload.
        let block = unsafe { pointer.sub(HEADER) };
        let end = block as usize - SCRATCH.base() as usize + block_len(block);
        REACH.fetch_max(end, Ordering::Relaxed);
        PEAK.fetch_max(arenas().in_use().1, Ordering::Relaxed);
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
            if let Some(pointer) = served(pointer, layout.size()) {
                return pointer;
            }
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        if tracing() && (ROOTED.holds(pointer) || SCRATCH.holds(pointer)) {
            let resized =
                unsafe { arenas().realloc(pointer, layout.size(), new_size, layout.align()) };
            if let Some(resized) = served(resized, new_size) {
                return resized;
            }
        }
        // Refused, or never the arena's: move it into `System`.
        unsafe {
            let new_layout = Layout::from_size_align_unchecked(new_size, layout.align());
            let moved = System.alloc(new_layout);
            if !moved.is_null() {
                ptr::copy_nonoverlapping(pointer, moved, layout.size().min(new_size));
                self.dealloc(pointer, layout);
            }
            moved
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if ROOTED.holds(pointer) || SCRATCH.holds(pointer) {
            unsafe { arenas().dealloc(pointer) }
        } else {
            unsafe { System.dealloc(pointer, layout) }
        }
    }
}

#[global_allocator]
static ALLOCATOR: DeviceArena = DeviceArena;

fn traced<T>(body: impl FnOnce() -> T) -> T {
    set_tracing(true);
    let value = body();
    set_tracing(false);
    value
}

/// `signing::Signing`, which `signing::begin` boxes into the scratch tier.
#[allow(dead_code)]
struct Signing {
    handle: u32,
    session: Session<Hedged<OsRng>>,
    fvk: FullViewingKey,
    coin_type: u32,
    account: u32,
    review: Option<Review>,
}

/// Everything the firmware allocates between `install_scratch` and
/// `release_scratch` for one PCZT, in its order. The corpus keys stand in for
/// the seed-derived ones; `keys()` does the same key expansion
/// `AccountKeys::derive` ends in.
fn device_sign(pczt: &[u8]) -> usize {
    let seed = [7u8; 64];
    // `signing::begin`.
    let (fvk, _) = keys();
    let _ = ironwood::seed_fingerprint(&seed);
    let mut session = Session::with_rng(
        policy(Network::Testnet, 1_000_000),
        Hedged::from_seed(OsRng, &seed),
    )
    .unwrap();
    session.begin(pczt.len(), &fvk, &SEED_FINGERPRINT).unwrap();
    let mut signing = Box::new(Signing {
        handle: 1,
        session,
        fvk,
        coin_type: 1,
        account: ACCOUNT,
        review: None,
    });
    // `session_feed`, one `ZcashPcztAck` at a time, each fed until consumed.
    for chunk in pczt.chunks(CHUNK_BYTES) {
        let mut fed = 0;
        while fed < chunk.len() {
            let (consumed, event) = signing.session.feed(&chunk[fed..], &signing.fvk).unwrap();
            fed += consumed;
            if let Event::Review(review) = event {
                signing.review = Some(review);
            }
        }
    }
    // `session_approve`, then `session_sign`, which ends the request.
    let review = signing.review.as_ref().expect("the PCZT reaches review");
    signing.session.approve(review.token()).unwrap();
    let (_, ask) = keys();
    let signatures = signing.session.sign(review.token(), &ask).unwrap();
    signatures.records().len()
}

/// One session in a fresh scratch tier: `(reach, peak, refusals)`.
fn session(pczt: &[u8]) -> (usize, usize, usize) {
    REACH.store(0, Ordering::Relaxed);
    PEAK.store(0, Ordering::Relaxed);
    REFUSED.store(0, Ordering::Relaxed);
    // SAFETY: the tier is a 16-byte aligned static, lent for this session only.
    assert!(unsafe { arenas().install_scratch(SCRATCH.base(), SCRATCH_BYTES, SCRATCH_BYTES) });
    let signed = traced(|| device_sign(pczt));
    assert!(signed > 0, "nothing was signed");
    assert_eq!(
        arenas().release_scratch(),
        Ok(()),
        "a block outlived the session"
    );
    (
        REACH.load(Ordering::Relaxed),
        PEAK.load(Ordering::Relaxed),
        REFUSED.load(Ordering::Relaxed),
    )
}

#[test]
fn every_admitted_shape_signs_in_three_quarters_of_the_scratch_tier() {
    // Built before anything is traced: the host builder is not device code.
    let cases = [
        ("2 actions", build_actions(2)),
        ("8 actions", build_actions(8)),
        ("16 actions", build_actions(16)),
        ("32 actions", build_actions(MAX_ACTIONS)),
        ("1 action + 31 transparent", build_wide_deshield()),
    ];

    // `session_begin` before the scratch exists: the persistent set is rooted.
    // SAFETY: the tier is a 16-byte aligned static, used only through here.
    unsafe { arenas().root(ROOTED.base(), ROOTED_BYTES) };
    traced(ironwood::prewarm);
    let persistent = arenas().in_use().0;

    let mut failures = Vec::new();
    println!("scratch tier {SCRATCH_BYTES} B, ceiling {SCRATCH_CEILING} B");
    for (label, pczt) in &cases {
        let (reach, peak, refused) = session(pczt);
        let margin = 100.0 * (SCRATCH_BYTES - reach) as f64 / SCRATCH_BYTES as f64;
        println!(
            "  {label:26} {:5} B PCZT: reach {reach:5} B, peak {peak:5} B in use, margin {margin:.1} %, {refused} refused",
            pczt.len()
        );
        if let Some(first) = FIRST_REFUSAL.lock().unwrap().take() {
            println!("    first refusal: {first}");
        }
        // The rooted tier is the same after every session.
        assert_eq!(
            arenas().in_use().0,
            persistent,
            "{label} moved the persistent set"
        );
        if refused != 0 || reach > SCRATCH_CEILING {
            failures.push(format!("{label}: reach {reach} B, {refused} refused"));
        }
    }
    assert!(
        failures.is_empty(),
        "over the {SCRATCH_CEILING} B ceiling of the {SCRATCH_BYTES} B scratch tier: {failures:?}"
    );
}
