//! Encodings of the addresses and keys the app shows or exports: ZIP-316
//! unified addresses and viewing keys, and Base58Check transparent addresses,
//! with `zcash_address`.

use alloc::string::String;
use alloc::vec;

use ironwood::TransparentKind;
use ironwood::receive::Network;
use trezor_app_sdk::{Error, Result};
use zcash_address::unified::{Address, Container, Encoding, Fvk, Receiver, Ufvk};
use zcash_address::{ToAddress, ZcashAddress};

use crate::account::network_type;

/// The Unified Address holding only `receiver`, the Orchard receiver that
/// Orchard and Ironwood share.
pub(crate) fn address(network: Network, receiver: [u8; 43]) -> Result<String> {
    Address::try_from_items(vec![Receiver::Orchard(receiver)])
        .map(|address| address.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash address encoding failed"))
}

/// The Orchard receiver of `address`, a unified address for `network`, if it
/// has one.
pub(crate) fn orchard_receiver(network: Network, address: &str) -> Result<Option<[u8; 43]>> {
    let invalid = || Error::DataError("Invalid unified address.");
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

/// The `t1…`/`t3…` (`tm…`/`t2…` on testnet) address of a transparent output
/// paying `hash`.
pub(crate) fn transparent_address(
    network: Network,
    kind: TransparentKind,
    hash: [u8; 20],
) -> String {
    let network = network_type(network);
    match kind {
        TransparentKind::P2pkh => ZcashAddress::from_transparent_p2pkh(network, hash),
        TransparentKind::P2sh => ZcashAddress::from_transparent_p2sh(network, hash),
    }
    .encode()
}

/// The Orchard-only Unified Full Viewing Key of `ak || nk || rivk`.
pub(crate) fn viewing_key(network: Network, full_viewing_key: [u8; 96]) -> Result<String> {
    Ufvk::try_from_items(vec![Fvk::Orchard(full_viewing_key)])
        .map(|key| key.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash viewing key encoding failed"))
}
