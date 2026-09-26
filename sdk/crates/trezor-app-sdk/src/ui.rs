//! High-level UI API for interacting with the Trezor display.
//!
//! Every function here sends one screen request over the stable-ABI
//! [`UiV1`](crate::traits::ui::UiV1) vtable handed to this app, and returns
//! Core's response.

pub use crate::traits::ui::{
    ConfirmAction, ConfirmProperties, ConfirmSummary, ConfirmTrade, ConfirmValue,
    ConfirmValueIntro, ConfirmWithInfo, Property, RequestNumber, SelectMenu, ShowAddress,
    ShowDanger, ShowInfoWithCancel, ShowMismatch, ShowProperties, ShowPublicKey, ShowSuccess,
    ShowWarning, StrExt, TrezorUiResult,
};

use core::sync::atomic::{AtomicBool, Ordering};

use crate::app_runtime2::{get_ui_or_die, systick_ms};
use crate::traits::ui::{UiV1Dyn as _, opt_bytes};
use crate::{Error, IntoAppResult, Result};

pub type UiResult = Result<TrezorUiResult>;

/// Converts a [`TrezorUiResult`] into `Ok(())` if confirmed, or [`Error::Cancelled`] otherwise.
///
/// Useful as the outermost check after a UI flow.
pub fn error_if_not_confirmed(result: TrezorUiResult) -> Result<()> {
    if matches!(result, TrezorUiResult::Confirmed) {
        Ok(())
    } else {
        Err(Error::Cancelled)
    }
}

/// Runs a sequence of confirmation screens as one linear flow: `Confirmed`
/// advances to the next factory, `Back` steps back to the previous one (if
/// any), `Cancelled` ends the whole flow as cancelled.
pub fn confirm_linear_flow(confirm_factories: &[&dyn Fn() -> UiResult]) -> UiResult {
    let mut i = 0usize;

    while i < confirm_factories.len() {
        let res = (confirm_factories[i])()?;

        match res {
            TrezorUiResult::Confirmed => {
                i += 1;
            }
            TrezorUiResult::Back if i > 0 => {
                i -= 1;
            }
            TrezorUiResult::Cancelled => {
                return Ok(TrezorUiResult::Cancelled);
            }
            _ => {
                // TODO: proper error type
                return Err(Error::Cancelled);
            }
        }
    }

    Ok(TrezorUiResult::Confirmed)
}

/// Runs a main UI layout in a loop, opening a [`Menu`] when the user requests more info.
pub fn interact_with_menu_flow<'a>(
    main_layout: impl for<'b> Fn(Option<&'b str>) -> UiResult,
    menu: &Menu<'a>,
    br_name: Option<&str>,
) -> UiResult {
    let mut first_br = br_name;

    loop {
        let result = main_layout(first_br)?;
        first_br = None; // ButtonRequest should be sent once (for the main layout)

        if matches!(result, TrezorUiResult::Info) {
            let menu_res = menu.interact()?;
            if matches!(menu_res, TrezorUiResult::Cancelled) {
                return Ok(TrezorUiResult::Cancelled);
            }
            continue;
        }

        return Ok(result);
    }
}

/// Runs a main layout paired with an optional info layout, looping until confirmed or cancelled.
pub fn interact_with_info_flow(
    main_layout: impl for<'a> Fn(Option<&'a str>) -> UiResult,
    info_layout: impl for<'a> Fn(Option<&'a str>) -> UiResult,
    br_name: &str,
    repeat_button_request: Option<bool>,
    info_layout_can_confirm: Option<bool>,
) -> UiResult {
    let repeat_button_request = repeat_button_request.unwrap_or(false);
    let info_layout_can_confirm = info_layout_can_confirm.unwrap_or(false);

    let mut first_br = Some(br_name);
    let next_br = if repeat_button_request {
        Some(br_name)
    } else {
        None
    };

    loop {
        let main_res = main_layout(first_br)?;

        first_br = next_br;
        match main_res {
            TrezorUiResult::Confirmed => {
                return Ok(TrezorUiResult::Confirmed);
            }
            TrezorUiResult::Info => {
                let info_res = info_layout(next_br)?;

                if info_layout_can_confirm && matches!(info_res, TrezorUiResult::Confirmed) {
                    return Ok(TrezorUiResult::Confirmed);
                } else {
                    // Return to the same main step after info flow.
                    continue;
                }
            }
            TrezorUiResult::Cancelled => {
                return Ok(TrezorUiResult::Cancelled);
            }
            _ => {
                // TODO: proper error type
                return Err(Error::Cancelled);
            }
        }
    }
}

/// Shows a paginated text screen and returns whether the user wants to see more.
pub fn should_show_more<'a>(
    title: &'a str,
    para: &'a [StrExt<'a>],
    button_text: &'a str,
    br_name: Option<&'a str>,
    br_code: i32,
    verb: &'a str,
) -> Result<bool> {
    match confirm_with_info(ConfirmWithInfo::new(
        title,
        None,
        para,
        verb,
        Some(button_text),
        br_name,
        br_code,
    )) {
        Ok(TrezorUiResult::Confirmed) => Ok(false),
        Ok(TrezorUiResult::Info) => Ok(true),
        _ => Err(Error::Cancelled),
    }
}

/// One entry in a [`Menu`], rendered as a [`ShowProperties`] screen when selected.
#[derive(Copy, Clone)]
pub struct Details<'a> {
    name: &'a str,
    props: &'a [Property<'a>],
    title: Option<&'a str>,
    subtitle: Option<&'a str>,
    br_code: i32,
}

impl<'a> Details<'a> {
    pub fn new(
        name: &'a str,
        props: &'a [Property<'a>],
        title: Option<&'a str>,
        subtitle: Option<&'a str>,
        br_code: i32,
    ) -> Self {
        Self {
            name,
            title,
            props,
            subtitle,
            br_code,
        }
    }

    fn interact(&self) -> Result<()> {
        show_properties(ShowProperties::new(
            self.title.unwrap_or(self.name),
            self.props,
            self.subtitle,
            None,
            self.br_code,
        ))
    }
}

/// The cancel entry of a [`Menu`], rendered as a [`ConfirmAction`] screen when selected.
#[derive(Copy, Clone)]
pub struct Cancel<'a> {
    title: &'a str,
}

impl<'a> Cancel<'a> {
    pub fn new(title: &'a str) -> Self {
        Self { title }
    }

    fn interact(&self) -> UiResult {
        // TODO: impl br code
        confirm_action(ConfirmAction::new(
            self.title, "", None, None, false, None, false, None, 1, false,
        ))
    }
}

/// A menu of [`Details`] entries plus an optional [`Cancel`] entry, driven by
/// [`Menu::interact`].
#[derive(Copy, Clone)]
pub struct Menu<'a> {
    children: &'a [Details<'a>],
    cancel: Option<Cancel<'a>>,
}

impl<'a> Menu<'a> {
    const MAX_MENU_ITEMS: usize = 5;

    pub fn new(children: &'a [Details<'a>], cancel: Option<Cancel<'a>>) -> Self {
        Self { children, cancel }
    }

    pub fn interact(&self) -> UiResult {
        if self.children.is_empty() && self.cancel.is_none() {
            // TODO: maybe raise error instead
            return Ok(TrezorUiResult::Confirmed);
        }

        if self.children.len() > Self::MAX_MENU_ITEMS {
            // TODO: proper error type
            return Err(Error::Cancelled);
        }

        let mut items = ["".into(); Self::MAX_MENU_ITEMS];
        let mut i = 0usize;
        while i < self.children.len() {
            items[i] = self.children[i].name.into();
            i += 1;
        }

        loop {
            let choice = select_menu(
                SelectMenu::new(
                    &items[..self.children.len()],
                    self.cancel.as_ref().map(|c| c.title),
                    1,
                ),
                self.children.len(),
            )?;

            match choice {
                TrezorUiResult::Integer(idx) if (idx as usize) < self.children.len() => {
                    // Same behavior as Python: open details, ignore its result, return to menu.
                    self.children[idx as usize].interact()?;
                    continue;
                }
                TrezorUiResult::Confirmed => {
                    return Ok(TrezorUiResult::Confirmed);
                }
                TrezorUiResult::Cancelled => {
                    if let Some(cancel) = self.cancel.as_ref() {
                        let r = cancel.interact()?;
                        match r {
                            TrezorUiResult::Confirmed => return Ok(TrezorUiResult::Cancelled),
                            TrezorUiResult::Cancelled => continue,
                            // TODO: proper error type
                            _ => return Err(Error::Cancelled),
                        }
                    }
                }
                // TODO: proper error type
                _ => return Err(Error::Cancelled),
            }
        }
    }
}

pub fn confirm_value<'a>(confirm_value: ConfirmValue<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_value(confirm_value)
        .into_app_result()
}

pub fn confirm_value_intro<'a>(confirm_value_intro: ConfirmValueIntro<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_value_intro(confirm_value_intro)
        .into_app_result()
}

pub fn confirm_summary<'a>(confirm_summary: ConfirmSummary<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_summary(confirm_summary)
        .into_app_result()
}

pub fn confirm_action<'a>(confirm_action: ConfirmAction<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_action(confirm_action)
        .into_app_result()
}

fn select_menu<'a>(select_menu: SelectMenu<'a>, len: usize) -> UiResult {
    match get_ui_or_die().select_menu(select_menu).into_app_result() {
        Ok(TrezorUiResult::Integer(idx)) if (idx as usize) < len => {
            Ok(TrezorUiResult::Integer(idx))
        }
        Ok(TrezorUiResult::Confirmed) => Ok(TrezorUiResult::Confirmed),
        Ok(_) => Ok(TrezorUiResult::Cancelled),
        Err(e) => Err(e),
    }
}

pub fn confirm_properties<'a>(confirm_properties: ConfirmProperties<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_properties(confirm_properties)
        .into_app_result()
}

pub fn show_properties<'a>(show_properties: ShowProperties<'a>) -> Result<()> {
    get_ui_or_die()
        .show_properties(show_properties)
        .into_app_result()
}

pub fn show_warning<'a>(show_warning: ShowWarning<'a>) -> Result<()> {
    get_ui_or_die()
        .show_warning(show_warning)
        .into_app_result()
}

pub fn show_info_with_cancel<'a>(show_info_with_cancel: ShowInfoWithCancel<'a>) -> UiResult {
    get_ui_or_die()
        .show_info_with_cancel(show_info_with_cancel)
        .into_app_result()
}

pub fn show_mismatch<'a>(show_mismatch: ShowMismatch<'a>) -> UiResult {
    get_ui_or_die()
        .show_mismatch(show_mismatch)
        .into_app_result()
}

pub fn confirm_trade<'a>(confirm_trade: ConfirmTrade<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_trade(confirm_trade)
        .into_app_result()
}

pub fn show_danger<'a>(show_danger: ShowDanger<'a>) -> UiResult {
    get_ui_or_die().show_danger(show_danger).into_app_result()
}

pub fn show_success<'a>(show_success: ShowSuccess<'a>) -> Result<()> {
    get_ui_or_die()
        .show_success(show_success)
        .into_app_result()
}

pub fn request_number<'a>(request_number: RequestNumber<'a>) -> UiResult {
    get_ui_or_die()
        .request_number(request_number)
        .into_app_result()
}

pub fn show_public_key<'a>(show_public_key: ShowPublicKey<'a>) -> UiResult {
    get_ui_or_die()
        .show_public_key(show_public_key)
        .into_app_result()
}

pub fn confirm_with_info<'a>(confirm_with_info: ConfirmWithInfo<'a>) -> UiResult {
    get_ui_or_die()
        .confirm_with_info(confirm_with_info)
        .into_app_result()
}

pub fn show_address<'a>(show_address: ShowAddress<'a>) -> UiResult {
    get_ui_or_die()
        .show_address(show_address)
        .into_app_result()
}

/// Whether Core shows a progress screen for the current request. Core stops an
/// app that updates or ends progress it never initialized, so `update_progress`
/// and `end_progress` check this first.
static PROGRESS_SHOWN: AtomicBool = AtomicBool::new(false);

/// Starts a progress indicator, with an optional `description`/`title` and
/// whether it's `indeterminate` (no known end point) or `danger`ous (drawn
/// in a warning style). Replaces one already shown.
///
/// Prefer [`Progress`], which also ends the screen on every return path.
pub fn init_progress<'a>(
    description: Option<&'a str>,
    title: Option<&'a str>,
    indeterminate: bool,
    danger: bool,
) -> Result<()> {
    get_ui_or_die()
        .init_progress(opt_bytes(description), opt_bytes(title), indeterminate, danger)
        .into_app_result()?;
    PROGRESS_SHOWN.store(true, Ordering::Relaxed);
    Ok(())
}

/// Updates the current progress indicator's `description` and `value` (0-1000).
///
/// Fails with [`Error::DataError`], without contacting Core, if no progress
/// indicator is shown ([`init_progress`] was not called, or it ended).
pub fn update_progress<'a>(description: Option<&'a str>, value: u32) -> Result<()> {
    if !PROGRESS_SHOWN.load(Ordering::Relaxed) {
        return Err(Error::DataError("Progress not initialized"));
    }
    get_ui_or_die()
        .update_progress(opt_bytes(description), value)
        .into_app_result()
}

/// Ends the current progress indicator. Does nothing if none is shown.
pub fn end_progress() -> Result<()> {
    if !PROGRESS_SHOWN.swap(false, Ordering::Relaxed) {
        return Ok(());
    }
    get_ui_or_die().end_progress().into_app_result()
}

/// Forgets a progress screen that Core has already ended.
pub(crate) fn forget_progress() {
    PROGRESS_SHOWN.store(false, Ordering::Relaxed);
}

/// A progress screen for the duration of a long computation, ended when
/// dropped.
///
/// Call [`Progress::keep_alive`] from the computation's inner loop to stay
/// within Core's 1 s limit ("Long computations" in `sdk/doc/development.md`).
///
/// ## Example
///
/// ```no_run
/// use trezor_app_sdk::ui::Progress;
/// # fn step(_: u32) {}
/// let mut progress = Progress::show(None, None, true)?;
/// for i in 0..1000 {
///     step(i);
///     progress.keep_alive()?;
/// }
/// drop(progress); // or let it go out of scope
/// # Ok::<(), trezor_app_sdk::Error>(())
/// ```
#[must_use = "the progress screen ends when this is dropped"]
pub struct Progress {
    value: u32,
    reported_at_ms: u32,
}

impl Progress {
    /// Shortest interval between two reports sent by [`Progress::keep_alive`].
    pub const KEEP_ALIVE_MS: u32 = 100;

    /// Shows a progress screen at value 0; the arguments are those of
    /// [`init_progress`].
    pub fn show(
        description: Option<&str>,
        title: Option<&str>,
        indeterminate: bool,
    ) -> Result<Self> {
        init_progress(description, title, indeterminate, false)?;
        Ok(Self {
            value: 0,
            reported_at_ms: systick_ms(),
        })
    }

    /// Sets the progress bar to `value` (0..=1000).
    pub fn report(&mut self, value: u32) -> Result<()> {
        update_progress(None, value)?;
        self.value = value;
        self.reported_at_ms = systick_ms();
        Ok(())
    }

    /// Repeats the last reported value if [`Progress::KEEP_ALIVE_MS`] has
    /// passed since it, so an indeterminate screen can use it without ever
    /// calling [`Progress::report`].
    pub fn keep_alive(&mut self) -> Result<()> {
        let elapsed = systick_ms().wrapping_sub(self.reported_at_ms);
        if elapsed >= Self::KEEP_ALIVE_MS {
            self.report(self.value)?;
        }
        Ok(())
    }
}

impl Drop for Progress {
    fn drop(&mut self) {
        // `end_progress` has already cleared the local state; a failed End
        // leaves only Core's screen, which the next screen replaces.
        let _ = end_progress();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Without a progress screen, updates fail and ends do nothing, both
    /// without contacting Core.
    #[test]
    fn progress_without_a_screen() {
        assert!(matches!(
            update_progress(None, 500),
            Err(Error::DataError("Progress not initialized"))
        ));
        assert!(end_progress().is_ok());
    }
}
