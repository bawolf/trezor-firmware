use ironwood::receive::derive_full_viewing_key_from_spending_key;
use trezor_app_sdk::ui::{self, ConfirmAction};
use trezor_app_sdk::{Error, Result};
use zeroize::Zeroizing;

use crate::account::{
    account_from_request, account_keys, account_label, account_path, network_from_request,
    network_label,
};
use crate::layout::{show_warning, show_weak_backup_warning, with_progress};
use crate::proto::common::button_request::ButtonRequestType;
use crate::proto::zcash::{ZcashGetViewingKey, ZcashViewingKey};
use crate::unified;

/// Exports the account's Orchard-only UFVK, and the seed fingerprint if asked,
/// after the user consents to each.
pub(crate) fn get_viewing_key(msg: ZcashGetViewingKey) -> Result<ZcashViewingKey> {
    let network = network_from_request(msg.network)?;
    let account = account_from_request(msg.account)?;
    let include_seed_fingerprint = msg.include_seed_fingerprint.unwrap_or(false);

    // Hold: an exported viewing key reveals the account's whole history and
    // cannot be revoked.
    let scope = uformat!(
        "Zcash {}\n{}\n{}",
        network_label(network),
        account_label(account).as_str(),
        account_path(network, account).as_str()
    );
    ui::error_if_not_confirmed(ui::confirm_action(ConfirmAction::new(
        tr!("zcash__export_viewing_key"),
        &scope,
        Some(tr!("zcash__viewing_key_warning")),
        None,
        true, // hold
        None,
        false, // cancel
        Some("zcash_export_viewing_key"),
        ButtonRequestType::SignTx.into(),
        false, // external_menu
    ))?)?;

    // The seed fingerprint is the same for every account and both networks,
    // so it links everything this seed signs. The screen above promises
    // account scope, so the fingerprint needs its own opt-in and warning.
    if include_seed_fingerprint {
        show_warning(
            tr!("zcash__seed_fingerprint_warning"),
            "zcash_seed_fingerprint",
        )?;
    }

    let keys = account_keys(network, account)?;
    if keys.weak_backup {
        show_weak_backup_warning()?;
    }
    let mut full_viewing_key = Zeroizing::new([0u8; 96]);
    with_progress(
        |progress| {
            derive_full_viewing_key_from_spending_key(
                &keys.spending_key,
                &mut full_viewing_key,
                progress,
            )
        },
        Error::DataError("Zcash viewing key derivation failed"),
    )?;
    let seed_fingerprint = include_seed_fingerprint.then(|| keys.seed_fingerprint.to_vec());
    drop(keys);

    Ok(ZcashViewingKey {
        key: unified::viewing_key(network, *full_viewing_key)?,
        seed_fingerprint,
    })
}
