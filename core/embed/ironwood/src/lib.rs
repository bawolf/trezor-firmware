#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::all)]
//! Orchard receiver and viewing-key derivation for the Zcash application.
//!
//! The device derives its own Orchard receivers and full viewing keys from the
//! wallet seed; the host selects only the network and the ZIP-32 account, and
//! the device validates both. Nothing in this crate retains seed material: the
//! key expansion runs inside `receive` and zeroizes its intermediates.
//!
//! The protocol this serves is written down in
//! `docs/common/zcash-ironwood-signing.md`.

#[cfg(all(feature = "test", target_os = "none"))]
compile_error!("the `test` feature is host-only and must not enter bare-metal firmware");

pub mod receive;

/// Production network selection.
///
/// Regtest is not selectable, but its coin type and branch are
/// indistinguishable from Testnet in the PCZT header. Test-only local-consensus
/// fixtures therefore remain admissible under Testnet; production callers must
/// derive from the selected network and account themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Network {
    Mainnet = 0,
    Testnet = 1,
}

/// ZIP-32 seed fingerprint: `BLAKE2b-256("Zcash_HD_Seed_FP", I2LEOSP_8(len) ‖
/// seed)` (ZIP 32 §"Seed Fingerprints"), computed over exactly the bytes the
/// device feeds into ZIP-32 master derivation. It equals
/// `zip32::fingerprint::SeedFingerprint::from_seed` for every seed length ZIP
/// 32 admits (32..=252), pinned byte-for-byte by `tests/seed_fingerprint.rs`;
/// `None` for an empty seed and above ZIP 32's 252-byte maximum. A public
/// identifier of the seed, not key material: hosts attach it as the
/// `seed_fingerprint` of a `zip32_derivation` and wallets store it next to
/// the account index.
///
/// **Trezor-only extension, 16-byte seeds.** ZIP 32 defines no fingerprint
/// below 32 bytes and `zip32` returns `None` there, but the device also
/// derives from the 16-byte secret of a restored SLIP-39 backup (itself
/// already outside ZIP 32's master-derivation range). For those wallets this
/// applies the identical construction with length byte 16 rather than
/// refusing the export, so the exported value and the value the wire check
/// compares against can never disagree. The consequence for hosts: for a
/// 16-byte-seed wallet the device's value is authoritative and there is no
/// second implementation to derive it from -- take it from
/// `ZcashGetViewingKey(include_seed_fingerprint=true)`, do not recompute it.
/// If ZIP 32 ever defines a rule for sub-32-byte seeds, this is the decision
/// to revisit.
pub fn seed_fingerprint(seed: &[u8]) -> Option<[u8; 32]> {
    if seed.is_empty() || seed.len() > 252 {
        return None;
    }
    let length = seed.len() as u8;
    let mut fingerprint = [0; 32];
    fingerprint.copy_from_slice(
        blake2b_simd::Params::new()
            .hash_length(32)
            .personal(b"Zcash_HD_Seed_FP")
            .to_state()
            .update(&[length])
            .update(seed)
            .finalize()
            .as_bytes(),
    );
    Some(fingerprint)
}
