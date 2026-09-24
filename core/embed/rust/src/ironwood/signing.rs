//! Device-side Zcash key derivation: the account's Orchard full viewing key
//! and the wallet's ZIP-32 seed fingerprint.
//!
//! The MicroPython bindings live in `micropython/zcash.rs`, keeping the
//! logic/FFI split `thp/mod.rs` and `thp/micropython.rs` use. The wallet seed
//! is lent for one call and never retained; every key object derived here is
//! wiped before the memory is reused, either by its own `Drop` or by the
//! explicit `wipe` below. The temporaries underneath them -- `zip32`'s
//! `HardenedOnlyKey`, which is not `Zeroize` -- are cleared by the handler's
//! `utils.zero_unused_stack()`, the way every other seed-touching Trezor call
//! does it.

use core::ptr;

use ironwood::Network;
use orchard::keys::{FullViewingKey, SpendingKey};

/// Seed lengths the receive module admits (`ironwood/src/receive/keys.rs`):
/// a restored SLIP-39 secret or a ZIP-32 seed. Kept identical so an account
/// that can show an address can also sign.
const RESTORED_SLIP39_SEED_BYTES: usize = 16;
const MIN_ZIP32_SEED_BYTES: usize = 32;
const MAX_SEED_BYTES: usize = 252;

/// Overwrites a plain-data value with zeros in place, surviving optimisation.
/// Only for types without heap ownership or a `Drop` that reads them; every
/// orchard key type used here is such a value (field elements and points).
fn wipe<T>(value: &mut T) {
    let bytes = (value as *mut T).cast::<u8>();
    for offset in 0..core::mem::size_of::<T>() {
        // SAFETY: `offset` is within the object `value` points to.
        unsafe { ptr::write_volatile(bytes.add(offset), 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

/// The seed lengths the device derives from: a restored SLIP-39 secret or a
/// ZIP-32 seed. One rule for signing and for the seed fingerprint, so a
/// wallet that can sign can also be identified.
fn admissible_seed(seed: &[u8]) -> bool {
    seed.len() == RESTORED_SLIP39_SEED_BYTES
        || (MIN_ZIP32_SEED_BYTES..=MAX_SEED_BYTES).contains(&seed.len())
}

/// The device's ZIP-32 seed fingerprint ([`ironwood::seed_fingerprint`])
/// into `output`. A public identifier of the seed, not key material; `seed`
/// is borrowed for this call only.
///
/// For a 16-byte restored SLIP-39 secret this is a Trezor-only extension of
/// the ZIP-32 construction (see [`ironwood::seed_fingerprint`]): the
/// standard defines no fingerprint that short, so the device's value is the
/// authoritative one for such a wallet.
pub fn seed_fingerprint(seed: &[u8], output: &mut [u8; 32]) -> core::result::Result<(), Failure> {
    if !admissible_seed(seed) {
        return Err(Failure::State);
    }
    *output = ironwood::seed_fingerprint(seed).ok_or(Failure::State)?;
    Ok(())
}

/// The account's Orchard full viewing key as `ak ‖ nk ‖ rivk`, derived
/// through `orchard` -- the implementation that signs.
///
/// This exists so the viewing-key export can be checked against the
/// independent port in `ironwood::receive`, which is what derives addresses.
/// Two implementations of ZIP-32 and the Orchard key expansion ship in this
/// image; if they ever disagreed for some seed the wallet would receive to
/// addresses the signing path cannot spend from, and every real spend would
/// fail the session's `fvk` equality check. Funds stuck, not stolen, and
/// silently.
///
/// `orchard`'s `from_zip32_seed` goes through `zip32`'s `HardenedOnlyKey`,
/// which puts no lower bound on the seed, so this covers the 16-byte restored
/// SLIP-39 secret too -- the one length with no external oracle.
pub fn orchard_full_viewing_key(
    seed: &[u8],
    network: Network,
    account: u32,
) -> core::result::Result<[u8; 96], Failure> {
    let keys = AccountKeys::derive(seed, coin_type(network), account).ok_or(Failure::State)?;
    let mut fvk = keys.full_viewing_key();
    let bytes = fvk.to_bytes();
    wipe(&mut fvk);
    Ok(bytes)
}

/// Derived keys of `m/32'/coin_type'/account'`. The spending key is wiped on
/// drop; callers move the derived keys out and wipe those themselves.
struct AccountKeys {
    sk: SpendingKey,
}

impl AccountKeys {
    fn derive(seed: &[u8], coin_type: u32, account: u32) -> Option<Self> {
        if !admissible_seed(seed) {
            return None;
        }
        // The account type is `zip32::AccountId`, inferred from the parameter so
        // this crate needs no direct `zip32` dependency; `try_into` rejects the
        // hardened bit exactly as `Account::new` did.
        let sk = SpendingKey::from_zip32_seed(seed, coin_type, account.try_into().ok()?).ok()?;
        Some(Self { sk })
    }

    fn full_viewing_key(&self) -> FullViewingKey {
        FullViewingKey::from(&self.sk)
    }
}

impl Drop for AccountKeys {
    fn drop(&mut self) {
        wipe(&mut self.sk);
    }
}

fn coin_type(network: Network) -> u32 {
    match network {
        Network::Mainnet => 133,
        Network::Testnet => 1,
    }
}

/// Failure classes the handler maps to wire failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// Calls were made out of order, or a derivation input is unusable.
    State,
}
