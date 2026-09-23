#![cfg(feature = "test")]
//! Host model of the device signing arenas, run over the real allocation
//! traces of `prewarm` and of a streamed session.
//!
//! The device splits its allocations in two (`ironwood::allocator`): a
//! boot-rooted persistent arena in `.zcash_region`, and a per-session scratch
//! arena borrowed from the MicroPython heap. Both are first-fit free lists with
//! a 16-byte header on a 16-byte unit, so a request of `n` bytes costs
//! `ceil((16 + max(n, 1)) / 16) * 16`. The global allocator below applies that
//! arithmetic to every allocation the traced thread makes, which is what lets a
//! host test size arenas the emulator cannot: `allocator_unix.rs` is plain
//! `malloc`, so no emulator run prices either tier.
//!
//! Three budgets are asserted, in one test because the traces are
//! process-global (Pasta's square-root table and orchard's `OnceBox` caches
//! build once per process, exactly as they do once per boot):
//!
//! 1. the persistent arena must hold the whole `prewarm` transient peak,
//! 2. the persistent set must not grow across sessions,
//! 3. a session's transient demand must fit the scratch arena.
//!
//! The model counts the requested block size, not the (occasionally larger)
//! block first-fit hands out when the remainder is too small to split, and it
//! ignores fragmentation. Both make it a lower bound on arena occupancy, which
//! is why the asserted margins are reported rather than merely checked.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use common::*;
use ironwood::{Event, Network, Session};
use ironwood_pasta_curves::group::{Group, GroupEncoding};
use ironwood_pasta_curves::pallas;
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;

/// `ironwood::allocator::REGION_BYTES`: the rooted tier in `.zcash_region`.
const ROOTED_BYTES: usize = 40 * 1024;
/// `sign_pczt.SCRATCH_BYTES`: the per-session tier taken from the GC heap.
const SCRATCH_BYTES: usize = 24 * 1024;

const HEADER: usize = 16;
const UNIT: usize = 16;

/// The device allocator's block arithmetic, byte for byte.
const fn block(size: usize) -> usize {
    let size = if size == 0 { 1 } else { size };
    (HEADER + size).div_ceil(UNIT) * UNIT
}

/// Signed so that a block allocated before a window and freed inside it shows
/// up as a negative balance instead of silently clamping.
static IN_USE: AtomicIsize = AtomicIsize::new(0);
static PEAK: AtomicIsize = AtomicIsize::new(0);
static COUNT: AtomicUsize = AtomicUsize::new(0);
/// The device returns null for an alignment its 16-byte grid cannot serve, and
/// null is a fatal exit. Nothing in these traces may ask for one.
static OVERALIGNED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Only the thread inside `trace` is modelled, so the harness's own
    /// allocations never enter the arena figures.
    static TRACING: Cell<bool> = const { Cell::new(false) };
}

fn tracing() -> bool {
    TRACING.try_with(Cell::get).unwrap_or(false)
}

struct ArenaModel;

// SAFETY: every call delegates to `System`; the bookkeeping is side-channel
// only and never changes which pointer the caller receives.
unsafe impl GlobalAlloc for ArenaModel {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if tracing() {
            if layout.align() > UNIT {
                OVERALIGNED.fetch_add(1, Ordering::Relaxed);
            }
            let cost = block(layout.size()) as isize;
            let now = IN_USE.fetch_add(cost, Ordering::Relaxed) + cost;
            PEAK.fetch_max(now, Ordering::Relaxed);
            COUNT.fetch_add(1, Ordering::Relaxed);
        }
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        if tracing() {
            IN_USE.fetch_sub(block(layout.size()) as isize, Ordering::Relaxed);
        }
        unsafe { System.dealloc(pointer, layout) }
    }
}

#[global_allocator]
static MODEL: ArenaModel = ArenaModel;

/// Runs `body` with the model armed and reports the peak reached and the bytes
/// left in use, both cumulative across every earlier `trace` (the arenas are
/// not reset between phases any more than a boot resets them).
fn trace<T>(body: impl FnOnce() -> T) -> (T, isize, isize, usize) {
    let before = COUNT.load(Ordering::Relaxed);
    let peak_before = PEAK.load(Ordering::Relaxed);
    PEAK.store(IN_USE.load(Ordering::Relaxed), Ordering::Relaxed);
    TRACING.with(|flag| flag.set(true));
    let value = body();
    TRACING.with(|flag| flag.set(false));
    let peak = PEAK.load(Ordering::Relaxed);
    PEAK.store(peak.max(peak_before), Ordering::Relaxed);
    (
        value,
        peak,
        IN_USE.load(Ordering::Relaxed),
        COUNT.load(Ordering::Relaxed) - before,
    )
}

/// Streams one PCZT through a fresh `Session` and signs it: everything the
/// device does between `install_scratch` and `release_scratch`. The review is
/// held to the end because the handler holds it too -- it owns the token the
/// consent screen answers for.
fn one_session(bytes: &[u8], seed: u8) {
    let mut session = Session::with_rng(
        policy(Network::Testnet, 100_000),
        ChaCha20Rng::from_seed([seed; 32]),
    )
    .unwrap();
    session
        .begin(bytes.len(), &keys().0, &SEED_FINGERPRINT)
        .unwrap();
    let mut review = None;
    let mut rest = bytes;
    while !rest.is_empty() {
        let (consumed, event) = session.feed(rest, &keys().0).unwrap();
        rest = &rest[consumed..];
        if let Event::Review(reviewed) = event {
            review = Some(reviewed);
        }
    }
    let review = review.expect("the corpus PCZT reaches its review");
    session.approve(review.token()).unwrap();
    let _ = session.sign(review.token(), &keys().1).unwrap();
}

#[test]
fn the_two_arenas_hold_what_the_device_puts_in_them() {
    // Phase 1 — the Pasta square-root table. `prewarm` triggers it by
    // decompressing one fixed public point; `Fp::sqrt` builds `SqrtTables<Fp>`
    // by growing four 256-element `Vec<Fp>` and holding all four live, so the
    // build peak, not the resident table, is what sizes the rooted tier.
    let encoded = pallas::Point::generator().to_bytes();
    let (_, table_peak, table_retained, table_allocations) =
        trace(|| black_box(pallas::Point::from_bytes(black_box(&encoded))));

    // Phase 2 — orchard's two `OnceBox<CommitDomain>` caches, on top of the
    // table. These are the rest of the persistent set: they are filled behind
    // Rust statics the collector never scans, so whatever arena they land in
    // has to outlive every session.
    let (_, prewarm_peak, persistent, warm_allocations) = trace(ironwood::prewarm);

    // The fixtures are built only now, because building one runs the host's
    // own Orchard bundle builder -- which would root the table and the caches
    // before the two phases above could watch them being rooted. The builder is
    // not device code and its allocations are in neither arena.
    let small = fixture();
    // The session working set is O(1) in the action count -- the streaming
    // session never holds more than one action -- so the corpus maximum is
    // representative of the 32-action cap.
    let full = build_actions(CORPUS_MAX_ACTIONS);

    // Phase 3 — two sessions, with the persistent set already rooted. The
    // transient demand above `persistent` is what the scratch tier must hold,
    // and `persistent` itself must be identical after each: the device asserts
    // the same thing at `release_scratch`, where a non-zero scratch in-use
    // means a Rust static was filled during a session and now dangles.
    let (_, first_peak, after_first, _) = trace(|| one_session(&small, 1));
    let (_, second_peak, after_second, _) = trace(|| one_session(&small, 2));
    let (_, full_peak, after_full, _) = trace(|| one_session(&full, 3));

    println!("rooted tier   : {ROOTED_BYTES} B");
    println!(
        "  table build : peak {table_peak}, retained {table_retained}, {table_allocations} allocations"
    );
    println!(
        "  prewarm     : peak {prewarm_peak}, persistent {persistent}, {warm_allocations} allocations in the domain warm"
    );
    println!("  slack       : {} B", ROOTED_BYTES as isize - prewarm_peak);
    println!("scratch tier  : {SCRATCH_BYTES} B");
    println!("  session 1   : peak {}", first_peak - persistent);
    println!("  session 2   : peak {}", second_peak - persistent);
    println!(
        "  {CORPUS_MAX_ACTIONS} actions  : peak {}",
        full_peak - persistent
    );

    assert_eq!(
        OVERALIGNED.load(Ordering::Relaxed),
        0,
        "an allocation asked for more than {UNIT}-byte alignment, which the device serves as null"
    );

    // 1. The rooted tier holds the whole prewarm transient, not just the table that
    //    survives it.
    assert!(
        prewarm_peak <= ROOTED_BYTES as isize,
        "prewarm peaks at {prewarm_peak} B, over the {ROOTED_BYTES} B rooted tier"
    );

    // 2. The persistent set is the same after every session. Drift here is a
    //    cross-session leak; the device reads the same number through
    //    `debug_region_info()` between two signs in one boot.
    assert_eq!(after_first, persistent, "session 1 retained arena memory");
    assert_eq!(after_second, persistent, "session 2 retained arena memory");
    assert_eq!(
        after_full, persistent,
        "the widest session retained arena memory"
    );

    // 3. A session's transient demand fits the scratch tier, at every size.
    for (label, peak) in [
        ("session 1", first_peak),
        ("session 2", second_peak),
        ("the widest session", full_peak),
    ] {
        let scratch = peak - persistent;
        assert!(
            scratch <= SCRATCH_BYTES as isize,
            "{label} needs {scratch} B of scratch, over the {SCRATCH_BYTES} B tier"
        );
    }
}
