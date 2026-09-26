//! Request validation, account labels and the account's key material.

use alloc::string::String;

use ironwood::receive::Network;
use orchard::keys::SpendingKey;
use trezor_app_sdk::{Error, Result, crypto};
use zcash_protocol::consensus::NetworkType;

use crate::proto::zcash::ZcashNetwork;

/// A required request field is absent or has the wrong size.
pub(crate) fn malformed() -> Error {
    Error::DataError("Malformed Zcash request")
}

/// A request field selects something the device does not allow.
pub(crate) fn policy_violation() -> Error {
    Error::DataError("Zcash request violates device policy")
}

pub(crate) fn network_from_request(network: Option<i32>) -> Result<Network> {
    match ZcashNetwork::try_from(network.ok_or_else(malformed)?) {
        Ok(ZcashNetwork::Mainnet) => Ok(Network::Mainnet),
        Ok(ZcashNetwork::Testnet) => Ok(Network::Testnet),
        Err(_) => Err(policy_violation()),
    }
}

/// The ZIP-32 account index, below 2^31.
pub(crate) fn account_from_request(account: Option<u32>) -> Result<u32> {
    match account.ok_or_else(malformed)? {
        account if account < 1 << 31 => Ok(account),
        _ => Err(policy_violation()),
    }
}

pub(crate) fn network_label(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "Mainnet",
        Network::Testnet => "Testnet",
    }
}

pub(crate) fn network_type(network: Network) -> NetworkType {
    match network {
        Network::Mainnet => NetworkType::Main,
        Network::Testnet => NetworkType::Test,
    }
}

/// "ZEC #1" for account 0.
pub(crate) fn account_label(account: u32) -> String {
    uformat!("ZEC #{}", account + 1)
}

/// "m/32'/133'/0'" for mainnet account 0.
pub(crate) fn account_path(network: Network, account: u32) -> String {
    uformat!("m/32'/{}'/{}'", network.coin_type(), account)
}

/// A ZIP-32 Orchard account's key material, from Core's key service. The
/// spending key is overwritten with zeros on drop.
pub(crate) struct AccountKeys {
    pub(crate) spending_key: SpendingKey,
    pub(crate) seed_fingerprint: [u8; 32],
    /// ZIP 315: the backup is a 12- or 18-word mnemonic or a 128-bit SLIP-39
    /// secret, which wallets should warn about.
    pub(crate) weak_backup: bool,
}

impl Drop for AccountKeys {
    fn drop(&mut self) {
        wipe(&mut self.spending_key);
    }
}

/// The only place the app obtains key material: account `account` of
/// `m/32'/coin_type'/account'`, from Core's key service. The first request
/// for an account of this app instance asks the user to allow it.
pub(crate) fn account_keys(network: Network, account: u32) -> Result<AccountKeys> {
    let keys = crypto::get_zip32_orchard_account(network.coin_type(), account)?;
    // Core derives with BLAKE2b alone and cannot tell whether the key is a
    // valid Orchard spending key; `orchard` refuses the rare one that is not.
    let spending_key = Option::from(SpendingKey::from_bytes(keys.spending_key))
        .ok_or(Error::DataError("Zcash key derivation failed"))?;
    Ok(AccountKeys {
        spending_key,
        seed_fingerprint: keys.seed_fingerprint,
        weak_backup: keys.weak_backup,
    })
}

/// Overwrites a plain-data value with zeros in place, surviving optimisation.
/// Only for types without heap ownership or a `Drop` that reads them, such as
/// the `orchard` key types, which implement no `Zeroize`.
pub(crate) fn wipe<T>(value: &mut T) {
    let bytes = (value as *mut T).cast::<u8>();
    for offset in 0..core::mem::size_of::<T>() {
        // SAFETY: `offset` is within the object `value` points to.
        unsafe { core::ptr::write_volatile(bytes.add(offset), 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

/// A value [`wipe`]d when dropped, for the `orchard` key types.
pub(crate) struct Wiped<T>(pub(crate) T);

impl<T> core::ops::Deref for Wiped<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T> Drop for Wiped<T> {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}
