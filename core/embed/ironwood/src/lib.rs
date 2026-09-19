#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::all)]
//! Bounded Ironwood PCZT approval core for the Safe 5 Zcash application.
//!
//! The host selects the network, account, and reference height. The device must
//! validate those values, derive the account from its own seed, display the
//! reference height as unverified, and bind all three to consent. Fee and
//! expiry limits are device-owned policy. This crate retains no seed or
//! separate full viewing-key object. The pending upstream PCZT still contains
//! viewing-key material; allocator-backed PCZT cleanup is a required later
//! integration boundary.

#[cfg(all(feature = "test", target_os = "none"))]
compile_error!("the `test` feature is host-only and must not enter bare-metal firmware");

extern crate alloc;

use alloc::vec::Vec;

/// Measurement-only per-operation micro-benchmarks (Sinsemilla vs Pallas
/// scalar mult). Reached only through the reserved diversifier-index bench path
/// in `get_address`; changes no signing behavior.
pub mod bench;
mod digest;
mod effects;
mod error;
mod prewarm;
mod session;
mod stream;
mod wire;

/// Session-start Pasta square-root table pre-warm (SHOULD-FIX #4); the device
/// signing handler calls it once after installing the region and before the
/// per-action loop.
pub use prewarm::prewarm;

/// Test-only access to bounded wire admission; not a production signing API.
#[cfg(feature = "test")]
pub mod testing {
    pub use crate::wire::preflight;
}

pub use error::{Error, ErrorCode, Result};
use orchard::Note;
use orchard::bundle::BundleVersion;
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendValidatingKey};
use orchard::note_encryption::IronwoodDomain;
use pczt::Pczt;
use pczt::roles::low_level_signer::{OrchardParseError, Signer as LowLevelSigner};
use pczt::roles::verifier::{OrchardError, Verifier};
use rand_core::{CryptoRng, RngCore};
pub use session::{Event, Session, SignatureRecord, Signatures};
/// Maximum number of admitted Ironwood actions.
pub use wire::MAX_ACTIONS;
/// Maximum uploaded or returned PCZT size. A transport enforces this before
/// allocation; [`Engine::begin`] rechecks the exact assembled bytes.
pub use wire::MAX_PCZT_BYTES;
use zcash_note_encryption::Domain;
use zcash_protocol::consensus::{
    BlockHeight, BranchId, MAIN_NETWORK, NetworkConstants, Parameters, TEST_NETWORK,
};
use zcash_protocol::constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID};
use zcash_protocol::value::MAX_MONEY;
use zeroize::{Zeroize, ZeroizeOnDrop};

const EMPTY_MEMO: [u8; 512] = {
    let mut memo = [0; 512];
    memo[0] = 0xf6;
    memo
};
const PADDING_MEMO: [u8; 512] = [0; 512];

fn ensure_malformed(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::malformed()) }
}

fn ensure_policy(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::policy()) }
}

fn ensure_state(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::state()) }
}

/// Production network selection.
///
/// Regtest is not selectable, but its coin type and branch are
/// indistinguishable from Testnet in the PCZT header. Test-only local-consensus
/// fixtures therefore remain admissible under Testnet; production callers must
/// derive from the selected network and account themselves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Network {
    Mainnet = 0,
    Testnet = 1,
}

impl Network {
    fn coin_type(self) -> u32 {
        match self {
            Self::Mainnet => MAIN_NETWORK.network_type().coin_type(),
            Self::Testnet => TEST_NETWORK.network_type().coin_type(),
        }
    }

    fn branch_for_height(self, height: u32) -> BranchId {
        let height = BlockHeight::from_u32(height);
        match self {
            Self::Mainnet => BranchId::for_height(&MAIN_NETWORK, height),
            Self::Testnet => BranchId::for_height(&TEST_NETWORK, height),
        }
    }
}

/// Host-selected ZIP-32 account, validated before device-side derivation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Account(u32);

impl Account {
    pub const MAX: u32 = 0x7fff_ffff;

    pub fn new(value: u32) -> Result<Self> {
        if value <= Self::MAX {
            Ok(Self(value))
        } else {
            Err(Error::policy())
        }
    }

    pub const fn value(self) -> u32 {
        self.0
    }
}

/// Untrusted request selections accepted only after device validation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RequestContext {
    network: Network,
    account: Account,
    host_reference_height: u32,
}

impl RequestContext {
    pub const fn new(network: Network, account: Account, host_reference_height: u32) -> Self {
        Self {
            network,
            account,
            host_reference_height,
        }
    }

    pub const fn network(self) -> Network {
        self.network
    }

    pub const fn account(self) -> Account {
        self.account
    }

    /// Host assertion only; it is neither a header nor authenticated chain
    /// state.
    pub const fn host_reference_height(self) -> u32 {
        self.host_reference_height
    }
}

/// Device-owned product limits, never supplied by the transport request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    maximum_fee: u64,
    expiry_window: u32,
}

impl Limits {
    pub fn new(maximum_fee: u64, expiry_window: u32) -> Result<Self> {
        if maximum_fee <= MAX_MONEY && expiry_window > 0 {
            Ok(Self {
                maximum_fee,
                expiry_window,
            })
        } else {
            Err(Error::policy())
        }
    }

    pub const fn maximum_fee(self) -> u64 {
        self.maximum_fee
    }

    pub const fn expiry_window(self) -> u32 {
        self.expiry_window
    }
}

/// Validated host selections combined with device-owned limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    request: RequestContext,
    limits: Limits,
}

impl Policy {
    pub fn new(request: RequestContext, limits: Limits) -> Result<Self> {
        request
            .host_reference_height
            .checked_add(limits.expiry_window)
            .ok_or(Error::policy())?;
        let branch = request
            .network
            .branch_for_height(request.host_reference_height);
        ensure_policy(branch == BranchId::Nu6_3)?;
        Ok(Self { request, limits })
    }

    pub const fn request(self) -> RequestContext {
        self.request
    }

    pub const fn limits(self) -> Limits {
        self.limits
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Payment,
    InternalChange,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewedOutput {
    pub action_index: usize,
    pub receiver: [u8; 43],
    pub value: u64,
    pub kind: OutputKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Projection {
    pub network: Network,
    pub account: Account,
    /// Unverified host assertion that the trusted UI must label accordingly.
    pub host_reference_height: u32,
    pub consensus_branch_id: u32,
    pub expiry_height: u32,
    pub blocks_until_expiry: u32,
    pub input_total: u64,
    pub payment_total: u64,
    pub change_total: u64,
    pub fee: u64,
    pub padding_outputs: usize,
    pub outputs: Vec<ReviewedOutput>,
}

#[derive(PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Token {
    session: [u8; 32],
    request: u64,
    context: [u8; 32],
}

impl Token {
    #[cfg(feature = "test")]
    pub fn context(&self) -> &[u8; 32] {
        &self.context
    }
}

pub struct Review {
    token: Token,
    projection: Projection,
    #[cfg(feature = "test")]
    sighash: [u8; 32],
}

impl core::fmt::Debug for Token {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Token { <redacted> }")
    }
}

impl core::fmt::Debug for Review {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Review { <redacted> }")
    }
}

impl Review {
    pub fn token(&self) -> &Token {
        &self.token
    }

    pub fn projection(&self) -> &Projection {
        &self.projection
    }

    #[cfg(feature = "test")]
    pub fn sighash(&self) -> &[u8; 32] {
        &self.sighash
    }
}

struct Pending {
    pczt: Pczt,
    signing_indices: Vec<usize>,
    approved: bool,
    header: wire::Header,
    sighash: [u8; 32],
    expected_ak: SpendValidatingKey,
}

struct PendingSlot {
    pending: Option<Pending>,
    // Deliberately duplicates the returned Token's binding. Review owns the
    // caller's token while the engine independently compares and clears this copy.
    session: [u8; 32],
    request: u64,
    context: [u8; 32],
}

impl PendingSlot {
    const fn empty() -> Self {
        Self {
            pending: None,
            session: [0; 32],
            request: 0,
            context: [0; 32],
        }
    }

    fn set(&mut self, pending: Pending, token: &Token) {
        debug_assert!(self.pending.is_none());
        self.session.copy_from_slice(&token.session);
        self.request = token.request;
        self.context.copy_from_slice(&token.context);
        self.pending = Some(pending);
    }

    fn matches(&self, token: &Token) -> bool {
        self.pending.is_some()
            && self.session == token.session
            && self.request == token.request
            && self.context == token.context
    }

    fn token(&self) -> Token {
        Token {
            session: self.session,
            request: self.request,
            context: self.context,
        }
    }

    fn clear_binding(&mut self) {
        self.session.zeroize();
        self.request.zeroize();
        self.context.zeroize();
    }

    fn clear(&mut self) -> bool {
        let had_pending = self.pending.is_some();
        self.pending = None;
        self.clear_binding();
        had_pending
    }

    fn take(&mut self) -> Option<(Pending, Token)> {
        let pending = self.pending.take()?;
        let token = self.token();
        self.clear_binding();
        Some((pending, token))
    }

    #[cfg(feature = "test")]
    fn binding_is_zero(&self) -> bool {
        self.session == [0; 32] && self.request == 0 && self.context == [0; 32]
    }
}

impl From<OrchardParseError> for Error {
    fn from(_: OrchardParseError) -> Self {
        Error::signing()
    }
}

/// Owned request state. Only a future trusted UI may call [`Engine::approve`].
pub struct Engine<R> {
    rng: R,
    policy: Policy,
    session: [u8; 32],
    counter: u64,
    slot: PendingSlot,
}

impl<R> Drop for Engine<R> {
    fn drop(&mut self) {
        self.slot.clear();
        self.session.zeroize();
        self.counter.zeroize();
    }
}

impl<R: RngCore + CryptoRng> Engine<R> {
    /// Creates an engine from device-owned limits, validated request context,
    /// and a trusted CSPRNG. Deterministic RNGs are test-only.
    ///
    /// Upstream signature generation uses the infallible
    /// [`RngCore::fill_bytes`] API. A device RNG must therefore fail-stop
    /// if entropy becomes unavailable after construction; the host-only
    /// panic test is only a proxy for that firmware behavior.
    pub fn with_rng(policy: Policy, mut rng: R) -> Result<Self> {
        let mut session = [0; 32];
        if rng.try_fill_bytes(&mut session).is_err() {
            session.zeroize();
            return Err(Error::entropy());
        }
        Ok(Self {
            rng,
            policy,
            session,
            counter: 0,
            slot: PendingSlot::empty(),
        })
    }

    /// Idempotently cancels the current request and clears crate-owned binding
    /// bytes.
    pub fn cancel(&mut self) {
        self.slot.clear();
    }

    /// Validates and retains the exact PCZT while borrowing the device-derived
    /// FVK.
    pub fn begin(&mut self, bytes: &[u8], fvk: &FullViewingKey) -> Result<Review> {
        self.slot.clear();
        self.counter = self.counter.checked_add(1).ok_or(Error::state())?;
        let Validated {
            pczt,
            projection,
            signing_indices,
            sighash,
            header,
            expected_ak,
        } = validate(bytes, &self.policy, fvk)?;

        let request = self.policy.request;
        let limits = self.policy.limits;
        let mut fvk_bytes = fvk.to_bytes();
        let mut h = blake2b_simd::Params::new()
            .hash_length(32)
            .personal(b"IWApprovalV1")
            .to_state();
        h.update(&self.session)
            .update(&self.counter.to_le_bytes())
            .update(&[request.network as u8])
            .update(&request.account.value().to_le_bytes())
            .update(&request.host_reference_height.to_le_bytes())
            .update(&limits.maximum_fee.to_le_bytes())
            .update(&limits.expiry_window.to_le_bytes())
            .update(&fvk_bytes)
            .update(&sighash)
            .update(&(bytes.len() as u64).to_le_bytes())
            .update(bytes);
        fvk_bytes.zeroize();
        let token = Token {
            session: self.session,
            request: self.counter,
            context: h
                .finalize()
                .as_bytes()
                .try_into()
                .expect("configured 32-byte hash"),
        };
        self.slot.set(
            Pending {
                pczt,
                signing_indices,
                approved: false,
                header,
                sighash,
                expected_ak,
            },
            &token,
        );
        Ok(Review {
            token,
            projection,
            #[cfg(feature = "test")]
            sighash,
        })
    }

    pub fn approve(&mut self, token: &Token) -> Result<()> {
        if self.slot.pending.is_none() {
            return Err(Error::state());
        }
        if !self.slot.matches(token)
            || self
                .slot
                .pending
                .as_ref()
                .is_some_and(|pending| pending.approved)
        {
            self.slot.clear();
            return Err(Error::state());
        }
        self.slot
            .pending
            .as_mut()
            .expect("checked pending")
            .approved = true;
        Ok(())
    }

    /// Consumes consent before every signing attempt, including all failures.
    pub fn sign(&mut self, token: &Token, ask: &SpendAuthorizingKey) -> Result<Pczt> {
        let Some((pending, mut retained_token)) = self.slot.take() else {
            return Err(Error::state());
        };
        ensure_state(pending.approved && retained_token == *token)?;
        if SpendValidatingKey::from(ask) != pending.expected_ak {
            return Err(Error::signing());
        }

        let signer = LowLevelSigner::new(pending.pczt).sign_ironwood_with(
            |_, bundle, modifiable| -> Result<()> {
                if *modifiable != 0 {
                    return Err(Error::internal());
                }
                if effects::sighash(bundle, &pending.header)? != pending.sighash {
                    return Err(Error::internal());
                }
                for index in &pending.signing_indices {
                    bundle
                        .actions_mut()
                        .get_mut(*index)
                        .ok_or(Error::internal())?
                        .sign(pending.sighash, ask, &mut self.rng)
                        .map_err(|_| Error::signing())?;
                }
                Ok(())
            },
        )?;
        retained_token.zeroize();
        Ok(signer.finish())
    }

    #[cfg(feature = "test")]
    pub fn test_request_binding_is_zero(&self) -> bool {
        self.slot.binding_is_zero()
    }

    #[cfg(feature = "test")]
    pub fn test_has_pending_request(&self) -> bool {
        self.slot.pending.is_some()
    }
}

fn add(total: u64, value: u64) -> Result<u64> {
    total
        .checked_add(value)
        .filter(|sum| *sum <= MAX_MONEY)
        .ok_or(Error::capacity())
}

struct Validated {
    pczt: Pczt,
    projection: Projection,
    signing_indices: Vec<usize>,
    sighash: [u8; 32],
    header: wire::Header,
    expected_ak: SpendValidatingKey,
}

fn validate(bytes: &[u8], policy: &Policy, fvk: &FullViewingKey) -> Result<Validated> {
    let header = wire::scan(bytes)?;
    ensure_policy(header.version == V6_TX_VERSION && header.group == V6_VERSION_GROUP_ID)?;
    let request = policy.request;
    let expected_branch = request
        .network
        .branch_for_height(request.host_reference_height);
    ensure_policy(expected_branch == BranchId::Nu6_3)?;
    ensure_policy(
        header.branch == u32::from(BranchId::Nu6_3)
            && header.coin_type == request.network.coin_type(),
    )?;
    ensure_policy(header.lock_time == 0)?;
    let maximum_expiry = request
        .host_reference_height
        .checked_add(policy.limits.expiry_window)
        .ok_or(Error::policy())?;
    ensure_policy(
        header.expiry > request.host_reference_height && header.expiry <= maximum_expiry,
    )?;

    let pczt = Pczt::parse(bytes).map_err(|_| Error::malformed())?;
    let mut sighash = None;
    let mut projection = Projection {
        network: request.network,
        account: request.account,
        host_reference_height: request.host_reference_height,
        consensus_branch_id: header.branch,
        expiry_height: header.expiry,
        blocks_until_expiry: header.expiry - request.host_reference_height,
        input_total: 0,
        payment_total: 0,
        change_total: 0,
        fee: 0,
        padding_outputs: 0,
        outputs: Vec::new(),
    };
    let mut signing_indices = Vec::new();
    let verifier = Verifier::new(pczt)
        .with_ironwood(|bundle| -> core::result::Result<(), OrchardError<Error>> {
            let digest = effects::sighash(bundle, &header).map_err(OrchardError::Custom)?;
            verify_bundle(
                bundle,
                fvk,
                policy,
                &digest,
                &mut projection,
                &mut signing_indices,
            )
            .map_err(OrchardError::Custom)?;
            sighash = Some(digest);
            Ok(())
        })
        .map_err(|error| match error {
            OrchardError::Custom(error) => error,
            _ => Error::malformed(),
        })?;
    Ok(Validated {
        // `Verifier` consumes and returns the same semantic PCZT, avoiding a
        // validation-time clone of the complete request. The low-level signer
        // still clones the Ironwood bundle internally; Boundary 2 must measure
        // and remove that extra live allocation before firmware integration.
        pczt: verifier.finish(),
        projection,
        signing_indices,
        sighash: sighash.ok_or(Error::internal())?,
        header,
        expected_ak: SpendValidatingKey::from(fvk.clone()),
    })
}

#[inline(never)]
fn verify_bundle(
    bundle: &orchard::pczt::Bundle,
    fvk: &FullViewingKey,
    policy: &Policy,
    sighash: &[u8; 32],
    projection: &mut Projection,
    signing_indices: &mut Vec<usize>,
) -> Result<()> {
    let version = BundleVersion::ironwood_v3();
    ensure_policy(
        bundle.flag_byte()
            == version
                .default_flags()
                .to_byte(version)
                .expect("valid default"),
    )?;
    // This is currently a no-op for the pinned Ironwood-v3 default flags
    // (cross-address enabled). Keep the check next to the exact flag gate so a
    // future reviewed flag change cannot silently bypass the restriction.
    bundle
        .verify_cross_address_restriction()
        .map_err(|_| Error::malformed())?;
    let mut output_total = 0;
    let mut nullifiers = Vec::new();
    // DEDUP LEVER 3: derive external+internal ivk (Commit^ivk) once per bundle.
    // Keyed on the DEVICE fvk; `verify_nullifier_with_classifier` only applies it
    // to spends validated under this same fvk (real spends), and falls back to
    // `fvk.scope_for_address` for dummy spends (MUST-FIX #3).
    let scope_classifier = fvk.scope_classifier();
    for (index, action) in bundle.actions().iter().enumerate() {
        let spend = action.spend();
        let output = action.output();
        let input_value = spend.value().ok_or(Error::malformed())?.inner();
        let output_value = output.value().ok_or(Error::malformed())?.inner();
        projection.input_total = add(projection.input_total, input_value)?;
        output_total = add(output_total, output_value)?;
        let nullifier = spend.nullifier().to_bytes();
        ensure_malformed(!nullifiers.contains(&nullifier))?;
        nullifiers.push(nullifier);
        action.verify_cv_net().map_err(|_| Error::malformed())?;
        spend
            .verify_nullifier_with_classifier(Some(fvk), Some(&scope_classifier))
            .map_err(|_| Error::malformed())?;
        spend.verify_rk(Some(fvk)).map_err(|_| Error::malformed())?;
        // MUST-FIX #4/#2: capture the validated output note (cmx already checked
        // equal to `output.cmx()` inside `verify_note_commitment`) and thread it
        // into `verify_encryption`, so recovery reuses this exact object instead
        // of recomputing a second `cmx`.
        let note = output
            .verify_note_commitment(spend)
            .map_err(|_| Error::malformed())?;
        if input_value == 0 {
            let signature = spend.spend_auth_sig().as_ref().ok_or(Error::malformed())?;
            spend
                .rk()
                .verify(sighash, signature)
                .map_err(|_| Error::malformed())?;
        } else {
            ensure_policy(spend.spend_auth_sig().is_none())?;
            signing_indices.push(index);
        }
        let recipient = output.recipient().ok_or(Error::malformed())?;
        let recipient_scope = scope_classifier.scope_for_address(&recipient);
        let outgoing_scope = if recipient_scope == Some(Scope::Internal) {
            Scope::Internal
        } else {
            Scope::External
        };
        verify_encryption(action, fvk, outgoing_scope, &note)?;
        if output_value == 0 {
            projection.padding_outputs += 1;
            continue;
        }
        let kind = if recipient_scope == Some(Scope::Internal) {
            projection.change_total = add(projection.change_total, output_value)?;
            OutputKind::InternalChange
        } else {
            projection.payment_total = add(projection.payment_total, output_value)?;
            OutputKind::Payment
        };
        projection.outputs.push(ReviewedOutput {
            action_index: index,
            receiver: recipient.to_raw_address_bytes(),
            value: output_value,
            kind,
        });
    }
    ensure_policy(!signing_indices.is_empty() && !projection.outputs.is_empty())?;
    projection.fee = projection
        .input_total
        .checked_sub(output_total)
        .ok_or(Error::malformed())?;
    ensure_malformed(i64::try_from(*bundle.value_sum()).ok() == Some(projection.fee as i64))?;
    ensure_policy(projection.fee <= policy.limits.maximum_fee)?;
    ensure_malformed(
        add(
            add(projection.payment_total, projection.change_total)?,
            projection.fee,
        )? == projection.input_total,
    )
}

fn verify_encryption(
    action: &orchard::pczt::Action,
    fvk: &FullViewingKey,
    outgoing_scope: Scope,
    // MUST-FIX #4/#2: the already-validated output note, whose commitment
    // `verify_note_commitment` checked equal to `action.output().cmx()`. Binding
    // is therefore self-contained: recovery is bound to the cmx-validated note,
    // not to a separately rebuilt one.
    note: &Note,
) -> Result<()> {
    let output = action.output();
    let domain = IronwoodDomain::for_pczt_action(action);
    // MUST-FIX #1: device-local recovery bound to `note` by field comparison.
    // `recover_output_bound_with_*` take the concrete `IronwoodDomain` and check
    // `domain.rho == note.rho()` and the domain version policy internally, so the
    // binding no longer relies on the caller having built `note` from this action.
    // Zero Sinsemilla.
    let memo = orchard::note_encryption::recover_output_bound_with_pkd_esk(
        &domain,
        IronwoodDomain::get_pk_d(note),
        IronwoodDomain::derive_esk(note).ok_or(Error::malformed())?,
        action,
        note,
    )
    .ok_or(Error::malformed())?;
    let empty_padding_memo = note.value().inner() == 0 && memo == PADDING_MEMO;
    ensure_policy(memo == EMPTY_MEMO || empty_padding_memo)?;
    let out_ciphertext = &output.encrypted_note().out_ciphertext;
    if let Some(ock) = output.ock() {
        ensure_malformed(
            orchard::note_encryption::recover_output_bound_with_ock(
                &domain, ock, action, out_ciphertext, note,
            )
            .is_some_and(|m| m == memo),
        )?;
    }
    if note.value().inner() > 0 {
        // Every value-bearing output must remain recoverable under the
        // receiver-appropriate device OVK. A host that constructed it with
        // `OvkPolicy::Discard` (`ovk=None`) is intentionally rejected.
        ensure_malformed(
            orchard::note_encryption::recover_output_bound_with_ovk(
                &domain,
                &fvk.to_ovk(outgoing_scope),
                action,
                action.cv_net(),
                out_ciphertext,
                note,
            )
            .is_some_and(|m| m == memo),
        )?;
    }
    // The standard Orchard builder creates zero-value padding with `ovk=None`,
    // so its `out_ciphertext` is random and cannot be recovered unless the PCZT
    // carries an OCK. We still validate note encryption above and OCK recovery
    // whenever present. The exact random ciphertext is included in the
    // transaction digest, bound into the approval token and retained PCZT, and
    // covered by the real spend signatures produced after approval. When the
    // paired spend is dummy, its pre-existing signature is additionally verified
    // above. Requiring OVK recovery here would reject standard padding.
    Ok(())
}
