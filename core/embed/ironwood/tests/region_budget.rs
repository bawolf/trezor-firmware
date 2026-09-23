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
//! 3. a session's transient demand must fit the scratch arena, at the two
//!    widest shapes the wire admits as well as at the corpus sizes.
//!
//! What the model is and is not. It counts LIVE BYTES: every request priced
//! with the device's block arithmetic, and every `Vec` growth as
//! alloc-copy-free with both capacities live (`ArenaModel` keeps
//! `GlobalAlloc::realloc` at its default). The device grows in place when the
//! next block is free, and no Rust type is larger on the 32-bit target than
//! on this host, so for live bytes the model is an upper bound on the device.
//! It says nothing about FRAGMENTATION -- where first-fit puts each block, and
//! whether a request finds one hole big enough -- or about the occasionally
//! larger block first-fit hands out when a remainder is too small to split.
//! A tier can refuse a request with bytes to spare in total, which is how the
//! rooted tier failed on the first Safe 5 boot. The `rooted_tier*.rs`
//! binaries answer that half by running the real arena.

mod common;

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;
use std::sync::atomic::{AtomicIsize, AtomicUsize, Ordering};

use common::*;
use ironwood::{MAX_ACTIONS, MAX_TRANSPARENT_OUTPUTS};
use ironwood_pasta_curves::group::{Group, GroupEncoding};
use ironwood_pasta_curves::pallas;

/// `ironwood::allocator::REGION_BYTES`: the rooted tier in `.zcash_region`.
const ROOTED_BYTES: usize = 40 * 1024;
/// `sign_pczt.SCRATCH_BYTES`: the per-session tier taken from the GC heap.
const SCRATCH_BYTES: usize = 48 * 1024;

/// Bytes every session must leave unclaimed in the scratch tier, counted as
/// live bytes (see the module note: fragmentation is `rooted_tier*.rs`'s).
///
/// Small, because the widest shielded session already spends all but 208 B
/// of the tier here. The device spends less -- no type is larger there, and a
/// growth there may extend in place -- but this is the figure a host can
/// state, and below this floor the difference between the two widths would
/// be the whole margin, which is not a margin worth quoting.
const MINIMUM_MARGIN: isize = 128;

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
    let corpus = build_actions(CORPUS_MAX_ACTIONS);
    // The two widest sessions the wire admits, which are the ones that size
    // the scratch tier. Both caps are exercised because the session pre-sizes
    // a `Vec` against each and they cannot both be at their maximum: the
    // projection's `outputs` is `Vec::with_capacity(MAX_ACTIONS)` whatever
    // arrives, its `transparent_outputs` is
    // `Vec::with_capacity(header.transparent_outputs)`, and the two counts
    // share one ZIP-317 cap, so `MAX_TRANSPARENT_OUTPUTS` transparent rows
    // leave room for exactly one action.
    let wide_shielded = build_actions(MAX_ACTIONS);
    let wide_transparent = build_wide_deshield();

    // Both fixtures really are at their cap. A fixture that quietly shrank --
    // one padding decision in the host builder is enough -- would leave the
    // budget below passing on a narrower session than the wire admits, which
    // is the gap this test exists to close.
    let view = json(&wide_shielded);
    assert_eq!(
        view["ironwood"]["actions"].as_array().unwrap().len(),
        MAX_ACTIONS
    );
    assert!(
        view["transparent"]["outputs"]
            .as_array()
            .is_none_or(Vec::is_empty)
    );
    let view = json(&wide_transparent);
    assert_eq!(view["ironwood"]["actions"].as_array().unwrap().len(), 1);
    assert_eq!(
        view["transparent"]["outputs"].as_array().unwrap().len(),
        MAX_TRANSPARENT_OUTPUTS
    );

    // Phase 3 — five sessions, with the persistent set already rooted. The
    // transient demand above `persistent` is what the scratch tier must hold,
    // and `persistent` itself must be identical after each: the device asserts
    // the same thing at `release_scratch`, where a non-zero scratch in-use
    // means a Rust static was filled during a session and now dangles.
    //
    // ZIP-317 charges `5_000 * actions`, so the widest shielded bundle pays
    // 160_000 and needs a `maximum_fee` that admits it; the deshield fixtures
    // charge `TRANSPARENT_FIXTURE_FEE` whatever their bundle.
    let (_, first_peak, after_first, _) = trace(|| sign_streamed(&small, 1, 100_000));
    let (_, second_peak, after_second, _) = trace(|| sign_streamed(&small, 2, 100_000));
    let (_, full_peak, after_full, _) = trace(|| sign_streamed(&corpus, 3, 100_000));
    let (_, shielded_peak, after_shielded, _) =
        trace(|| sign_streamed(&wide_shielded, 4, 5_000 * MAX_ACTIONS as u64));
    let (_, transparent_peak, after_transparent, _) =
        trace(|| sign_streamed(&wide_transparent, 5, 100_000));

    println!("rooted tier   : {ROOTED_BYTES} B");
    println!(
        "  table build : peak {table_peak}, retained {table_retained}, {table_allocations} allocations"
    );
    println!(
        "  prewarm     : peak {prewarm_peak}, persistent {persistent}, {warm_allocations} allocations in the domain warm"
    );
    // The rooted tier has to hold the LARGER of the two phases, and it is the
    // table build, not the domain warm -- `trace` reports each phase's own
    // peak, so neither figure alone is the tier's high-water mark.
    let rooted_peak = table_peak.max(prewarm_peak);
    println!("  peak        : {rooted_peak}");
    println!("  slack       : {} B", ROOTED_BYTES as isize - rooted_peak);
    println!("scratch tier  : {SCRATCH_BYTES} B");
    for (label, peak) in [
        ("session 1", first_peak),
        ("session 2", second_peak),
        ("corpus max", full_peak),
        ("32 actions", shielded_peak),
        ("1 + 31 transparent", transparent_peak),
    ] {
        let scratch = peak - persistent;
        println!(
            "  {label:<18}: peak {scratch}, margin {}",
            SCRATCH_BYTES as isize - scratch
        );
    }

    assert_eq!(
        OVERALIGNED.load(Ordering::Relaxed),
        0,
        "an allocation asked for more than {UNIT}-byte alignment, which the device serves as null"
    );

    // 1. The rooted tier holds the whole prewarm transient, not just the table that
    //    survives it -- and the transient that binds is the table BUILD, which
    //    grows four 256-element `Vec<Fp>` and holds all four live.
    assert!(
        rooted_peak <= ROOTED_BYTES as isize,
        "prewarm peaks at {rooted_peak} B (table build {table_peak}, domain warm \
         {prewarm_peak}), over the {ROOTED_BYTES} B rooted tier"
    );

    // 2. The persistent set is the same after every session. Drift here is a
    //    cross-session leak; the device reads the same number through
    //    `debug_region_info()` between two signs in one boot.
    for (label, after) in [
        ("session 1", after_first),
        ("session 2", after_second),
        ("the corpus maximum", after_full),
        ("the widest shielded session", after_shielded),
        ("the widest deshield", after_transparent),
    ] {
        assert_eq!(after, persistent, "{label} retained arena memory");
    }

    // 3. A session's transient demand fits the scratch tier at every size, with
    //    something left to argue about.
    for (label, peak) in [
        ("session 1", first_peak),
        ("session 2", second_peak),
        ("the corpus maximum", full_peak),
        ("the widest shielded session", shielded_peak),
        ("the widest deshield", transparent_peak),
    ] {
        let margin = SCRATCH_BYTES as isize - (peak - persistent);
        assert!(
            margin >= MINIMUM_MARGIN,
            "{label} leaves {margin} B of the {SCRATCH_BYTES} B scratch tier, \
             under the stated {MINIMUM_MARGIN} B floor"
        );
    }

    // 4. The session working set is O(1) in the action count all the way to the
    //    cap: the streaming session holds one action at a time, and the only
    //    per-bundle term is the 32 logical-action slots both projection vectors
    //    share. A widest-shielded peak above the corpus peak would mean something
    //    now scales with the count, which is the shape of growth the 208 B margin
    //    cannot absorb.
    assert_eq!(
        shielded_peak, full_peak,
        "{MAX_ACTIONS} actions cost more than {CORPUS_MAX_ACTIONS}: the session \
         working set is no longer O(1) in the action count"
    );
}
