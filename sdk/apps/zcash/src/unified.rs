//! ZIP-316 encodings of what the app exports, with `zcash_address`.

use alloc::string::String;
use alloc::vec;

use trezor_app_sdk::{Error, Result};
use zcash_address::unified::{Address, Encoding, Fvk, Receiver, Ufvk};

use ironwood::receive::Network;

use crate::account::network_type;

/// The Unified Address holding only `receiver`, the Orchard receiver that
/// Orchard and Ironwood share.
pub(crate) fn address(network: Network, receiver: [u8; 43]) -> Result<String> {
    Address::try_from_items(vec![Receiver::Orchard(receiver)])
        .map(|address| address.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash address encoding failed"))
}

/// The Orchard-only Unified Full Viewing Key of `ak || nk || rivk`.
pub(crate) fn viewing_key(network: Network, full_viewing_key: [u8; 96]) -> Result<String> {
    Ufvk::try_from_items(vec![Fvk::Orchard(full_viewing_key)])
        .map(|key| key.encode(&network_type(network)))
        .map_err(|_| Error::DataError("Zcash viewing key encoding failed"))
}
