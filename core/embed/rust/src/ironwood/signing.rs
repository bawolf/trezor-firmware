//! Device state of one streamed Ironwood signing request
//! (docs/common/zcash-ironwood-signing.md §4; the handler it serves is §10).
//!
//! The MicroPython module `trezorironwood` drives exactly one request at a
//! time through `begin`, `feed`, `approve`, `sign` and `cancel`; the request
//! lives in a single `static` because the firmware runs one workflow at a
//! time and a second `begin` must clear the first as `Session::begin` does.
//! Python sees only projection data (receivers, amounts, totals), an opaque
//! "the review is pending" state, and signature records. Every key object is
//! derived here from the wallet seed the handler lends for one call: the FVK
//! is kept for the request because `Session::feed` borrows it on every call,
//! and the spend authorizing key exists only inside `sign`. Both are wiped by
//! their own `Drop` before the memory is reused.
//!
//! That covers the objects this module names, and not the temporaries
//! underneath them. `AccountKeys::derive` runs `zip32`'s `HardenedOnlyKey`,
//! which is not `Zeroize`, and `FullViewingKey::from(&sk)` computes the
//! spend-authorizing scalar as a temporary; both `begin` and `sign` therefore
//! leave key-derived bytes below the stack pointer. The handler clears them
//! the way every other seed-touching Trezor call does -- a
//! `utils.zero_unused_stack()` in a `finally` around each native call -- so
//! that guarantee lives in `apps/zcash/sign_pczt.py`, not here.

use alloc::boxed::Box;
use core::ptr;

use ironwood::{
    Account, Event, Hedged, Limits, Memo, Network, Policy, RequestContext, Result, Review, Session,
    TransparentKind,
};
use orchard::keys::{FullViewingKey, SpendAuthorizingKey, SpendingKey};
use rand_core::{CryptoRng, Error as RngError, RngCore};

/// Pool tag of a signature record (design §3).
pub const POOL_IRONWOOD: u8 = 0x03;
/// `pool ‖ action_index ‖ signature`.
pub const RECORD_LEN: usize = 1 + 1 + 64;

/// Seed lengths the receive module admits (`ironwood/src/receive/keys.rs`):
/// a restored SLIP-39 secret or a ZIP-32 seed. Kept identical so an account
/// that can show an address can also sign.
const RESTORED_SLIP39_SEED_BYTES: usize = 16;
const MIN_ZIP32_SEED_BYTES: usize = 32;
const MAX_SEED_BYTES: usize = 252;

/// Thin adapter over the firmware CSPRNG.
///
/// Never handed to a signing call directly: [`begin`] wraps it in
/// [`Hedged`], so a nonce is a function of the wallet secret as well as the
/// TRNG. See `ironwood/src/hedge.rs` for why.
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
    /// A transparent output to confirm. Its own step because the handler must
    /// warn about it and render a Base58Check address, not a unified one.
    TransparentOutput(TransparentOutput),
    /// The whole PCZT verified; the totals may be shown and approved.
    Review(Totals),
}

pub struct Output {
    pub action_index: usize,
    pub receiver: [u8; 43],
    pub value: u64,
    /// What to show for the memo (§11: text within the display budget
    /// verbatim, anything else as a hash), recovered from the signed
    /// ciphertext of this action.
    pub memo: Memo,
}

/// Base58Check version selector of a transparent output, as the handler sees
/// it: 0 is a public key hash (`t1…`/`tm…`), 1 a script hash (`t3…`/`t2…`).
/// Mirrored by `_T_P2PKH` / `_T_P2SH` in core/src/apps/zcash/sign_pczt.py;
/// the two lists must move together.
pub const T_P2PKH: u8 = 0;
pub const T_P2SH: u8 = 1;

pub struct TransparentOutput {
    pub index: usize,
    /// [`T_P2PKH`] or [`T_P2SH`].
    pub kind: u8,
    /// The 20 bytes the address encodes, solved from the signed script.
    pub hash: [u8; 20],
    pub value: u64,
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
    /// What the transparent outputs total: the public part of the payment.
    pub transparent_total: u64,
    pub padding_outputs: usize,
    pub payment_outputs: usize,
    pub transparent_outputs: usize,
    /// Total Orchard actions in the bundle (payments + change + padding). This
    /// is the quantity the MAX_ACTIONS = 32 cap bounds; the handler shows it so
    /// the user sees the true size of what they authorize, not just the visible
    /// payment count.
    pub action_count: usize,
}

/// One request. Dropped (and so wiped) on `cancel`, on any error, after
/// `sign`, and when a new request begins.
pub struct Signing {
    /// Identifies the request to the workflow that began it. See [`begin`].
    handle: u32,
    session: Session<Hedged<DeviceRng>>,
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

/// The device's ZIP-32 seed fingerprint ([`ironwood::seed_fingerprint`])
/// into `output`. A public identifier of the seed, not key material; `seed`
/// is borrowed for this call only.
///
/// For a 16-byte restored SLIP-39 secret this is a Trezor-only extension of
/// the ZIP-32 construction (see [`ironwood::seed_fingerprint`]): the
/// standard defines no fingerprint that short, so the device's value is the
/// authoritative one for such a wallet.
pub fn seed_fingerprint(seed: &[u8], output: &mut [u8; 32]) -> core::result::Result<(), Failure> {
    if !admissible_seed(seed) {
        return Err(Failure::State);
    }
    *output = ironwood::seed_fingerprint(seed).ok_or(Failure::State)?;
    Ok(())
}

/// The account's Orchard full viewing key as `ak ‖ nk ‖ rivk`, derived
/// through `orchard` -- the implementation that signs.
///
/// This exists so the viewing-key export can be checked against the
/// independent port in `ironwood::receive`, which is what derives addresses.
/// Two implementations of ZIP-32 and the Orchard key expansion ship in this
/// image; if they ever disagreed for some seed the wallet would receive to
/// addresses the signing path cannot spend from, and every real spend would
/// fail the session's `fvk` equality check. Funds stuck, not stolen, and
/// silently.
///
/// `orchard`'s `from_zip32_seed` goes through `zip32`'s `HardenedOnlyKey`,
/// which puts no lower bound on the seed, so this covers the 16-byte restored
/// SLIP-39 secret too -- the one length with no external oracle.
pub fn orchard_full_viewing_key(
    seed: &[u8],
    network: Network,
    account: u32,
) -> core::result::Result<[u8; 96], Failure> {
    let keys = AccountKeys::derive(seed, coin_type(network), account).ok_or(Failure::State)?;
    let mut fvk = keys.full_viewing_key();
    let bytes = fvk.to_bytes();
    wipe(&mut fvk);
    Ok(bytes)
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

/// Handle of the next request. Never 0, so a caller that lost its handle
/// cannot pass a default and be believed; boot-monotone, so a handle is never
/// reused within a boot except after 2^32 requests.
static mut NEXT_HANDLE: u32 = 1;

fn next_handle() -> u32 {
    // SAFETY: single-threaded, same argument as `active()`.
    unsafe {
        let handle = NEXT_HANDLE;
        NEXT_HANDLE = NEXT_HANDLE.wrapping_add(1);
        if NEXT_HANDLE == 0 {
            NEXT_HANDLE = 1;
        }
        handle
    }
}

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
    /// A value, or a running total, is outside the money range. Its own class
    /// because the `Capacity` message names the action cap, which is the wrong
    /// thing to tell a user about a transaction whose amounts do not add up.
    Amount,
    /// Signing or entropy failed.
    Signing,
}

impl From<ironwood::ErrorCode> for Failure {
    fn from(code: ironwood::ErrorCode) -> Self {
        use ironwood::ErrorCode::*;
        match code {
            Malformed => Self::Malformed,
            Capacity => Self::Capacity,
            Amount => Self::Amount,
            Policy => Self::Policy,
            State | Internal => Self::State,
            Entropy | Signing => Self::Signing,
        }
    }
}

/// Starts a request, replacing any earlier one. `seed` is borrowed for this
/// call only. Region installation is the caller's business: it must happen
/// before this allocates.
/// Starts a request and returns its handle.
///
/// The handle is what binds the native request to the Python workflow that
/// began it: `feed`, `approve` and `sign` refuse any other handle. The
/// firmware serialises workflows today (the wire layer closes the running one
/// before a new handler starts), so this changes nothing that happens now; it
/// is what keeps a second `ZcashSignPczt` from adopting a live session if that
/// ever weakens, which would otherwise show one workflow's transaction under
/// another's labels.
#[allow(clippy::too_many_arguments)]
pub fn begin(
    seed: &[u8],
    network: Network,
    account: u32,
    host_reference_height: u32,
    maximum_fee: u64,
    expiry_window: u32,
    declared_len: usize,
) -> core::result::Result<u32, Failure> {
    // Drop the old request first so its blocks return to the region before
    // the new one is carved.
    *active() = None;
    // Pre-warm the persistent Pasta square-root table into the
    // freshly installed region BEFORE any per-action scratch is allocated. Pasta
    // builds this ~29.8 KB table lazily behind a Rust `static` the GC never
    // scans; if it is first built mid-stream (interleaved with transient verify
    // scratch), the table is left at a high offset when that scratch frees and
    // splits the region — the fragmentation that faulted the 8-action run.
    // `prewarm` roots it at a low,
    // stable address for the whole session by decompressing one fixed public
    // point (one `Fp` square root), without the ~2-3 s of Sinsemilla hashing the
    // old `bench::warmup` wasted here and without rooting the region for process
    // lifetime. It touches no signing material and changes no signing behavior.
    ironwood::prewarm();
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
    let seed_fingerprint = ironwood::seed_fingerprint(seed).ok_or(Failure::State)?;
    // Hedge the signing nonces on the wallet secret, not on the TRNG alone.
    let mut session = Session::with_rng(policy, Hedged::from_seed(DeviceRng, seed))?;
    session.begin(declared_len, &fvk, &seed_fingerprint)?;
    let handle = next_handle();
    *active() = Some(Box::new(Signing {
        handle,
        session,
        fvk,
        coin_type,
        account,
        review: None,
    }));
    Ok(handle)
}

/// Whether the live request, if there is one, belongs to `handle`.
fn owns(handle: u32) -> bool {
    active()
        .as_deref()
        .is_some_and(|signing| signing.handle == handle)
}

fn with_active<T>(
    handle: u32,
    f: impl FnOnce(&mut Signing) -> Result<T>,
) -> core::result::Result<T, Failure> {
    let slot = active();
    let signing = slot.as_deref_mut().ok_or(Failure::State)?;
    // A caller that does not own this request is refused without touching it.
    // Destroying the live session here would turn a stray call from an already
    // dead workflow into a way to cancel the running one.
    if signing.handle != handle {
        return Err(Failure::State);
    }
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
pub fn feed(handle: u32, chunk: &[u8]) -> core::result::Result<(usize, Step), Failure> {
    with_active(handle, |signing| {
        if signing.review.is_some() {
            return Err(ironwood::ErrorCode::State);
        }
        let (consumed, event) = signing.session.feed(chunk, &signing.fvk)?;
        let step = match event {
            Event::None => Step::Continue,
            Event::ConfirmTransparentOutput(output) => Step::TransparentOutput(TransparentOutput {
                index: output.index,
                kind: match output.kind {
                    TransparentKind::P2pkh => T_P2PKH,
                    TransparentKind::P2sh => T_P2SH,
                },
                hash: output.hash,
                value: output.value,
            }),
            Event::ConfirmOutput(output) => {
                // The session offers only payments for confirmation; change
                // and padding are proved to be the device's own and are never
                // shown (`Session::action`). Checked here rather than in the
                // handler, where the equivalent `if is_change: raise` could
                // only ever be dead code, so a regression in the session
                // stops at the boundary instead of reaching a screen.
                if output.kind != ironwood::OutputKind::Payment {
                    return Err(ironwood::ErrorCode::State);
                }
                Step::Output(Output {
                    action_index: output.action_index,
                    receiver: output.receiver,
                    value: output.value,
                    memo: output.memo,
                })
            }
            Event::Review(review) => {
                let projection = review.projection();
                let totals = Totals {
                    expiry_height: projection.expiry_height,
                    blocks_until_expiry: projection.blocks_until_expiry,
                    input_total: projection.input_total,
                    payment_total: projection.payment_total,
                    change_total: projection.change_total,
                    transparent_total: projection.transparent_total,
                    fee: projection.fee,
                    padding_outputs: projection.padding_outputs,
                    payment_outputs: projection
                        .outputs
                        .iter()
                        .filter(|output| output.kind == ironwood::OutputKind::Payment)
                        .count(),
                    transparent_outputs: projection.transparent_outputs.len(),
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
pub fn approve(handle: u32) -> core::result::Result<(), Failure> {
    with_active(handle, |signing| {
        let review = signing.review.as_ref().ok_or(ironwood::ErrorCode::State)?;
        signing.session.approve(review.token())
    })
}

/// Signs every real spend and ends the request. Writes `n * RECORD_LEN`
/// bytes into `records` and returns `n`. The spend authorizing key is
/// derived from `seed` for this call only and wiped before returning.
pub fn sign(handle: u32, seed: &[u8], records: &mut [u8]) -> core::result::Result<usize, Failure> {
    // Not this workflow's request: refuse it without disturbing the one that
    // is live, since nothing was consumed.
    if !owns(handle) {
        return Err(Failure::State);
    }
    let result = with_active(handle, |signing| {
        let review = signing.review.as_ref().ok_or(ironwood::ErrorCode::State)?;
        let keys = AccountKeys::derive(seed, signing.coin_type, signing.account)
            .ok_or(ironwood::ErrorCode::State)?;
        let mut ask = keys.spend_authorizing_key();
        drop(keys);
        let signatures = signing.session.sign(review.token(), &ask);
        wipe(&mut ask);
        let signatures = signatures?;
        let mut written = 0;
        for record in signatures.records() {
            let slot = records
                .get_mut(written..written + RECORD_LEN)
                .ok_or(ironwood::ErrorCode::Capacity)?;
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

/// Ends the request. Idempotent.
///
/// `Some(handle)` ends it only if it is that handle's request: a workflow that
/// still holds its handle cannot tear down a request that is no longer its
/// own. `None` ends whatever is live, which is what blind teardown needs --
/// `sign_pczt`'s `finally` runs before `session_begin` on the paths that fail
/// early, and autolock unwinds it with a `GeneratorExit` from outside. Blind
/// cancel is fail-closed where blind adoption would not be.
pub fn cancel(handle: Option<u32>) {
    match handle {
        Some(handle) if !owns(handle) => (),
        _ => *active() = None,
    }
}
