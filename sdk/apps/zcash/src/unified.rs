//! ZIP-316 unified addresses and viewing keys, and Base58Check transparent
//! addresses.

use alloc::string::String;
use alloc::vec;

use trezor_app_sdk::{Error, Result};
use zcash_address::unified::{Address, Container, Encoding, Fvk, Receiver, Ufvk};
use zcash_address::{ToAddress, ZcashAddress};
use zcash_protocol::consensus::NetworkType;
use zcash_signer::{Network, TransparentKind};

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

/// The Orchard receiver of a unified `address` for `network`, if it has one.
pub fn orchard_receiver(network: Network, address: &str) -> Result<Option<[u8; 43]>> {
    let invalid = || Error::DataError("Invalid unified address");
    let (address_network, address) = Address::decode(address).map_err(|_| invalid())?;
    if address_network != network_type(network) {
        return Err(invalid());
    }
    Ok(address
        .items_as_parsed()
        .iter()
        .find_map(|receiver| match receiver {
            Receiver::Orchard(receiver) => Some(*receiver),
            _ => None,
        }))
}

/// The `t1`/`t3` (`tm`/`t2` on testnet) address paying `hash`.
pub fn transparent_address(network: Network, kind: TransparentKind, hash: [u8; 20]) -> String {
    let network = network_type(network);
    match kind {
        TransparentKind::P2pkh => ZcashAddress::from_transparent_p2pkh(network, hash),
        TransparentKind::P2sh => ZcashAddress::from_transparent_p2sh(network, hash),
    }
    .encode()
}

/// The Orchard-only unified full viewing key of the raw `fvk`.
pub fn viewing_key(network: Network, fvk: [u8; 96]) -> Result<String> {
    Ufvk::try_from_items(vec![Fvk::Orchard(fvk)])
        .map(|key| key.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash viewing key encoding failed"))
}
