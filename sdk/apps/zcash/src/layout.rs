//! Screens shared by the handlers.

use trezor_app_sdk::ui::{self, Progress, ShowWarning};
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

/// Runs a derivation behind a progress screen, keeping coreapp's 1 s limit
/// between app messages: `derive` calls its argument often. A derivation
/// failure becomes `error`.
pub(crate) fn with_progress<T, E>(
    derive: impl FnOnce(&mut dyn FnMut()) -> core::result::Result<T, E>,
    error: Error,
) -> Result<T> {
    let mut progress = Progress::show(None, None, true)?;
    let mut failure = None;
    let value = derive(&mut || {
        if failure.is_none() {
            failure = progress.keep_alive().err();
        }
    });
    if let Some(failure) = failure {
        return Err(failure);
    }
    value.map_err(|_| error)
}
