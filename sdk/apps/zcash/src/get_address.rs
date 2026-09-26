use orchard::keys::FullViewingKey;
use trezor_app_sdk::ui::{self, ShowAddress};
use trezor_app_sdk::{Error, Result};

use crate::helpers::{account_label, network_label};
use crate::keys::{Wiped, account_keys, spending_key};
use crate::layout::with_progress;
use crate::paths::{account_from_path, format_path};
use crate::proto::common::button_request::ButtonRequestType;
use crate::proto::zcash::{Address, GetAddress};
use crate::unified;

/// Shows the account's unified address at the requested diversifier index and
/// returns it once the user confirms it.
pub fn get_address(msg: GetAddress) -> Result<Address> {
    let (network, account) = account_from_path(&msg.address_n)?;
    let diversifier_index: [u8; 11] = msg
        .diversifier_index
        .as_slice()
        .try_into()
        .map_err(|_| Error::DataError("Invalid diversifier index"))?;

    let keys = account_keys(network, account)?;
    let receiver = with_progress(|progress| {
        let spending_key = spending_key(&keys, progress)?;
        progress();
        let fvk = Wiped::new(FullViewingKey::from(&*spending_key));
        progress();
        Ok(zcash_signer::keys::external_receiver(
            &fvk,
            diversifier_index,
            progress,
        ))
    })?;
    drop(keys);

    let address = unified::address(network, receiver)?;
    ui::error_if_not_confirmed(ui::show_address(ShowAddress::new(
        &address,
        &address,
        None,
        Some(network_label(network)),
        Some(&account_label(network, account)),
        Some(&format_path(network, account)),
        &[],
        msg.chunkify(),
        ButtonRequestType::Address.into(),
        false, // case_sensitive
    ))?)?;
    Ok(Address { address })
}
