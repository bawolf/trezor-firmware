//! Session-start pre-warm of the Pasta square-root table (SHOULD-FIX #4).
//!
//! Pasta keeps its `SqrtTables<Fp>` (~29.8 KB) behind a lazy Rust `static` that
//! the MicroPython GC never scans. On the device the signing scratch lives in a
//! per-session region; if that table is first built mid-stream (interleaved
//! with ~KBs of transient per-action verify scratch), it is left at a high
//! region offset when the scratch frees and splits the region — the
//! fragmentation that faulted the 8-action run (RAM-RETENTION-ANALYSIS.md,
//! lever R2).
//!
//! [`prewarm`] forces that table to build ONCE at session start, right after
//! the region is installed and before any per-action scratch, by decompressing
//! a single fixed public point. Point decompression recovers `y` from `x` via
//! an `Fp` square root, which is exactly what allocates the table. Unlike the
//! former `bench::warmup` this does NO Sinsemilla hashing (no `CommitDomain`,
//! no `hash_to_point`), so it adds none of the ~2-3 s of wasted per-transaction
//! latency, and it lives on the production signing path with no `bench`
//! coupling (removing `bench` can no longer silently drop the pre-warm).

use core::hint::black_box;

use ironwood_pasta_curves::group::{Group, GroupEncoding};
use ironwood_pasta_curves::pallas;

/// Roots the persistent Pasta square-root table in the freshly installed region
/// before the per-action loop. Decompresses the Pallas generator (a single
/// public point); touches no signing material and changes no signing behaviour.
pub fn prewarm() {
    // `to_bytes` on the generator needs no square root; `from_bytes` recovers
    // `y` via `Fp::sqrt`, which builds the lazy `SqrtTables<Fp>` in the region.
    let encoded = pallas::Point::generator().to_bytes();
    let _ = black_box(pallas::Point::from_bytes(black_box(&encoded)));
}
