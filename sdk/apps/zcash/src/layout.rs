use alloc::string::String;

use trezor_app_sdk::Result;
use trezor_app_sdk::ui::{
    self, Cancel, ConfirmSummary, ConfirmValue, Details, Menu, Progress, Property, ShowWarning,
};

use crate::proto::common::button_request::ButtonRequestType;
use crate::uformat;

/// The ZIP-315 warning for a weak backup.
pub fn show_weak_backup_warning() -> Result<()> {
    show_warning(tr!("zcash__weak_backup_warning"), "zcash_weak_backup")
}

pub fn show_warning(content: &str, br_name: &str) -> Result<()> {
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

/// Runs `work`, which must call its argument often: each call reports
/// progress at most every [`Progress::KEEP_ALIVE_MS`], within Core's 1 s limit.
pub fn with_keep_alive<T>(
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

/// Runs `work` behind an indeterminate progress screen.
pub fn with_progress<T>(work: impl FnOnce(&mut dyn FnMut()) -> Result<T>) -> Result<T> {
    let mut progress = Progress::show(None, None, true)?;
    with_keep_alive(&mut progress, work)?
}

/// The account a transaction spends from, as its screens name it.
pub struct Source<'a> {
    /// "Zcash account #1 (Mainnet)".
    pub label: &'a str,
    /// "m/32'/133'/0'".
    pub path: &'a str,
}

/// "Recipient #1" for output 0.
fn recipient_title(number: usize) -> String {
    uformat!("{} #{}", tr!("words__recipient"), number + 1)
}

/// One output, as Core's `confirm_output` shows it: the address, then the
/// amount, each with a menu holding the source account and a cancel.
pub fn confirm_output(
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

    let steps: [&dyn Fn() -> ui::UiResult; 2] = [
        &|| {
            ui::interact_with_menu_flow(
                |br_name| {
                    ui::confirm_value(ConfirmValue::new(
                        tr!("words__send"),
                        address,
                        None,
                        br_name,
                        ButtonRequestType::ConfirmOutput.into(),
                        true, // is_data
                        Some(tr!("buttons__continue")),
                        Some(&title),
                        false, // info
                        false, // hold
                        true,  // chunkify: the user compares the address
                        true,  // page_counter
                        false, // cancel
                        true,  // external_menu
                        None,
                    ))
                },
                &menu,
                Some("confirm_output"),
            )
        },
        &|| {
            ui::interact_with_menu_flow(
                |br_name| {
                    ui::confirm_value(ConfirmValue::new(
                        tr!("words__send"),
                        amount,
                        Some(tr!("words__amount")),
                        br_name,
                        ButtonRequestType::ConfirmOutput.into(),
                        false, // is_data
                        None,
                        Some(&title),
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
                Some("confirm_output"),
            )
        },
    ];
    ui::error_if_not_confirmed(ui::confirm_linear_flow(&steps)?)
}

/// A payment's memo: its text, or the hex hash of a memo not shown as text.
pub fn confirm_memo(memo: &str, is_digest: bool, number: usize) -> Result<()> {
    let title = recipient_title(number);
    let description = if is_digest {
        tr!("zcash__memo_hash")
    } else {
        tr!("zcash__memo")
    };
    let menu = Menu::new(&[], Some(Cancel::new(tr!("send__cancel_sign"))));
    ui::error_if_not_confirmed(ui::interact_with_menu_flow(
        |br_name| {
            ui::confirm_value(ConfirmValue::new(
                &title,
                memo,
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

/// The consent screen, as Core's `confirm_total`.
pub fn confirm_total(
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
