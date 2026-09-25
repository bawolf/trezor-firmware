//! Request validation, account labels and the account's key material.

use alloc::string::String;

use ironwood::receive::Network;
use trezor_app_sdk::{Error, Result};
use zcash_protocol::consensus::NetworkType;
use zeroize::Zeroize;

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

/// A ZIP-32 Orchard account's key material. The spending key is zeroized on
/// drop.
pub(crate) struct AccountKeys {
    pub(crate) spending_key: [u8; 32],
    pub(crate) seed_fingerprint: [u8; 32],
    /// ZIP 315: the backup is a 12- or 18-word mnemonic or a 128-bit SLIP-39
    /// secret, which wallets should warn about.
    pub(crate) weak_backup: bool,
}

impl Drop for AccountKeys {
    fn drop(&mut self) {
        self.spending_key.zeroize();
    }
}

/// The only place the app obtains key material: account `account` of
/// `m/32'/coin_type'/account'`. That needs a coreapp operation that is not
/// available yet, so only a `dev-test-seed` build has keys.
pub(crate) fn account_keys(network: Network, account: u32) -> Result<AccountKeys> {
    #[cfg(feature = "dev-test-seed")]
    {
        Ok(crate::dev_test_seed::account_keys(network, account))
    }
    #[cfg(not(feature = "dev-test-seed"))]
    {
        let _ = (network, account);
        Err(Error::DataError("Zcash account keys are not available"))
    }
}
