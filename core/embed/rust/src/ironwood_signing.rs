//! Device state of one streamed Ironwood signing request
//! (docs/proposals/STREAMING_SIGNING_DESIGN.md §4, phase 4 handler).
//!
//! The MicroPython module `trezorironwood` drives exactly one request at a
//! time through `begin`, `feed`, `approve`, `sign` and `cancel`; the request
//! lives in a single `static` because the firmware runs one workflow at a
//! time and a second `begin` must clear the first as `Session::begin` does.
//! Python sees only projection data (receivers, amounts, totals), an opaque
//! "the review is pending" state, and signature records. Every key object is
//! derived here from the wallet seed the handler lends for one call: the FVK
//! is kept for the request because `Session::feed` borrows it on every call,
//! the spend authorizing key exists only inside `sign`, and both are wiped
//! before the memory is reused.

use alloc::boxed::Box;
use core::ptr;

use orchard::keys::{FullViewingKey, SpendAuthorizingKey, SpendingKey};
use rand_core::{CryptoRng, Error as RngError, RngCore};
use trezor_ironwood::{
    Account, Event, Limits, Network, Policy, RequestContext, Result, Review, Session,
};

/// Pool tag of a signature record (design §3).
pub const POOL_IRONWOOD: u8 = 0x03;
/// `pool ‖ action_index ‖ signature`.
pub const RECORD_LEN: usize = 1 + 1 + 64;

/// Seed lengths the receive crate admits (`ironwood-receive/src/keys.rs`):
/// a restored SLIP-39 secret or a ZIP-32 seed. Kept identical so an account
/// that can show an address can also sign.
const RESTORED_SLIP39_SEED_BYTES: usize = 16;
const MIN_ZIP32_SEED_BYTES: usize = 32;
const MAX_SEED_BYTES: usize = 252;

/// Thin adapter over the firmware CSPRNG for RedPallas signing nonces.
pub struct DeviceRng;

impl RngCore for DeviceRng {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        crate::trezorhal::random::bytes(destination);
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> core::result::Result<(), RngError> {
        self.fill_bytes(destination);
        Ok(())
    }
}

impl CryptoRng for DeviceRng {}

/// What the handler must show or do after feeding bytes.
pub enum Step {
    /// More bytes are needed.
    Continue,
    /// A verified payment output to confirm before streaming continues.
    Output(Output),
    /// The whole PCZT verified; the totals may be shown and approved.
    Review(Totals),
}

pub struct Output {
    pub action_index: usize,
    pub receiver: [u8; 43],
    pub value: u64,
    pub is_change: bool,
}

/// The digest-bound totals of the review projection. Network, account and
/// host height are omitted: the handler supplied them and shows its own copy.
pub struct Totals {
    pub expiry_height: u32,
    pub blocks_until_expiry: u32,
    pub input_total: u64,
    pub payment_total: u64,
    pub change_total: u64,
    pub fee: u64,
    pub padding_outputs: usize,
    pub payment_outputs: usize,
    /// Total Orchard actions in the bundle (payments + change + padding). This
    /// is the quantity the MAX_ACTIONS = 32 cap bounds; the handler shows it so
    /// the user sees the true size of what they authorize, not just the visible
    /// payment count.
    pub action_count: usize,
}

/// One request. Dropped (and so wiped) on `cancel`, on any error, after
/// `sign`, and when a new request begins.
pub struct Signing {
    session: Session<DeviceRng>,
    fvk: FullViewingKey,
    coin_type: u32,
    account: u32,
    review: Option<Review>,
}

impl Drop for Signing {
    fn drop(&mut self) {
        // `Session` and `Review` zeroize their own secrets on drop; the
        // orchard key type has no `Zeroize`, so it is overwritten here.
        wipe(&mut self.fvk);
    }
}

/// Overwrites a plain-data value with zeros in place, surviving optimisation.
/// Only for types without heap ownership or a `Drop` that reads them; every
/// orchard key type used here is such a value (field elements and points).
fn wipe<T>(value: &mut T) {
    let bytes = (value as *mut T).cast::<u8>();
    for offset in 0..core::mem::size_of::<T>() {
        // SAFETY: `offset` is within the object `value` points to.
        unsafe { ptr::write_volatile(bytes.add(offset), 0) };
    }
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

/// The seed lengths the device derives from: a restored SLIP-39 secret or a
/// ZIP-32 seed. One rule for signing and for the seed fingerprint, so a
/// wallet that can sign can also be identified.
fn admissible_seed(seed: &[u8]) -> bool {
    seed.len() == RESTORED_SLIP39_SEED_BYTES
        || (MIN_ZIP32_SEED_BYTES..=MAX_SEED_BYTES).contains(&seed.len())
}

/// The device's ZIP-32 seed fingerprint ([`trezor_ironwood::seed_fingerprint`])
/// into `output`. A public identifier of the seed, not key material; `seed`
/// is borrowed for this call only.
pub fn seed_fingerprint(seed: &[u8], output: &mut [u8; 32]) -> core::result::Result<(), Failure> {
    if !admissible_seed(seed) {
        return Err(Failure::State);
    }
    *output = trezor_ironwood::seed_fingerprint(seed).ok_or(Failure::State)?;
    Ok(())
}

/// Derived keys of `m/32'/coin_type'/account'`. The spending key is wiped on
/// drop; callers move the derived keys out and wipe those themselves.
struct AccountKeys {
    sk: SpendingKey,
}

impl AccountKeys {
    fn derive(seed: &[u8], coin_type: u32, account: u32) -> Option<Self> {
        if !admissible_seed(seed) {
            return None;
        }
        // The account type is `zip32::AccountId`, inferred from the parameter so
        // this crate needs no direct `zip32` dependency; `try_into` rejects the
        // hardened bit exactly as `Account::new` did.
        let sk = SpendingKey::from_zip32_seed(seed, coin_type, account.try_into().ok()?).ok()?;
        Some(Self { sk })
    }

    fn full_viewing_key(&self) -> FullViewingKey {
        FullViewingKey::from(&self.sk)
    }

    fn spend_authorizing_key(&self) -> SpendAuthorizingKey {
        SpendAuthorizingKey::from(&self.sk)
    }
}

impl Drop for AccountKeys {
    fn drop(&mut self) {
        wipe(&mut self.sk);
    }
}

fn coin_type(network: Network) -> u32 {
    match network {
        Network::Mainnet => 133,
        Network::Testnet => 1,
    }
}

static mut ACTIVE: Option<Box<Signing>> = None;

/// The one request slot.
fn active() -> &'static mut Option<Box<Signing>> {
    // SAFETY: the firmware is single-threaded and every caller is a MicroPython
    // function that runs to completion before another can touch the slot.
    unsafe { &mut *ptr::addr_of_mut!(ACTIVE) }
}

/// Failure classes the handler maps to wire failures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The bytes are not a well-formed PCZT.
    Malformed,
    /// The request or PCZT violates device policy.
    Policy,
    /// Calls were made out of order, or a derivation input is unusable.
    State,
    /// The PCZT exceeds a fixed device bound (too many actions, or too large).
    /// Distinct from `Malformed` so the user sees a comprehensible reason
    /// instead of a raw "malformed" for an otherwise well-formed but oversized
    /// transaction (Ironwood action-cap guardrail; MAX_ACTIONS = 32).
    Capacity,
    /// Signing or entropy failed.
    Signing,
}

impl From<trezor_ironwood::ErrorCode> for Failure {
    fn from(code: trezor_ironwood::ErrorCode) -> Self {
        use trezor_ironwood::ErrorCode::*;
        match code {
            Malformed => Self::Malformed,
            Capacity => Self::Capacity,
            Policy => Self::Policy,
            State | Internal => Self::State,
            Entropy | Signing => Self::Signing,
        }
    }
}

/// Starts a request, replacing any earlier one. `seed` is borrowed for this
/// call only. Region installation is the caller's business: it must happen
/// before this allocates.
#[allow(clippy::too_many_arguments)]
pub fn begin(
    seed: &[u8],
    network: Network,
    account: u32,
    host_reference_height: u32,
    maximum_fee: u64,
    expiry_window: u32,
    declared_len: usize,
) -> core::result::Result<(), Failure> {
    // Drop the old request first so its blocks return to the region before
    // the new one is carved.
    *active() = None;
    // SHOULD-FIX #4: pre-warm the persistent Pasta square-root table into the
    // freshly installed region BEFORE any per-action scratch is allocated. Pasta
    // builds this ~29.8 KB table lazily behind a Rust `static` the GC never
    // scans; if it is first built mid-stream (interleaved with transient verify
    // scratch), the table is left at a high offset when that scratch frees and
    // splits the region — the fragmentation that faulted the 8-action run (see
    // RAM-RETENTION-ANALYSIS.md §2, lever R2). `prewarm` roots it at a low,
    // stable address for the whole session by decompressing one fixed public
    // point (one `Fp` square root), without the ~2-3 s of Sinsemilla hashing the
    // old `bench::warmup` wasted here and without rooting the region for process
    // lifetime. It touches no signing material and changes no signing behavior.
    trezor_ironwood::prewarm();
    let policy = Policy::new(
        RequestContext::new(network, Account::new(account)?, host_reference_height),
        Limits::new(maximum_fee, expiry_window)?,
    )?;
    let coin_type = coin_type(network);
    let keys = AccountKeys::derive(seed, coin_type, account).ok_or(Failure::State)?;
    let fvk = keys.full_viewing_key();
    drop(keys);
    // The device's own seed fingerprint: with the consented account it is the
    // only `zip32_derivation` the session admits on the wire.
    let seed_fingerprint = trezor_ironwood::seed_fingerprint(seed).ok_or(Failure::State)?;
    let mut session = Session::with_rng(policy, DeviceRng)?;
    session.begin(declared_len, &fvk, &seed_fingerprint)?;
    *active() = Some(Box::new(Signing {
        session,
        fvk,
        coin_type,
        account,
        review: None,
    }));
    Ok(())
}

fn with_active<T>(f: impl FnOnce(&mut Signing) -> Result<T>) -> core::result::Result<T, Failure> {
    let slot = active();
    let signing = slot.as_deref_mut().ok_or(Failure::State)?;
    match f(signing) {
        Ok(value) => Ok(value),
        Err(code) => {
            // Any error ends the request: the session has already reset
            // itself; drop the rest so nothing waits for a cancel.
            *slot = None;
            Err(code.into())
        }
    }
}

/// Feeds bytes; returns how many were consumed and what to do next. Bytes
/// left over belong after the returned step and must be fed again.
pub fn feed(chunk: &[u8]) -> core::result::Result<(usize, Step), Failure> {
    with_active(|signing| {
        if signing.review.is_some() {
            return Err(trezor_ironwood::ErrorCode::State);
        }
        let (consumed, event) = signing.session.feed(chunk, &signing.fvk)?;
        let step = match event {
            Event::None => Step::Continue,
            Event::ConfirmOutput(output) => Step::Output(Output {
                action_index: output.action_index,
                receiver: output.receiver,
                value: output.value,
                is_change: output.kind == trezor_ironwood::OutputKind::InternalChange,
            }),
            Event::Review(review) => {
                let projection = review.projection();
                let totals = Totals {
                    expiry_height: projection.expiry_height,
                    blocks_until_expiry: projection.blocks_until_expiry,
                    input_total: projection.input_total,
                    payment_total: projection.payment_total,
                    change_total: projection.change_total,
                    fee: projection.fee,
                    padding_outputs: projection.padding_outputs,
                    payment_outputs: projection
                        .outputs
                        .iter()
                        .filter(|output| output.kind == trezor_ironwood::OutputKind::Payment)
                        .count(),
                    // One Orchard action per output; `outputs` holds every
                    // payment and change output, `padding_outputs` counts the
                    // rest. Their sum is the declared action count the wire
                    // parser capped at MAX_ACTIONS.
                    action_count: projection.outputs.len() + projection.padding_outputs,
                };
                signing.review = Some(review);
                Step::Review(totals)
            }
        };
        Ok((consumed, step))
    })
}

/// Records consent after the trusted totals screen.
pub fn approve() -> core::result::Result<(), Failure> {
    with_active(|signing| {
        let review = signing
            .review
            .as_ref()
            .ok_or(trezor_ironwood::ErrorCode::State)?;
        signing.session.approve(review.token())
    })
}

/// Signs every real spend and ends the request. Writes `n * RECORD_LEN`
/// bytes into `records` and returns `n`. The spend authorizing key is
/// derived from `seed` for this call only and wiped before returning.
pub fn sign(seed: &[u8], records: &mut [u8]) -> core::result::Result<usize, Failure> {
    let result = with_active(|signing| {
        let review = signing
            .review
            .as_ref()
            .ok_or(trezor_ironwood::ErrorCode::State)?;
        let keys = AccountKeys::derive(seed, signing.coin_type, signing.account)
            .ok_or(trezor_ironwood::ErrorCode::State)?;
        let mut ask = keys.spend_authorizing_key();
        drop(keys);
        let signatures = signing.session.sign(review.token(), &ask);
        wipe(&mut ask);
        let signatures = signatures?;
        let mut written = 0;
        for record in signatures.records() {
            let slot = records
                .get_mut(written..written + RECORD_LEN)
                .ok_or(trezor_ironwood::ErrorCode::Capacity)?;
            slot[0] = POOL_IRONWOOD;
            slot[1] = record.action_index;
            slot[2..].copy_from_slice(&record.signature);
            written += RECORD_LEN;
        }
        Ok(written / RECORD_LEN)
    });
    // Consent is consumed by `Session::sign` whatever happened; the request
    // is over either way.
    *active() = None;
    result
}

/// Ends the request, if any. Idempotent.
pub fn cancel() {
    *active() = None;
}
