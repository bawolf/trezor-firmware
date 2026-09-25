//! TEST ONLY, never enabled by default. Derives the account keys from the seed
//! of the PUBLIC test mnemonic inside the app, so that the app can be tested
//! before coreapp can hand it keys. Delete this module and the feature then.

use blake2b_simd::Params;
use ironwood::receive::Network;
use zeroize::Zeroize;

use crate::account::AccountKeys;

/// BIP-39 seed of "abandon abandon abandon abandon abandon abandon abandon
/// abandon abandon abandon abandon about" with an empty passphrase.
const SEED: [u8; 64] = [
    0x5e, 0xb0, 0x0b, 0xbd, 0xdc, 0xf0, 0x69, 0x08, 0x48, 0x89, 0xa8, 0xab, 0x91, 0x55, 0x56, 0x81,
    0x65, 0xf5, 0xc4, 0x53, 0xcc, 0xb8, 0x5e, 0x70, 0x81, 0x1a, 0xae, 0xd6, 0xf6, 0xda, 0x5f, 0xc1,
    0x9a, 0x5a, 0xc4, 0x0b, 0x38, 0x9c, 0xd3, 0x70, 0xd0, 0x86, 0x20, 0x6d, 0xec, 0x8a, 0xa6, 0xc4,
    0x3d, 0xae, 0xa6, 0x69, 0x0f, 0x20, 0xad, 0x3d, 0x8d, 0x48, 0xb2, 0xd2, 0xce, 0x9e, 0x38, 0xe4,
];

fn blake2b_512(personal: &[u8; 16], parts: &[&[u8]]) -> [u8; 64] {
    let mut state = Params::new().hash_length(64).personal(personal).to_state();
    for part in parts {
        state.update(part);
    }
    let mut output = [0; 64];
    output.copy_from_slice(state.finalize().as_bytes());
    output
}

/// ZIP 32: Orchard master key generation and hardened child derivation of
/// `m/32'/coin_type'/account'`; mirrors `ExtendedSpendingKey` in
/// `ironwood::receive::keys`, without its per-level key validation.
pub(crate) fn account_keys(network: Network, account: u32) -> AccountKeys {
    let mut key = blake2b_512(b"ZcashIP32Orchard", &[&SEED]);
    for index in [32, network.coin_type(), account] {
        let (spending_key, chain_code) = key.split_at(32);
        let index = (index | 1 << 31).to_le_bytes();
        let child = blake2b_512(
            b"Zcash_ExpandSeed",
            &[chain_code, &[0x81], spending_key, &index],
        );
        key.zeroize();
        key = child;
    }
    let mut spending_key = [0; 32];
    spending_key.copy_from_slice(&key[..32]);
    key.zeroize();
    AccountKeys {
        spending_key,
        seed_fingerprint: ironwood::seed_fingerprint(&SEED).unwrap_or_default(),
        // 12 words
        weak_backup: true,
    }
}
