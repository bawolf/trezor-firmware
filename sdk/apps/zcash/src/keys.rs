use orchard::keys::{FullViewingKey, SpendingKey};
use trezor_app_sdk::crypto::{self, Zip32OrchardAccount};
use trezor_app_sdk::{Error, Result};
use zcash_signer::Network;

use crate::layout::show_weak_backup_warning;

/// The account's keys from Core's key service, after the ZIP-315 warning for
/// a weak backup. The first request for an account asks the user to allow it.
pub fn account_keys(network: Network, account: u32) -> Result<Zip32OrchardAccount> {
    let keys = crypto::get_zip32_orchard_account(network.coin_type(), account)?;
    if keys.weak_backup {
        show_weak_backup_warning()?;
    }
    Ok(keys)
}

/// The account's spending key, checked here because Core cannot tell whether
/// its bytes are a valid Orchard key.
pub fn spending_key(
    keys: &Zip32OrchardAccount,
    progress: &mut dyn FnMut(),
) -> Result<Wiped<SpendingKey>> {
    Option::from(SpendingKey::from_bytes_with_progress(
        keys.spending_key,
        progress,
    ))
    .map(Wiped::new)
    .ok_or(Error::DataError("Invalid Zcash spending key"))
}

/// An orchard key type that is plain data: no pointers, no `Drop`, and valid
/// when zeroed.
///
/// # Safety
///
/// Implement only for types that `zeroize::zeroize_flat_type` accepts.
pub unsafe trait FlatKey {}

// SAFETY: a 32-byte array.
unsafe impl FlatKey for SpendingKey {}
// SAFETY: two Pallas field elements and a RedPallas verification key (a point
// and its encoding).
unsafe impl FlatKey for FullViewingKey {}

/// An orchard key, zeroed when dropped. Orchard's keys do not implement
/// `Zeroize`.
pub struct Wiped<T: FlatKey>(T);

impl<T: FlatKey> Wiped<T> {
    pub fn new(key: T) -> Self {
        Self(key)
    }
}

impl<T: FlatKey> core::ops::Deref for Wiped<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: FlatKey> Drop for Wiped<T> {
    fn drop(&mut self) {
        // SAFETY: `T: FlatKey`, and the key is not used after this.
        unsafe { zeroize::zeroize_flat_type(&mut self.0) }
    }
}
