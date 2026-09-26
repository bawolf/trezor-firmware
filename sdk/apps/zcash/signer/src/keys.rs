//! Orchard receivers, with a progress callback during their Sinsemilla hash.

use blake2b_simd::Params;
use orchard::keys::{Diversifier, FullViewingKey, Scope};
use zeroize::Zeroizing;

use crate::ff1;

/// The external receiver of `fvk` at `diversifier_index` (ZIP 32), as
/// `FullViewingKey::address_at` derives it.
pub fn external_receiver(
    fvk: &FullViewingKey,
    diversifier_index: [u8; 11],
    progress: &mut dyn FnMut(),
) -> [u8; 43] {
    let diversifier = ff1::encrypt_diversifier_index(&diversifier_key(fvk), diversifier_index);
    fvk.address_with_progress(
        Diversifier::from_bytes(diversifier),
        Scope::External,
        progress,
    )
    .to_raw_address_bytes()
}

/// `dk`, the first half of `PRF^expand_rivk(0x82 || ak || nk)`.
fn diversifier_key(fvk: &FullViewingKey) -> Zeroizing<[u8; 32]> {
    let encoding = Zeroizing::new(fvk.to_bytes());
    let (ak_nk, rivk) = encoding.split_at(64);
    let hash = Params::new()
        .hash_length(64)
        .personal(b"Zcash_ExpandSeed")
        .to_state()
        .update(rivk)
        .update(&[0x82])
        .update(ak_nk)
        .finalize();
    let mut key = Zeroizing::new([0; 32]);
    key.copy_from_slice(&hash.as_bytes()[..32]);
    key
}

#[cfg(test)]
mod tests {
    use orchard::keys::{DiversifierIndex, SpendingKey};

    use super::*;

    #[test]
    fn test_external_receiver_matches_orchard() {
        let key: Option<SpendingKey> = SpendingKey::from_bytes([7; 32]).into();
        let fvk = FullViewingKey::from(&key.unwrap());
        let indices = [[0; 11], [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0], [0xff; 11]];
        for index in indices {
            let mut calls = 0;
            let receiver = external_receiver(&fvk, index, &mut || calls += 1);
            let expected = fvk.address_at(DiversifierIndex::from(index), Scope::External);
            assert_eq!(receiver, expected.to_raw_address_bytes());
            assert!(calls > 0);
        }
    }
}
