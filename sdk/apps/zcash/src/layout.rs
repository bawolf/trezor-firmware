//! Screens shared by the handlers.

use alloc::string::String;

use trezor_app_sdk::ui::{
    self, Cancel, ConfirmSummary, ConfirmValue, Details, Menu, Progress, Property, ShowWarning,
};
use trezor_app_sdk::{Error, Result};

use crate::proto::common::button_request::ButtonRequestType;

/// ZIP-315 warning for a weak backup.
pub(crate) fn show_weak_backup_warning() -> Result<()> {
    show_warning(tr!("zcash__weak_backup_warning"), "zcash_weak_backup")
}

pub(crate) fn show_warning(content: &str, br_name: &str) -> Result<()> {
    ui::show_warning(ShowWarning::new(
        tr!("words__important"),
        content,
        tr!("words__continue_anyway"),
        Some(br_name),
        ButtonRequestType::Warning.into(),
        false, // allow_cancel
        true,  // danger
    ))
}

/// Runs `work` while `progress` is shown, keeping coreapp's 1 s limit between
/// app messages: `work` must call its argument often, and each call reports
/// at most every [`Progress::KEEP_ALIVE_MS`]. A failed report is returned
/// once `work` is done.
pub(crate) fn keeping_alive<T>(
    progress: &mut Progress,
    work: impl FnOnce(&mut dyn FnMut()) -> T,
) -> Result<T> {
    let mut failure = None;
    let value = work(&mut || {
        if failure.is_none() {
            failure = progress.keep_alive().err();
        }
    });
    match failure {
        Some(failure) => Err(failure),
        None => Ok(value),
    }
}

/// Runs a derivation behind an indeterminate progress screen (see
/// [`keeping_alive`]). A derivation failure becomes `error`.
pub(crate) fn with_progress<T, E>(
    derive: impl FnOnce(&mut dyn FnMut()) -> core::result::Result<T, E>,
    error: Error,
) -> Result<T> {
    let mut progress = Progress::show(None, None, true)?;
    keeping_alive(&mut progress, derive)?.map_err(|_| error)
}

/// The account a transaction spends from, as its screens name it.
pub(crate) struct Source<'a> {
    /// "ZEC #1".
    pub(crate) label: &'a str,
    /// "m/32'/133'/0'".
    pub(crate) path: &'a str,
}

/// "Recipient #1" for output 0.
fn recipient_title(number: usize) -> String {
    uformat!("{} #{}", tr!("words__recipient"), number + 1)
}

/// One output, as Core's `confirm_output` shows it: the address, then the
/// amount, each with a menu holding the source account and a cancel.
pub(crate) fn confirm_output(
    address: &str,
    amount: &str,
    number: usize,
    source: &Source<'_>,
) -> Result<()> {
    let title = recipient_title(number);
    let account_info = [
        Property::plain(tr!("words__wallet"), source.label),
        Property::plain(tr!("address_details__derivation_path"), source.path),
    ];
    let details = [Details::new(
        tr!("address_details__account_info"),
        &account_info,
        Some(tr!("address_details__account_info")),
        Some(tr!("send__send_from")),
        ButtonRequestType::Other.into(),
    )];
    let menu = Menu::new(&details, Some(Cancel::new(tr!("send__cancel_sign"))));
    let screen = |value: &str,
                  description: Option<&str>,
                  is_data: bool,
                  verb: Option<&str>,
                  chunkify: bool,
                  page_counter: bool| {
        let title = title.as_str();
        ui::interact_with_menu_flow(
            move |br_name| {
                ui::confirm_value(ConfirmValue::new(
                    tr!("words__send"),
                    value,
                    description,
                    br_name,
                    ButtonRequestType::ConfirmOutput.into(),
                    is_data,
                    verb,
                    Some(title),
                    false, // info
                    false, // hold
                    chunkify,
                    page_counter,
                    false, // cancel
                    true,  // external_menu
                    None,
                ))
            },
            &menu,
            Some("confirm_output"),
        )
    };
    ui::error_if_not_confirmed(ui::confirm_linear_flow(&[
        // Always chunked: an address the user must compare is grouped in
        // fours, as `ZcashGetAddress` shows it.
        &|| {
            screen(
                address,
                None,
                true,
                Some(tr!("buttons__continue")),
                true,
                true,
            )
        },
        &|| {
            screen(
                amount,
                Some(tr!("words__amount")),
                false,
                None,
                false,
                false,
            )
        },
    ])?)
}

/// A payment's memo on its own screen after the output: `text` verbatim, or
/// the hex `digest` of a memo the device does not show as text.
pub(crate) fn confirm_memo(value: &str, is_digest: bool, number: usize) -> Result<()> {
    let title = recipient_title(number);
    let description = if is_digest {
        tr!("zcash__memo_hash")
    } else {
        tr!("zcash__memo")
    };
    let menu = Menu::new(&[], Some(Cancel::new(tr!("buttons__cancel"))));
    ui::error_if_not_confirmed(ui::interact_with_menu_flow(
        |br_name| {
            ui::confirm_value(ConfirmValue::new(
                &title,
                value,
                Some(description),
                br_name,
                ButtonRequestType::ConfirmOutput.into(),
                is_digest, // is_data
                Some(tr!("buttons__continue")),
                None,
                false, // info
                false, // hold
                false, // chunkify
                false, // page_counter
                false, // cancel
                true,  // external_menu
                None,
            ))
        },
        &menu,
        Some("confirm_memo"),
    )?)
}

/// The consent screen, as Core's `confirm_total` shows it: what leaves the
/// wallet, the fee, the source account and the fee details.
pub(crate) fn confirm_total(
    total: &str,
    fee: &str,
    account_items: &[Property<'_>],
    fee_items: &[Property<'_>],
) -> Result<()> {
    ui::error_if_not_confirmed(ui::confirm_summary(ConfirmSummary::new(
        tr!("words__send"),
        Some(total),
        Some(tr!("send__total_amount")),
        fee,
        tr!("send__incl_transaction_fee"),
        None, // account_title
        Some(account_items),
        Some(tr!("confirm_total__title_fee")),
        Some(fee_items),
        false, // back_button
        Some("confirm_total"),
        ButtonRequestType::SignTx.into(),
    ))?)
}
