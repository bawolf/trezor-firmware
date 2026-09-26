use orchard::keys::FullViewingKey;
use trezor_app_sdk::Result;
use trezor_app_sdk::ui::{self, ConfirmAction};

use crate::helpers::account_label;
use crate::keys::{account_keys, spending_key};
use crate::layout::{show_warning, with_progress};
use crate::paths::{account_from_path, format_path};
use crate::proto::common::button_request::ButtonRequestType;
use crate::proto::zcash::{GetViewingKey, ViewingKey};
use crate::{uformat, unified};

/// Exports the account's Orchard-only unified full viewing key, and the seed
/// fingerprint if asked, after the user consents to each.
pub fn get_viewing_key(msg: GetViewingKey) -> Result<ViewingKey> {
    let (network, account) = account_from_path(&msg.address_n)?;

    // Hold: a viewing key reveals the account's history and cannot be revoked.
    let scope = uformat!(
        "{}\n{}",
        account_label(network, account).as_str(),
        format_path(network, account).as_str()
    );
    ui::error_if_not_confirmed(ui::confirm_action(ConfirmAction::new(
        tr!("zcash__title_export_viewing_key"),
        &scope,
        Some(tr!("zcash__viewing_key_warning")),
        None,
        true, // hold
        None,
        false, // cancel
        Some("zcash_export_viewing_key"),
        ButtonRequestType::PublicKey.into(),
        false, // external_menu
    ))?)?;
    // The fingerprint links all of the seed's accounts.
    if msg.include_seed_fingerprint() {
        show_warning(
            tr!("zcash__seed_fingerprint_warning"),
            "zcash_seed_fingerprint",
        )?;
    }

    let keys = account_keys(network, account)?;
    let fvk = with_progress(|progress| {
        let spending_key = spending_key(&keys, progress)?;
        progress();
        Ok(FullViewingKey::from(&*spending_key).to_bytes())
    })?;
    let seed_fingerprint = msg
        .include_seed_fingerprint()
        .then(|| keys.seed_fingerprint.to_vec());
    drop(keys);

    Ok(ViewingKey {
        key: unified::viewing_key(network, fvk)?,
        seed_fingerprint,
    })
}
