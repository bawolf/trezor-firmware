use ironwood::receive::derive_external_receiver_from_spending_key;
use trezor_app_sdk::ui::{self, ShowAddress};
use trezor_app_sdk::{Error, Result};

use crate::account::{
    account_from_request, account_keys, account_label, account_path, malformed,
    network_from_request, network_label,
};
use crate::layout::{show_weak_backup_warning, with_progress};
use crate::proto::common::button_request::ButtonRequestType;
use crate::proto::zcash::{ZcashAddress, ZcashGetAddress};
use crate::unified;

/// Shows the account's Unified Address at the requested diversifier index and
/// returns it once the user confirms it.
pub(crate) fn get_address(msg: ZcashGetAddress) -> Result<ZcashAddress> {
    let network = network_from_request(msg.network)?;
    let account = account_from_request(msg.account)?;
    let diversifier_index: [u8; 11] = msg
        .diversifier_index
        .as_deref()
        .and_then(|index| index.try_into().ok())
        .ok_or_else(malformed)?;

    let keys = account_keys(network, account)?;
    if keys.weak_backup {
        show_weak_backup_warning()?;
    }
    let receiver = with_progress(
        |progress| {
            derive_external_receiver_from_spending_key(
                &keys.spending_key,
                diversifier_index,
                progress,
            )
        },
        Error::DataError("Zcash receiver derivation failed"),
    )?;
    drop(keys);

    let address = unified::address(network, receiver)?;
    ui::error_if_not_confirmed(ui::show_address(ShowAddress::new(
        &address,
        &address,
        None,
        Some(network_label(network)),
        Some(&account_label(account)),
        Some(&account_path(network, account)),
        &[],
        msg.chunkify.unwrap_or(false), // chunkify
        ButtonRequestType::Address.into(),
        false, // case_sensitive
    ))?)?;
    Ok(ZcashAddress { address })
}
