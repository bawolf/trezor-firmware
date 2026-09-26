//! Builds the lazily allocated Pasta square-root table and orchard's
//! `CommitDomain` caches before the first session, so these long-lived
//! allocations sit at the bottom of the heap rather than between per-action
//! allocations.

use core::hint::black_box;
use core::sync::atomic::{AtomicBool, Ordering};

use orchard::keys::{Diversifier, FullViewingKey, Scope, SpendingKey};
use orchard::note::{Note, NoteVersion, RandomSeed, Rho};
use orchard::value::NoteValue;
use pasta_curves::group::{Group, GroupEncoding};
use pasta_curves::pallas;

/// A publicly known spending key and note seed, used only to fill the caches.
const WARM_KEY: [u8; 32] = [1; 32];
const WARM_RSEED: [u8; 32] = [0; 32];

/// The caches are statics, so they are built once.
static WARMED: AtomicBool = AtomicBool::new(false);

/// Builds the caches, calling `progress` between its steps and during their
/// Sinsemilla hashes.
pub fn prewarm(progress: &mut dyn FnMut()) {
    if WARMED.swap(true, Ordering::Relaxed) {
        return;
    }
    // Decompressing a point takes a square root, which builds the table.
    let encoded = pallas::Point::generator().to_bytes();
    let _ = black_box(pallas::Point::from_bytes(black_box(&encoded)));
    progress();

    let key: Option<SpendingKey> = SpendingKey::from_bytes_with_progress(WARM_KEY, progress).into();
    let Some(key) = key else {
        return;
    };
    progress();
    let fvk = FullViewingKey::from(&key);
    progress();
    // Fills the `Commit^ivk` domain.
    let _ = black_box(fvk.scope_classifier_with_progress(progress));
    progress();
    // Fills the note commitment domain. A raw diversifier, since `address_at`
    // would link FF1 from `fpe`.
    let address =
        fvk.address_with_progress(Diversifier::from_bytes([0; 11]), Scope::External, progress);
    progress();
    let rho: Option<Rho> = Rho::from_bytes(&[0; 32]).into();
    let Some(rho) = rho else {
        return;
    };
    let rseed: Option<RandomSeed> = RandomSeed::from_bytes(WARM_RSEED, &rho).into();
    let Some(rseed) = rseed else {
        return;
    };
    let value = NoteValue::from_raw(0);
    let _ = black_box(Note::from_parts_with_progress(
        address,
        value,
        rho,
        rseed,
        NoteVersion::V3,
        progress,
    ));
    progress();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_constants_are_valid() {
        let key: Option<SpendingKey> = SpendingKey::from_bytes(WARM_KEY).into();
        let fvk = FullViewingKey::from(&key.unwrap());
        let rho: Option<Rho> = Rho::from_bytes(&[0; 32]).into();
        let rho = rho.unwrap();
        let rseed: Option<RandomSeed> = RandomSeed::from_bytes(WARM_RSEED, &rho).into();
        let address = fvk.address(Diversifier::from_bytes([0; 11]), Scope::External);
        let note = Note::from_parts(
            address,
            NoteValue::from_raw(0),
            rho,
            rseed.unwrap(),
            NoteVersion::V3,
        );
        assert!(bool::from(note.is_some()));
    }

    #[test]
    fn test_runs_once() {
        let mut calls = 0;
        prewarm(&mut || calls += 1);
        assert!(calls > 0);
        calls = 0;
        prewarm(&mut || calls += 1);
        assert_eq!(calls, 0);
    }
}
