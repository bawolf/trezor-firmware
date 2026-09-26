use trezor_app_sdk::Result;
use trezor_app_sdk::ui::{self, Progress, ShowWarning};

use crate::proto::common::button_request::ButtonRequestType;

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
