//! ZIP-316 unified addresses and viewing keys.

use alloc::string::String;
use alloc::vec;

use trezor_app_sdk::{Error, Result};
use zcash_address::unified::{Address, Encoding, Fvk, Receiver, Ufvk};
use zcash_protocol::consensus::NetworkType;
use zcash_signer::Network;

fn network_type(network: Network) -> NetworkType {
    match network {
        Network::Mainnet => NetworkType::Main,
        Network::Testnet => NetworkType::Test,
    }
}

/// The unified address with only the Orchard `receiver`, which the Orchard
/// and Ironwood pools share.
pub fn address(network: Network, receiver: [u8; 43]) -> Result<String> {
    Address::try_from_items(vec![Receiver::Orchard(receiver)])
        .map(|address| address.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash address encoding failed"))
}

/// The Orchard-only unified full viewing key of the raw `fvk`.
pub fn viewing_key(network: Network, fvk: [u8; 96]) -> Result<String> {
    Ufvk::try_from_items(vec![Fvk::Orchard(fvk)])
        .map(|key| key.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash viewing key encoding failed"))
}
