#![no_std]
//! Experimental host reference for docs/APPROVAL_CONTRACT.md. Synthetic regtest only.
//! This is not a firmware integration or a security boundary within the host process.
extern crate alloc;
use alloc::vec::Vec;
mod effects;
pub mod wire;

use orchard::{
    Note, ValuePool,
    bundle::BundleVersion,
    keys::{FullViewingKey, Scope, SpendAuthorizingKey},
    note::{NoteVersion, Rho},
    note_encryption::IronwoodDomain,
};
use pczt::{
    Pczt,
    roles::{
        low_level_signer::{OrchardParseError, Signer as LowLevelSigner},
        verifier::{OrchardError, Verifier},
    },
};
#[cfg(feature = "std")]
use rand_core::OsRng;
use rand_core::{CryptoRng, RngCore};
use zcash_note_encryption::{
    Domain, try_output_recovery_with_ock, try_output_recovery_with_ovk,
    try_output_recovery_with_pkd_esk,
};
use zcash_protocol::{
    consensus::BranchId,
    constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID},
    memo::MemoBytes,
    value::MAX_MONEY,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Error(pub &'static str);
pub type Result<T> = core::result::Result<T, Error>;
fn ensure(ok: bool, message: &'static str) -> Result<()> {
    if ok { Ok(()) } else { Err(Error(message)) }
}

#[derive(Clone, Debug)]
pub struct Policy {
    reference_height: u32,
    maximum_fee: u64,
}
impl Policy {
    /// The height is trusted test context; the adapter does not authenticate chain state.
    pub fn regtest(reference_height: u32, maximum_fee: u64) -> Result<Self> {
        ensure(
            reference_height <= u32::MAX - 100 && maximum_fee <= MAX_MONEY,
            "invalid trusted policy",
        )?;
        Ok(Self {
            reference_height,
            maximum_fee,
        })
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
    pub network: &'static str,
    pub pool: &'static str,
    pub branch: u32,
    pub expiry: u32,
    pub total_input: u64,
    pub payments: u64,
    pub change: u64,
    pub fee: u64,
    pub padding_outputs: usize,
    pub outputs: Vec<ReviewedOutput>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    session: [u8; 32],
    request: u64,
    context: [u8; 32],
}
impl Token {
    pub fn context(&self) -> &[u8; 32] {
        &self.context
    }
    pub fn request(&self) -> u64 {
        self.request
    }
}
#[derive(Clone, Debug)]
pub struct Review {
    token: Token,
    projection: Projection,
    sighash: [u8; 32],
}
impl Review {
    pub fn token(&self) -> &Token {
        &self.token
    }
    pub fn projection(&self) -> &Projection {
        &self.projection
    }
    pub fn sighash(&self) -> &[u8; 32] {
        &self.sighash
    }
}
struct Pending {
    review: Review,
    pczt: Pczt,
    signing_indices: Vec<usize>,
    approved: bool,
    header: wire::Header,
}
/// One new Ironwood spend signature; profile 1 has no other value pool.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpendAuthSignature {
    action_index: usize,
    signature: [u8; 64],
}
impl SpendAuthSignature {
    pub fn value_pool(&self) -> ValuePool {
        ValuePool::Ironwood
    }
    pub fn action_index(&self) -> usize {
        self.action_index
    }
    pub fn signature(&self) -> &[u8; 64] {
        &self.signature
    }
}
impl From<OrchardParseError> for Error {
    fn from(_: OrchardParseError) -> Self {
        // Upstream covers both parsing and positional action-restoration checks.
        Error("preverified signing failed")
    }
}

pub struct Signed {
    pub token: Token,
    pub sighash: [u8; 32],
    pub signatures: Vec<SpendAuthSignature>,
    pub pczt: Pczt,
}

/// Owned state; approval must be invoked only by the future trusted UI, never by transport.
pub struct Engine<R> {
    rng: R,
    policy: Policy,
    fvk: FullViewingKey,
    session: [u8; 32],
    counter: u64,
    pending: Option<Pending>,
}
#[cfg(feature = "std")]
impl Engine<OsRng> {
    pub fn new(policy: Policy, fvk: FullViewingKey) -> Result<Self> {
        Self::with_rng(policy, fvk, OsRng)
    }
}
impl<R: RngCore + CryptoRng> Engine<R> {
    /// Owns the trusted RNG used for session IDs and signature randomness.
    /// Supply independently seeded CSPRNG state from trusted integration, never
    /// untrusted host-provided seeds or RNGs. `CryptoRng` does not enforce this provenance.
    /// Fixed or reused seeds are for deterministic synthetic tests only.
    pub fn with_rng(policy: Policy, fvk: FullViewingKey, rng: R) -> Result<Self> {
        Self::with_rng_and_counter(policy, fvk, rng, 0)
    }
    /// Uses the same trusted RNG contract as [`Self::with_rng`].
    /// `initial_counter` is already consumed/reserved by trusted integration,
    /// not chosen by the USB host. First begin attempts initial_counter + 1,
    /// including a validation-failing attempt. Recreated Engines must
    /// use disjoint counter ranges; this is not a substitute for secure entropy.
    /// The bridge reserves one counter per Engine and calls begin exactly once.
    pub fn with_rng_and_counter(
        policy: Policy,
        fvk: FullViewingKey,
        mut rng: R,
        initial_counter: u64,
    ) -> Result<Self> {
        let mut session = [0; 32];
        rng.try_fill_bytes(&mut session)
            .map_err(|_| Error("session entropy unavailable"))?;
        Ok(Self {
            policy,
            fvk,
            rng,
            session,
            counter: initial_counter,
            pending: None,
        })
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
    pub fn begin(&mut self, bytes: &[u8]) -> Result<Review> {
        self.cancel();
        self.counter = self
            .counter
            .checked_add(1)
            .ok_or(Error("session exhausted"))?;
        let Validated {
            pczt,
            projection,
            signing_indices,
            sighash,
            header,
        } = validate(bytes, &self.policy, &self.fvk)?;
        let mut h = blake2b_simd::Params::new()
            .hash_length(32)
            .personal(b"IWApprovalV1")
            .to_state();
        h.update(&self.session)
            .update(&self.counter.to_le_bytes())
            .update(&self.policy.reference_height.to_le_bytes())
            .update(&self.policy.maximum_fee.to_le_bytes())
            .update(&self.fvk.to_bytes())
            .update(&(bytes.len() as u64).to_le_bytes())
            .update(bytes);
        let token = Token {
            session: self.session,
            request: self.counter,
            context: h
                .finalize()
                .as_bytes()
                .try_into()
                .expect("configured 32-byte hash"),
        };
        let review = Review {
            token,
            projection,
            sighash,
        };
        self.pending = Some(Pending {
            review: review.clone(),
            pczt,
            signing_indices,
            approved: false,
            header,
        });
        Ok(review)
    }
    pub fn approve(&mut self, token: &Token) -> Result<()> {
        let Some(mut pending) = self.pending.take() else {
            return Err(Error("no pending review"));
        };
        ensure(
            !pending.approved && pending.review.token == *token,
            "approval token mismatch",
        )?;
        pending.approved = true;
        self.pending = Some(pending);
        Ok(())
    }
    /// Consumes consent even on failure. There is deliberately no replacement-PCZT parameter.
    ///
    /// Upstream signatures use the trusted RNG's infallible `RngCore::fill_bytes`.
    /// Entropy failure must stop signing (panic, abort, or reset), never substitute bytes.
    /// Such failures are not returned as this method's ordinary `Error`.
    pub fn sign(&mut self, token: &Token, ask: &SpendAuthorizingKey) -> Result<Signed> {
        let pending = self.pending.take().ok_or(Error("no approved request"))?;
        ensure(
            pending.approved && pending.review.token == *token,
            "signing token mismatch",
        )?;
        // Full Verifier checks covered this identical retained PCZT before consent.
        // This low-level path skips FVK validation and relies on those earlier checks.
        // No untrusted signing callback is exposed.
        let signer = LowLevelSigner::new(pending.pczt).sign_ironwood_with(
            |_, bundle, modifiable| -> Result<()> {
                ensure(*modifiable == 0, "signing flags changed")?;
                // The retained header supplies globals; this rechecks Ironwood effects.
                ensure(
                    effects::sighash(bundle, &pending.header)? == pending.review.sighash,
                    "signing digest changed",
                )?;
                for index in &pending.signing_indices {
                    bundle
                        .actions_mut()
                        .get_mut(*index)
                        .ok_or(Error("signing index changed"))?
                        .sign(pending.review.sighash, ask, &mut self.rng)
                        .map_err(|_| Error("signing failed"))?;
                }
                Ok(())
            },
        )?;
        let pczt = signer.finish();
        let signatures = pending
            .signing_indices
            .iter()
            .map(|index| {
                let signature = pczt
                    .ironwood()
                    .actions()
                    .get(*index)
                    .and_then(|action| *action.spend().spend_auth_sig())
                    .ok_or(Error("missing signature"))?;
                Ok(SpendAuthSignature {
                    action_index: *index,
                    signature,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Signed {
            token: pending.review.token,
            sighash: pending.review.sighash,
            signatures,
            pczt,
        })
    }
}

fn add(total: u64, value: u64) -> Result<u64> {
    total
        .checked_add(value)
        .filter(|x| *x <= MAX_MONEY)
        .ok_or(Error("accounting overflow"))
}

struct Validated {
    pczt: Pczt,
    projection: Projection,
    /// Action indices of verified positive inputs to sign, in action order.
    signing_indices: Vec<usize>,
    sighash: [u8; 32],
    header: wire::Header,
}
fn validate(bytes: &[u8], policy: &Policy, fvk: &FullViewingKey) -> Result<Validated> {
    let header = wire::scan(bytes)?;
    ensure(
        header.version == V6_TX_VERSION && header.group == V6_VERSION_GROUP_ID,
        "unsupported transaction format",
    )?;
    ensure(
        header.branch == u32::from(BranchId::Nu6_3) && header.coin_type == 1,
        "network or branch mismatch",
    )?;
    ensure(header.lock_time == 0, "nonzero lock time")?;
    ensure(
        header.expiry > policy.reference_height && header.expiry <= policy.reference_height + 100,
        "expiry outside trusted window",
    )?;
    // The v2 container is already required by wire::scan above; the v2-only
    // parse keeps the v1 decoder out of the image.
    // Unreachable after a successful scan, which is stricter; a single generic
    // message for every variant so an ironwood-bundle defect is never surfaced
    // under an upstream orchard-labeled field path.
    let pczt = pczt::v2::Pczt::parse(bytes).map_err(|_| Error("upstream PCZT parse failed"))?;
    let mut sighash = None;
    let mut projection = Projection {
        network: "regtest (synthetic)",
        pool: "Ironwood",
        branch: header.branch,
        expiry: header.expiry,
        total_input: 0,
        payments: 0,
        change: 0,
        fee: 0,
        padding_outputs: 0,
        outputs: Vec::new(),
    };
    let mut signing_indices = Vec::new();
    Verifier::new(pczt.clone())
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
        .map_err(|e| match e {
            OrchardError::Custom(e) => e,
            _ => Error("upstream bundle parse failed"),
        })?;
    // Retain the exact original parsed object, not a reserialized replacement from Verifier.
    Ok(Validated {
        pczt,
        projection,
        signing_indices,
        sighash: sighash.ok_or(Error("missing digest"))?,
        header,
    })
}

// Keep later verification temporaries out of callers during bundle parsing.
#[inline(never)]
fn verify_bundle(
    bundle: &orchard::pczt::Bundle,
    fvk: &FullViewingKey,
    policy: &Policy,
    sighash: &[u8; 32],
    p: &mut Projection,
    signing_indices: &mut Vec<usize>,
) -> Result<()> {
    let version = BundleVersion::ironwood_v3();
    ensure(
        bundle.flag_byte()
            == version
                .default_flags()
                .to_byte(version)
                .expect("valid default"),
        "unsupported bundle flags",
    )?;
    bundle
        .verify_cross_address_restriction()
        .map_err(|_| Error("cross-address restriction"))?;
    let mut output_total = 0;
    let mut nullifiers = Vec::new();
    for (index, action) in bundle.actions().iter().enumerate() {
        let spend = action.spend();
        let output = action.output();
        let input_value = spend.value().ok_or(Error("missing spend value"))?.inner();
        let output_value = output.value().ok_or(Error("missing output value"))?.inner();
        p.total_input = add(p.total_input, input_value)?;
        output_total = add(output_total, output_value)?;
        let nf = spend.nullifier().to_bytes();
        ensure(!nullifiers.contains(&nf), "duplicate nullifier")?;
        nullifiers.push(nf);
        action
            .verify_cv_net()
            .map_err(|_| Error("value commitment mismatch"))?;
        spend
            .verify_nullifier(Some(fvk))
            .map_err(|_| Error("nullifier or ownership mismatch"))?;
        spend
            .verify_rk(Some(fvk))
            .map_err(|_| Error("randomized key mismatch"))?;
        output
            .verify_note_commitment(spend)
            .map_err(|_| Error("note commitment mismatch"))?;
        if input_value == 0 {
            let sig = spend
                .spend_auth_sig()
                .as_ref()
                .ok_or(Error("missing padding signature"))?;
            spend
                .rk()
                .verify(sighash, sig)
                .map_err(|_| Error("invalid padding signature"))?;
        } else {
            ensure(
                spend.spend_auth_sig().is_none(),
                "real spend already signed",
            )?;
            signing_indices.push(index);
        }
        verify_encryption(action, fvk)?;
        if output_value == 0 {
            p.padding_outputs += 1;
            continue;
        }
        let recipient = output.recipient().ok_or(Error("missing recipient"))?;
        let kind = if fvk.scope_for_address(&recipient) == Some(Scope::Internal) {
            p.change = add(p.change, output_value)?;
            OutputKind::InternalChange
        } else {
            p.payments = add(p.payments, output_value)?;
            OutputKind::Payment
        };
        p.outputs.push(ReviewedOutput {
            action_index: index,
            receiver: recipient.to_raw_address_bytes(),
            value: output_value,
            kind,
        });
    }
    ensure(
        !signing_indices.is_empty() && !p.outputs.is_empty(),
        "no real spend or output",
    )?;
    p.fee = p
        .total_input
        .checked_sub(output_total)
        .ok_or(Error("negative fee"))?;
    ensure(
        i64::try_from(*bundle.value_sum()).ok() == Some(p.fee as i64),
        "bundle accounting mismatch",
    )?;
    ensure(p.fee <= policy.maximum_fee, "fee exceeds policy")?;
    ensure(
        add(add(p.payments, p.change)?, p.fee)? == p.total_input,
        "payment accounting mismatch",
    )
}

fn verify_encryption(action: &orchard::pczt::Action, fvk: &FullViewingKey) -> Result<()> {
    let output = action.output();
    let note = Note::from_parts(
        output.recipient().ok_or(Error("missing recipient"))?,
        output.value().ok_or(Error("missing value"))?,
        Rho::from_bytes(&action.spend().nullifier().to_bytes())
            .into_option()
            .ok_or(Error("invalid output rho"))?,
        output.rseed().ok_or(Error("missing rseed"))?,
        NoteVersion::V3,
    )
    .into_option()
    .ok_or(Error("invalid output note"))?;
    let domain = IronwoodDomain::for_pczt_action(action);
    let recovered = try_output_recovery_with_pkd_esk(
        &domain,
        IronwoodDomain::get_pk_d(&note),
        IronwoodDomain::derive_esk(&note).ok_or(Error("missing ephemeral secret"))?,
        action,
    )
    .ok_or(Error("note ciphertext mismatch"))?;
    ensure(
        recovered.0 == note && recovered.1 == note.recipient(),
        "recovered note mismatch",
    )?;
    // The upstream builder uses an empty text memo (all zero bytes) for padding.
    let empty_padding_memo = note.value().inner() == 0 && recovered.2 == [0; 512];
    ensure(
        recovered.2 == *MemoBytes::empty().as_array() || empty_padding_memo,
        "nonempty memo unsupported",
    )?;
    let out_ciphertext = &output.encrypted_note().out_ciphertext;
    if let Some(ock) = output.ock() {
        ensure(
            try_output_recovery_with_ock(&domain, ock, action, out_ciphertext)
                .is_some_and(|r| r == recovered),
            "outgoing key mismatch",
        )?;
    }
    if note.value().inner() > 0 {
        ensure(
            try_output_recovery_with_ovk(
                &domain,
                &fvk.to_ovk(Scope::External),
                action,
                action.cv_net(),
                out_ciphertext,
            )
            .is_some_and(|r| r == recovered),
            "outgoing recovery policy mismatch",
        )?;
    }
    Ok(())
}
