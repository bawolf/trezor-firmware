//! Session-start pre-warm of the Pasta square-root table.
//!
//! Pasta keeps its `SqrtTables<Fp>` (~29.8 KB) behind a lazy Rust `static`
//! that lives for the rest of the program. If that table is first built
//! mid-stream (interleaved with ~KBs of transient per-action verify scratch),
//! it is left at a high heap offset when the scratch frees and splits the
//! device's small heap.
//!
//! [`prewarm`] forces that table to build ONCE at session start, before any
//! per-action scratch, by decompressing a single fixed public point. Point
//! decompression recovers `y` from `x` via an `Fp` square root, which is
//! exactly what allocates the table.
//!
//! It also warms orchard's two `OnceBox<CommitDomain>` caches (NoteCommit and
//! CommitIvk), which also live behind Rust statics and allocate on first use.
//! Left to fill mid-action-0, their two ~200 B blocks land at arbitrary offsets
//! interleaved with transient verify scratch and then survive, permanently
//! splitting the heap ([free][island][free]...) so session 2's layout (and its
//! high-water) differ from session 1's. Warming them here places them
//! contiguous and early next to the Pasta table. This costs one note
//! commitment + one `commit_ivk`, and the domain builds themselves are the same
//! two `hash_to_curve` per domain that action 0 would pay anyway. It touches no
//! wallet material: the fixed throwaway key below is a constant, not a secret.
//!
//! On a Cortex-M33 each call below takes up to ~0.25 s, so
//! [`prewarm_with_progress`] reports progress after every one of them.

use core::hint::black_box;
use core::sync::atomic::{AtomicBool, Ordering};

use ironwood_pasta_curves::group::{Group, GroupEncoding};
use ironwood_pasta_curves::pallas;
use orchard::keys::{Diversifier, FullViewingKey, Scope, SpendingKey};
use orchard::note::{Note, NoteVersion, RandomSeed, Rho};
use orchard::value::NoteValue;

/// Set once `prewarm` has built the caches. They are statics, so they survive
/// every later session; re-running the ~1.4 s of constant-input Sinsemilla /
/// `commit_ivk` work on every `session_begin` would be pure waste.
static WARMED: AtomicBool = AtomicBool::new(false);

/// Roots the persistent Pasta square-root table AND orchard's two
/// `OnceBox<CommitDomain>` caches before the per-action loop, once. Touches no signing material and changes no
/// signing behaviour.
pub fn prewarm() {
    prewarm_with_progress(&mut || {});
}

/// [`prewarm`], calling `progress` after each of its expensive calls, for a
/// caller that must report progress while it runs.
pub fn prewarm_with_progress(progress: &mut dyn FnMut()) {
    // Once: the caches built below persist across sessions, so a second run
    // only costs device time. Single-threaded, so a relaxed test-and-set is
    // sufficient.
    if WARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    // `to_bytes` on the generator needs no square root; `from_bytes` recovers
    // `y` via `Fp::sqrt`, which builds the lazy `SqrtTables<Fp>`.
    let encoded = pallas::Point::generator().to_bytes();
    let _ = black_box(pallas::Point::from_bytes(black_box(&encoded)));
    progress();

    warm_orchard_domains(progress);
}

/// Forces orchard's `commit_ivk` and `note_commit` `OnceBox<CommitDomain>`
/// caches to allocate now, at prewarm, so they sit low and contiguous instead
/// of fragmenting the heap mid-action-0. Best-effort and never panics: if a
/// constant somehow fails to yield a valid key/note the caches simply fill on
/// first real use (correctness is unaffected either way).
fn warm_orchard_domains(progress: &mut dyn FnMut()) {
    // A fixed, non-secret throwaway spending key. `from_bytes` rejects a few
    // byte patterns (ask/ivk == 0), so scan a handful of constants for a valid
    // one; [1; 32] is valid in practice, the loop is just belt-and-braces.
    let mut found = None;
    for seed in 1u8..=32 {
        let sk = SpendingKey::from_bytes([seed; 32]);
        progress();
        if bool::from(sk.is_some()) {
            found = Some(sk.unwrap());
            break;
        }
    }
    let fvk = match found {
        Some(sk) => FullViewingKey::from(&sk),
        None => return,
    };
    progress();

    // CommitIvk domain: building the scope classifier derives the external and
    // internal ivks via `spec::commit_ivk`, which fills `commit_ivk_domain` — the
    // exact call `Session::feed` makes on action 0.
    let _ = black_box(fvk.scope_classifier());
    progress();

    // NoteCommit domain: evaluate one note commitment on a throwaway note built
    // from one of the fvk's addresses; `Note::from_parts` checks that the note
    // has a commitment, which fills `note_commit_domain`. The address comes
    // from a raw diversifier, not `address_at`, whose FF1 index encryption
    // would link the `fpe`/`num-bigint` code nothing else on the device uses.
    let address = fvk.address(Diversifier::from_bytes([0; 11]), Scope::External);
    progress();
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
        let note = Note::from_parts(address, value, rho, rseed, NoteVersion::V3);
        progress();
        if bool::from(note.is_some()) {
            return;
        }
    }
}
