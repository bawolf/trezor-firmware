//! The device's seed fingerprint against the canonical implementation.

use trezor_ironwood::seed_fingerprint;
use zip32::fingerprint::SeedFingerprint;

/// For every seed length ZIP 32 admits the device fingerprint is the ZIP-32
/// seed fingerprint byte for byte.
#[test]
fn matches_zip32_for_admitted_seed_lengths() {
    for length in [32usize, 33, 64, 128, 251, 252] {
        let seed: Vec<u8> = (0..length).map(|i| (i * 7 + length) as u8).collect();
        assert_eq!(
            seed_fingerprint(&seed),
            Some(SeedFingerprint::from_seed(&seed).unwrap().to_bytes()),
            "{length}"
        );
    }
}

/// The 16-byte restored SLIP-39 secret the device also derives from is below
/// ZIP 32's range (the canonical function refuses it); the device applies the
/// same construction with length byte 16. Pinned so an export and a wire
/// claim for such a wallet can never drift apart.
#[test]
fn extends_the_construction_to_the_restored_slip39_secret() {
    let seed = [0x11u8; 16];
    assert!(SeedFingerprint::from_seed(&seed).is_none());
    let expected = blake2b_simd::Params::new()
        .hash_length(32)
        .personal(b"Zcash_HD_Seed_FP")
        .to_state()
        .update(&[16])
        .update(&seed)
        .finalize();
    assert_eq!(
        seed_fingerprint(&seed).unwrap()[..],
        expected.as_bytes()[..]
    );
}

/// No fingerprint where the length byte cannot express the length, or for
/// nothing at all.
#[test]
fn refuses_lengths_outside_the_length_byte() {
    assert_eq!(seed_fingerprint(&[]), None);
    assert_eq!(seed_fingerprint(&[0; 253]), None);
    assert!(seed_fingerprint(&[0; 252]).is_some());
}
