//! Session-start pre-warm of the Pasta square-root table.
//!
//! Pasta keeps its `SqrtTables<Fp>` (~29.8 KB) behind a lazy Rust `static` that
//! the MicroPython GC never scans. On the device the signing scratch lives in a
//! per-session region; if that table is first built mid-stream (interleaved
//! with ~KBs of transient per-action verify scratch), it is left at a high
//! region offset when the scratch frees and splits the region — the
//! fragmentation that faulted the 8-action run.
//!
//! [`prewarm`] forces that table to build ONCE at session start, right after
//! the region is installed and before any per-action scratch, by decompressing
//! a single fixed public point. Point decompression recovers `y` from `x` via
//! an `Fp` square root, which is exactly what allocates the table.
//!
//! It also warms orchard's two `OnceBox<CommitDomain>` caches. Those caches
//! (NoteCommit and CommitIvk) live
//! behind Rust statics the GC never scans and allocate through this region on
//! first use. Left to fill mid-action-0, their two ~200 B blocks land at
//! arbitrary offsets interleaved with transient verify scratch and then survive
//! for the whole boot, permanently splitting the region
//! ([free][island][free]...) so session 2's layout (and its high-water) differ
//! from session 1's. Warming them here places them contiguous+early next to the
//! Pasta table. This costs one note commitment + one `commit_ivk`, and the
//! domain builds themselves are the same two `hash_to_curve` per domain that
//! action 0 would pay anyway. It touches no wallet material: the fixed
//! throwaway key below is a constant, not a secret.

use core::hint::black_box;
use core::sync::atomic::{AtomicBool, Ordering};

use ironwood_pasta_curves::group::{Group, GroupEncoding};
use ironwood_pasta_curves::pallas;
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use orchard::note::{Note, NoteVersion, RandomSeed, Rho};
use orchard::value::NoteValue;

/// Set once `prewarm` has rooted the caches this boot. The signing region is
/// boot-rooted (`install_region` formats it exactly once per boot and is a
/// no-op thereafter), so the Pasta sqrt table and orchard `OnceBox` caches this
/// builds survive every later session. Re-running the ~1.4 s of constant-input
/// Sinsemilla / `commit_ivk` work on every `session_begin` is therefore pure
/// waste; run it once per boot.
static WARMED: AtomicBool = AtomicBool::new(false);

/// Roots the persistent Pasta square-root table AND orchard's two
/// `OnceBox<CommitDomain>` caches in the freshly installed region before the
/// per-action loop, once per boot. Touches no signing material and changes no
/// signing behaviour.
pub fn prewarm() {
    // Once per boot: the caches built below live in the boot-lifetime region and
    // persist across sessions, so a second run only costs device time. Single-
    // threaded firmware, so a relaxed test-and-set is sufficient.
    if WARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    // `to_bytes` on the generator needs no square root; `from_bytes` recovers
    // `y` via `Fp::sqrt`, which builds the lazy `SqrtTables<Fp>` in the region.
    let encoded = pallas::Point::generator().to_bytes();
    let _ = black_box(pallas::Point::from_bytes(black_box(&encoded)));

    warm_orchard_domains();
}

/// Forces orchard's `commit_ivk` and `note_commit` `OnceBox<CommitDomain>`
/// caches to allocate now, at prewarm, so they sit low and contiguous instead
/// of fragmenting the region mid-action-0. Best-effort and never panics: if a
/// constant somehow fails to yield a valid key/note the caches simply fill on
/// first real use (correctness is unaffected either way).
fn warm_orchard_domains() {
    // A fixed, non-secret throwaway spending key. `from_bytes` rejects a few
    // byte patterns (ask/ivk == 0), so scan a handful of constants for a valid
    // one; [1; 32] is valid in practice, the loop is just belt-and-braces.
    let mut found = None;
    for seed in 1u8..=32 {
        let sk = SpendingKey::from_bytes([seed; 32]);
        if bool::from(sk.is_some()) {
            found = Some(sk.unwrap());
            break;
        }
    }
    let fvk = match found {
        Some(sk) => FullViewingKey::from(&sk),
        None => return,
    };

    // CommitIvk domain: building the scope classifier derives the external and
    // internal ivks via `spec::commit_ivk`, which fills `commit_ivk_domain` — the
    // exact call `Session::feed` makes on action 0.
    let _ = black_box(fvk.scope_classifier());

    // NoteCommit domain: evaluate one note commitment on a throwaway note built
    // from the fvk's own address; `Note::commitment()` calls
    // `NoteCommitment::derive`, which fills `note_commit_domain`.
    let address = fvk.address_at(0u32, Scope::External);
    let value = NoteValue::from_raw(0);
    let rho = match Option::<Rho>::from(Rho::from_bytes(&[0u8; 32])) {
        Some(rho) => rho,
        None => return,
    };
    for seed in 0u8..=32 {
        let rseed = match Option::<RandomSeed>::from(RandomSeed::from_bytes([seed; 32], &rho)) {
            Some(rseed) => rseed,
            None => continue,
        };
        if let Some(note) = Option::<Note>::from(Note::from_parts(
            address,
            value,
            rho,
            rseed,
            NoteVersion::V3,
        )) {
            let _ = black_box(note.commitment());
            return;
        }
    }
}
