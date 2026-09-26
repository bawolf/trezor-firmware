//! Streaming PCZT verification: [`Session`] reads the PCZT in chunks, keeps
//! at most one action in memory, and signs only after [`Session::approve`].
//!
//! The FVK is never retained: `begin` keeps its encoding and every `feed`
//! borrows the key again.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
use alloc::string::String;
use alloc::vec::Vec;

use blake2b_simd::{Params, State};
use orchard::Anchor;
use orchard::bundle::{BundleVersion, Flags};
use orchard::keys::{
    FullViewingKey, Scope, ScopeClassifier, SpendAuthorizingKey, SpendValidatingKey,
};
use orchard::note::NoteVersion;
use orchard::pczt::{Action, Output, Spend};
use orchard::primitives::redpallas::{Signature, SpendAuth, VerificationKey};
use rand_core::{CryptoRng, RngCore};
use subtle::ConstantTimeEq;
use zcash_protocol::consensus::BranchId;
use zcash_protocol::constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::error::{Error, Result, ensure};
use crate::limits::{MAX_ACTIONS, MAX_TRANSPARENT_OUTPUTS};
use crate::scanner::{self, Item, Scanner};
use crate::sighash::{ActionEffects, Sighash, finalize};
use crate::{
    OutputKind, Policy, Review, ReviewedOutput, Summary, Token, TransparentOutput, add_amount, memo,
};

/// Personalization of the hash of the PCZT bytes, which the token commits to.
const STREAM_PERSONAL: &[u8; 15] = b"TrezorZcashPczt";

/// Personalization of the token's digest.
const TOKEN_PERSONAL: &[u8; 16] = b"TrezorZcashToken";

/// `Spend::parse` requires a nullifier; the signing path never reads it.
const UNUSED_NULLIFIER: [u8; 32] = [0; 32];

/// What the session yields after consuming bytes.
#[derive(Debug)]
pub enum Event {
    /// More bytes are needed.
    NeedMore,
    /// A verified payment, to show before the next feed. Not consent: a later
    /// error still refuses the whole PCZT. `user_address` is the wallet's
    /// recipient string, unverified.
    ConfirmOutput {
        output: ReviewedOutput,
        user_address: Option<String>,
    },
    /// A transparent output, which is public and has a Base58Check address.
    ConfirmTransparentOutput(TransparentOutput),
    /// The whole PCZT verified; the token awaits the user's approval.
    Review(Review),
}

/// What signing needs of one action.
#[derive(Clone, Copy, Zeroize)]
enum Record {
    /// A real spend, signed after approval.
    Real { rk: [u8; 32], alpha: [u8; 32] },
    /// A dummy spend: its `rk` is checked against the host's own FVK, and the
    /// host's signature over the sighash authorizes it.
    Dummy { rk: [u8; 32], signature: [u8; 64] },
}

#[derive(Default, Zeroize, ZeroizeOnDrop)]
struct Records([Option<Record>; MAX_ACTIONS]);

impl Records {
    fn iter(&self) -> impl Iterator<Item = (usize, &Record)> {
        self.0
            .iter()
            .enumerate()
            .filter_map(|(index, record)| record.as_ref().map(|record| (index, record)))
    }

    /// Copies the records out and zeroizes them where they were, which a move
    /// would not.
    fn take_zeroizing(&mut self) -> Self {
        let taken = Self(self.0);
        self.zeroize();
        taken
    }
}

/// The spend authorization signature of one action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignatureRecord {
    pub action_index: u8,
    pub signature: [u8; 64],
}

/// Every real spend's signature, released only once all of them exist.
#[derive(Debug)]
pub struct Signatures {
    records: [SignatureRecord; MAX_ACTIONS],
    len: usize,
}

impl Signatures {
    const fn empty() -> Self {
        Self {
            records: [SignatureRecord {
                action_index: 0,
                signature: [0; 64],
            }; MAX_ACTIONS],
            len: 0,
        }
    }

    fn push(&mut self, record: SignatureRecord) -> Result<()> {
        let slot = self.records.get_mut(self.len).ok_or(Error::Internal)?;
        *slot = record;
        self.len += 1;
        Ok(())
    }

    /// Records in ascending action order.
    pub fn records(&self) -> &[SignatureRecord] {
        &self.records[..self.len]
    }
}

/// A reviewed request awaiting approval.
struct Pending {
    approved: bool,
    sighash: [u8; 32],
    expected_ak: SpendValidatingKey,
    records: Records,
}

/// The pending request and a copy of its token, cleared in place so that
/// both zeroize where they are stored.
struct Slot {
    /// Boxed so the `MAX_ACTIONS`-sized records stay off the stack.
    pending: Option<Box<Pending>>,
    token: Option<Token>,
}

impl Slot {
    const fn empty() -> Self {
        Self {
            pending: None,
            token: None,
        }
    }

    fn set(&mut self, pending: Box<Pending>, token: &Token) {
        debug_assert!(self.pending.is_none());
        self.token = Some(Token {
            session_id: token.session_id,
            counter: token.counter,
            digest: token.digest,
        });
        self.pending = Some(pending);
    }

    fn matches(&self, token: &Token) -> bool {
        self.pending.is_some() && self.token.as_ref() == Some(token)
    }

    fn clear(&mut self) {
        self.pending = None;
        self.token = None;
    }
}

/// The account's cached ivks, wiped on drop (`ScopeClassifier` has no
/// `Zeroize`).
struct IvkCache(ScopeClassifier);

impl Drop for IvkCache {
    fn drop(&mut self) {
        self.0.wipe();
    }
}

/// Per-PCZT state after the header.
struct Body {
    sighash: Sighash,
    summary: Summary,
    output_total: u64,
    /// The action count, which follows the transparent bundle; zero until then.
    count: usize,
    seen: usize,
    /// Transparent outputs already shown. They are shown once `count` is known
    /// to fit the action limit they share.
    confirmed: usize,
    nullifiers: [[u8; 32]; MAX_ACTIONS],
    records: Records,
    /// The account's ivks, built on the first action and reused.
    ivk_cache: Option<IvkCache>,
}

struct Stream {
    scanner: Scanner,
    /// Hash of LE64(declared length) || every byte || LE64(action count).
    bytes: State,
    declared_len: usize,
    /// The FVK given to `begin`, which every `feed` and every real spend must
    /// carry.
    fvk: Zeroizing<[u8; 96]>,
    expected_ak: SpendValidatingKey,
    /// The derivation any `zip32_derivation` on the wire must name.
    account: AccountDerivation,
    /// Present once the header has been verified. Boxed so the
    /// `MAX_ACTIONS`-sized state stays off the stack.
    body: Option<Box<Body>>,
}

/// The seed fingerprint and account path the device signs under.
struct AccountDerivation {
    seed_fingerprint: [u8; 32],
    path: [u32; 3],
}

impl AccountDerivation {
    /// A claim for another seed or account is refused without a screen. This
    /// answers only about values the host must hold to build the PCZT.
    fn check(&self, claim: &scanner::Zip32Derivation<'_>) -> Result<()> {
        ensure(
            bool::from(
                claim
                    .seed_fingerprint
                    .as_slice()
                    .ct_eq(self.seed_fingerprint.as_slice()),
            ) && claim.path == self.path,
            Error::Policy,
        )
    }
}

#[derive(Clone, Copy)]
struct Trailer {
    flags: u8,
    value_sum: u64,
    anchor: Option<[u8; 32]>,
}

/// One PCZT at a time. Call [`Session::approve`] only after the user
/// confirmed the review.
pub struct Session<R> {
    rng: R,
    policy: Policy,
    id: [u8; 32],
    counter: u64,
    /// Boxed so the `MAX_ACTIONS`-sized state stays off the stack.
    stream: Option<Box<Stream>>,
    slot: Slot,
}

impl<R> Drop for Session<R> {
    fn drop(&mut self) {
        self.reset();
        self.id.zeroize();
        self.counter.zeroize();
    }
}

impl<R> Session<R> {
    fn reset(&mut self) {
        self.stream = None;
        self.slot.clear();
    }
}

impl<R: RngCore + CryptoRng> Session<R> {
    /// Fails with `Entropy` if `rng` cannot produce the session id.
    pub fn new(policy: Policy, mut rng: R) -> Result<Self> {
        let mut id = [0; 32];
        if rng.try_fill_bytes(&mut id).is_err() {
            id.zeroize();
            return Err(Error::Entropy);
        }
        Ok(Self {
            rng,
            policy,
            id,
            counter: 0,
            stream: None,
            slot: Slot::empty(),
        })
    }

    /// Starts a request of exactly `declared_len` bytes for the account's FVK
    /// and seed fingerprint. Clears any pending request.
    pub fn begin(
        &mut self,
        declared_len: usize,
        fvk: &FullViewingKey,
        seed_fingerprint: &[u8; 32],
    ) -> Result<()> {
        self.reset();
        self.counter = self.counter.checked_add(1).ok_or(Error::State)?;
        let scanner = Scanner::new(declared_len)?;
        let mut bytes = Params::new()
            .hash_length(32)
            .personal(STREAM_PERSONAL)
            .to_state();
        bytes.update(&(declared_len as u64).to_le_bytes());
        self.stream = Some(Box::new(Stream {
            scanner,
            bytes,
            declared_len,
            fvk: Zeroizing::new(fvk.to_bytes()),
            expected_ak: SpendValidatingKey::from(fvk.clone()),
            account: AccountDerivation {
                seed_fingerprint: *seed_fingerprint,
                path: self.policy.account_path(),
            },
            body: None,
        }));
        Ok(())
    }

    /// Consumes bytes and returns how many were taken and the event they
    /// completed; the rest must be fed again after the event. A transparent
    /// output held back until the action count is known is returned with
    /// zero bytes consumed. `fvk` must be the key given to `begin`.
    /// `progress` is called between the expensive steps of each action. Any
    /// error resets the session.
    pub fn feed(
        &mut self,
        chunk: &[u8],
        fvk: &FullViewingKey,
        progress: &mut dyn FnMut(),
    ) -> Result<(usize, Event)> {
        let result = self.advance(chunk, fvk, progress);
        if result.is_err() {
            self.reset();
        }
        result
    }

    fn advance(
        &mut self,
        chunk: &[u8],
        fvk: &FullViewingKey,
        progress: &mut dyn FnMut(),
    ) -> Result<(usize, Event)> {
        let bound = self.stream.as_deref().ok_or(Error::State)?;
        let offered = Zeroizing::new(fvk.to_bytes());
        ensure(
            bool::from(offered.as_slice().ct_eq(bound.fvk.as_slice())),
            Error::State,
        )?;
        let mut consumed = 0;
        // Held-back transparent outputs come first, in bundle order.
        if let Some(row) = self.next_transparent()? {
            return Ok((0, Event::ConfirmTransparentOutput(row)));
        }
        while consumed < chunk.len() {
            let stream = self.stream.as_deref_mut().ok_or(Error::State)?;
            let (used, item) = stream.scanner.feed(&chunk[consumed..])?;
            stream.bytes.update(&chunk[consumed..consumed + used]);
            consumed += used;
            let trailer = match item {
                None => continue,
                Some(Item::Header(header)) => {
                    if stream.body.is_some() {
                        return Err(Error::Internal);
                    }
                    stream.body = Some(Body::new(header, &self.policy)?);
                    continue;
                }
                Some(Item::TransparentOutput(output)) => {
                    let body = stream.body.as_deref_mut().ok_or(Error::Internal)?;
                    // Shown only once the action count is known to fit.
                    body.transparent_output(&output)?;
                    continue;
                }
                Some(Item::Shielded(shielded)) => {
                    let body = stream.body.as_deref_mut().ok_or(Error::Internal)?;
                    body.shielded(&shielded)?;
                    if let Some(row) = body.next_transparent() {
                        return Ok((consumed, Event::ConfirmTransparentOutput(row)));
                    }
                    continue;
                }
                Some(Item::Action(action)) => {
                    let body = stream.body.as_deref_mut().ok_or(Error::Internal)?;
                    match body.action(&action, fvk, &stream.fvk, &stream.account, progress)? {
                        Some(output) => {
                            let user_address = action.output.user_address.map(String::from);
                            return Ok((
                                consumed,
                                Event::ConfirmOutput {
                                    output,
                                    user_address,
                                },
                            ));
                        }
                        None => continue,
                    }
                }
                Some(Item::Trailer(trailer)) => Trailer {
                    flags: trailer.flags,
                    value_sum: trailer.value_sum,
                    anchor: trailer.anchor.copied(),
                },
            };
            let review = self.review(trailer, progress)?;
            return Ok((consumed, Event::Review(review)));
        }
        Ok((consumed, Event::NeedMore))
    }

    fn next_transparent(&mut self) -> Result<Option<TransparentOutput>> {
        let Some(stream) = self.stream.as_deref_mut() else {
            return Err(Error::State);
        };
        Ok(stream
            .body
            .as_deref_mut()
            .and_then(|body| body.next_transparent()))
    }

    /// The trailer, the dummy spends' signatures and the totals, then the
    /// token. The stream is dropped where it lives on every exit, which wipes
    /// its buffer and FVK encoding.
    fn review(&mut self, trailer: Trailer, progress: &mut dyn FnMut()) -> Result<Review> {
        let result = self.review_stream(trailer, progress);
        self.stream = None;
        result
    }

    // Out of line, so that its locals are not on the stack while an action
    // is verified.
    #[inline(never)]
    fn review_stream(&mut self, trailer: Trailer, progress: &mut dyn FnMut()) -> Result<Review> {
        let stream = self.stream.as_deref_mut().ok_or(Error::Internal)?;
        if !stream.scanner.is_finished() {
            return Err(Error::Internal);
        }
        let body = stream.body.as_deref_mut().ok_or(Error::Internal)?;
        if body.seen != body.count {
            return Err(Error::Internal);
        }
        // Every transparent output must have been shown.
        if body.confirmed != body.summary.transparent_outputs.len() {
            return Err(Error::Internal);
        }
        let records = body.records.take_zeroizing();
        // Wiped here as well as on drop, before the box is released.
        if let Some(cache) = body.ivk_cache.as_mut() {
            cache.0.wipe();
        }
        // Only the summary leaves the box; the rest is freed in place.
        let Body {
            sighash,
            mut summary,
            output_total,
            count,
            seen: _,
            confirmed: _,
            nullifiers: _,
            records: _,
            ivk_cache: _,
        } = *stream.body.take().ok_or(Error::Internal)?;

        // The flags must parse and be the defaults.
        let version = BundleVersion::ironwood_v3();
        Flags::from_byte(trailer.flags, version).ok_or(Error::Malformed)?;
        let default_flags = version
            .default_flags()
            .to_byte(version)
            .ok_or(Error::Internal)?;
        ensure(trailer.flags == default_flags, Error::Policy)?;
        // A present anchor must be a valid Pallas base.
        if let Some(anchor) = trailer.anchor {
            Anchor::from_bytes(anchor)
                .into_option()
                .ok_or(Error::Malformed)?;
        }
        let value_balance = i64::try_from(trailer.value_sum).map_err(|_| Error::Malformed)?;
        let sighash = sighash.finish(trailer.flags, value_balance)?;

        for (_, record) in records.iter() {
            if let Record::Dummy { rk, signature } = record {
                VerificationKey::<SpendAuth>::try_from(*rk)
                    .map_err(|_| Error::Internal)?
                    .verify(&sighash, &Signature::from(*signature))
                    .map_err(|_| Error::Malformed)?;
                progress();
            }
        }

        // A real spend, and at least one output to review.
        let real = records
            .iter()
            .any(|(_, record)| matches!(record, Record::Real { .. }));
        let reviewed_an_output =
            !summary.outputs.is_empty() || !summary.transparent_outputs.is_empty();
        ensure(real && reviewed_an_output, Error::Policy)?;
        // Transparent outputs leave through the value sum: payments, not fee.
        let transparent_total = summary.transparent_total;
        summary.fee = summary
            .input_total
            .checked_sub(output_total)
            .and_then(|left| left.checked_sub(transparent_total))
            .ok_or(Error::Malformed)?;
        ensure(
            i64::try_from(trailer.value_sum).ok()
                == Some(add_amount(summary.fee, transparent_total)? as i64),
            Error::Malformed,
        )?;
        ensure(summary.fee <= self.policy.max_fee, Error::Policy)?;
        ensure(
            add_amount(
                add_amount(
                    add_amount(summary.payment_total, summary.change_total)?,
                    transparent_total,
                )?,
                summary.fee,
            )? == summary.input_total,
            Error::Malformed,
        )?;

        stream.bytes.update(&(count as u64).to_le_bytes());
        let stream_digest = finalize(&stream.bytes);
        let policy = &self.policy;
        let mut h = Params::new()
            .hash_length(32)
            .personal(TOKEN_PERSONAL)
            .to_state();
        h.update(&self.id)
            .update(&self.counter.to_le_bytes())
            .update(&[policy.network as u8])
            .update(&policy.account.to_le_bytes())
            .update(&policy.reference_height.to_le_bytes())
            .update(&policy.max_fee.to_le_bytes())
            .update(&policy.expiry_window.to_le_bytes())
            .update(&*stream.fvk)
            .update(&sighash)
            .update(&(stream.declared_len as u64).to_le_bytes())
            .update(&stream_digest);
        let token = Token {
            session_id: self.id,
            counter: self.counter,
            digest: finalize(&h),
        };
        self.slot.set(
            Box::new(Pending {
                approved: false,
                sighash,
                expected_ak: stream.expected_ak.clone(),
                records,
            }),
            &token,
        );
        Ok(Review { token, summary })
    }

    /// Approves the reviewed request. Approving while bytes are still
    /// streaming discards the stream.
    pub fn approve(&mut self, token: &Token) -> Result<()> {
        self.stream = None;
        let matches = self.slot.matches(token);
        match self.slot.pending.as_deref_mut() {
            Some(pending) if matches && !pending.approved => {
                pending.approved = true;
                Ok(())
            }
            Some(_) => {
                self.slot.clear();
                Err(Error::State)
            }
            None => Err(Error::State),
        }
    }

    /// Signs every real spend of the approved request, calling `progress`
    /// after each signature. Consumes the approval whether or not it
    /// succeeds.
    pub fn sign(
        &mut self,
        token: &Token,
        ask: &SpendAuthorizingKey,
        progress: &mut dyn FnMut(),
    ) -> Result<Signatures> {
        self.stream = None;
        let result = match (self.slot.pending.as_deref_mut(), &self.slot.token) {
            (Some(pending), Some(retained)) => {
                // Consumed before signing, so that it is gone even if signing
                // never returns.
                let approved = core::mem::replace(&mut pending.approved, false);
                ensure(approved && *retained == *token, Error::State)
                    .and_then(|()| Self::sign_records(pending, ask, &mut self.rng, progress))
            }
            _ => Err(Error::State),
        };
        self.slot.clear();
        result
    }

    fn sign_records(
        pending: &Pending,
        ask: &SpendAuthorizingKey,
        rng: &mut R,
        progress: &mut dyn FnMut(),
    ) -> Result<Signatures> {
        if SpendValidatingKey::from(ask) != pending.expected_ak {
            return Err(Error::Signing);
        }
        let mut signatures = Signatures::empty();
        for (index, record) in pending.records.iter() {
            let Record::Real { rk, alpha } = record else {
                continue;
            };
            // Parsed again because `pallas::Scalar` is not nameable here.
            let spend = Spend::parse(
                UNUSED_NULLIFIER,
                *rk,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(*alpha),
                None,
                None,
                NoteVersion::V3,
                BTreeMap::new(),
            )
            .map_err(|_| Error::Internal)?;
            let alpha = spend.alpha().as_ref().ok_or(Error::Internal)?;
            let rsk = ask.randomize(alpha);
            if VerificationKey::from(&rsk) != *spend.rk() {
                return Err(Error::Signing);
            }
            let signature = rsk.sign(&mut *rng, &pending.sighash);
            signatures.push(SignatureRecord {
                action_index: u8::try_from(index).map_err(|_| Error::Internal)?,
                signature: <[u8; 64]>::from(&signature),
            })?;
            progress();
        }
        Ok(signatures)
    }
}

impl Body {
    /// The header checks, then the sighash. Out of line so the boxed body is
    /// built off the caller's stack.
    #[inline(never)]
    fn new(header: scanner::Header, policy: &Policy) -> Result<Box<Self>> {
        // The scanner has capped this, so the subtraction below cannot
        // underflow.
        if header.transparent_outputs > MAX_TRANSPARENT_OUTPUTS {
            return Err(Error::Internal);
        }
        ensure(
            header.version == V6_TX_VERSION && header.group == V6_VERSION_GROUP_ID,
            Error::Policy,
        )?;
        let network = policy.network;
        ensure(
            network.branch_for_height(policy.reference_height) == BranchId::Nu6_3,
            Error::Policy,
        )?;
        ensure(
            header.branch == u32::from(BranchId::Nu6_3) && header.coin_type == network.coin_type(),
            Error::Policy,
        )?;
        ensure(header.lock_time == 0, Error::Policy)?;
        let maximum_expiry = policy
            .reference_height
            .checked_add(policy.expiry_window)
            .ok_or(Error::Policy)?;
        ensure(
            header.expiry > policy.reference_height && header.expiry <= maximum_expiry,
            Error::Policy,
        )?;

        let sighash = Sighash::new(&header)?;
        let summary = Summary {
            expiry_height: header.expiry,
            input_total: 0,
            payment_total: 0,
            change_total: 0,
            transparent_total: 0,
            fee: 0,
            padding_outputs: 0,
            // Sized to their share of the action limit, so that neither grows.
            outputs: Vec::with_capacity(MAX_ACTIONS - header.transparent_outputs),
            transparent_outputs: Vec::with_capacity(header.transparent_outputs),
        };
        Ok(Box::new(Self {
            sighash,
            summary,
            output_total: 0,
            count: 0,
            seen: 0,
            confirmed: 0,
            nullifiers: [[0; 32]; MAX_ACTIONS],
            records: Records::default(),
            ivk_cache: None,
        }))
    }

    /// Hashes a transparent output and adds it to the summary. Its address is
    /// solved from the hashed `scriptPubKey`.
    fn transparent_output(&mut self, output: &scanner::TransparentOutput<'_>) -> Result<()> {
        let index = self.summary.transparent_outputs.len();
        if index >= MAX_TRANSPARENT_OUTPUTS {
            return Err(Error::Internal);
        }
        self.sighash
            .transparent_output(output.value, output.script_pubkey);
        self.summary.transparent_total = add_amount(self.summary.transparent_total, output.value)?;
        let row = TransparentOutput {
            index,
            kind: output.kind,
            hash: output.hash,
            value: output.value,
        };
        self.summary.transparent_outputs.push(row);
        Ok(())
    }

    /// The next transparent output to show, once the action count is known.
    fn next_transparent(&mut self) -> Option<TransparentOutput> {
        if self.count == 0 {
            return None;
        }
        let row = *self.summary.transparent_outputs.get(self.confirmed)?;
        self.confirmed += 1;
        Some(row)
    }

    /// The Ironwood action count, which the scanner has capped.
    fn shielded(&mut self, shielded: &scanner::Shielded) -> Result<()> {
        if self.count != 0 || shielded.actions == 0 {
            return Err(Error::Internal);
        }
        self.count = shielded.actions;
        Ok(())
    }

    /// Verifies one action and returns its output if it is a payment. A dummy
    /// spend's signature is checked in `review_stream`, once the sighash is
    /// known.
    #[inline(never)]
    fn action(
        &mut self,
        action: &scanner::Action<'_>,
        fvk: &FullViewingKey,
        fvk_bytes: &[u8; 96],
        account: &AccountDerivation,
        progress: &mut dyn FnMut(),
    ) -> Result<Option<ReviewedOutput>> {
        let index = self.seen;
        if index >= self.count {
            return Err(Error::Internal);
        }
        let spend = &action.spend;
        let output = &action.output;
        let input_value = spend.value;
        let output_value = output.value;

        for claim in [spend.zip32_derivation, output.zip32_derivation]
            .iter()
            .flatten()
        {
            account.check(claim)?;
        }

        // A real spend's FVK must be the account's, byte for byte. A dummy
        // spend is parsed with its own FVK, which the verifier checks.
        let wire_fvk = if input_value == 0 {
            Some(*spend.fvk)
        } else {
            ensure(
                bool::from(fvk_bytes.as_slice().ct_eq(spend.fvk.as_slice())),
                Error::Malformed,
            )?;
            None
        };
        // `Action::from_parts` refuses an identity `rk`, whose only encoding
        // is all zeros; a dummy spend could otherwise reach it.
        ensure(*spend.rk != [0; 32], Error::Malformed)?;
        let parsed = parse(action, wire_fvk, progress)?;
        progress();

        self.summary.input_total = add_amount(self.summary.input_total, input_value)?;
        self.output_total = add_amount(self.output_total, output_value)?;
        ensure(
            !self.nullifiers[..index].contains(spend.nullifier),
            Error::Malformed,
        )?;
        self.nullifiers[index] = *spend.nullifier;
        parsed.verify_cv_net().map_err(|_| Error::Malformed)?;
        progress();
        if self.ivk_cache.is_none() {
            self.ivk_cache = Some(IvkCache(fvk.scope_classifier_with_progress(progress)));
            progress();
        }
        let classifier = &self.ivk_cache.as_ref().ok_or(Error::Internal)?.0;
        parsed
            .spend()
            .verify_nullifier_with_progress(Some(fvk), Some(classifier), progress)
            .map_err(|_| Error::Malformed)?;
        progress();
        parsed
            .spend()
            .verify_rk(Some(fvk))
            .map_err(|_| Error::Malformed)?;
        progress();
        let note = parsed
            .output()
            .verify_note_commitment_with_progress(parsed.spend(), progress)
            .map_err(|_| Error::Malformed)?;
        progress();
        let record = if input_value == 0 {
            let signature = spend.spend_auth_sig.ok_or(Error::Malformed)?;
            Record::Dummy {
                rk: *spend.rk,
                signature: *signature,
            }
        } else {
            ensure(spend.spend_auth_sig.is_none(), Error::Policy)?;
            Record::Real {
                rk: *spend.rk,
                alpha: *spend.alpha,
            }
        };
        self.sighash.action(&ActionEffects {
            cv_net: action.cv_net,
            nullifier: spend.nullifier,
            rk: spend.rk,
            cmx: output.cmx,
            ephemeral_key: output.ephemeral_key,
            enc_ciphertext: output.enc_ciphertext,
            out_ciphertext: output.out_ciphertext,
        });
        let recipient = parsed.output().recipient().ok_or(Error::Malformed)?;
        let recipient_scope = classifier.scope_for_address(&recipient);
        let outgoing_scope = if recipient_scope == Some(Scope::Internal) {
            Scope::Internal
        } else {
            Scope::External
        };
        let memo = memo::recover(&parsed, fvk, outgoing_scope, &note, progress)?;
        progress();
        self.records.0[index] = Some(record);
        self.seen += 1;
        if output_value == 0 {
            self.summary.padding_outputs += 1;
            return Ok(None);
        }
        let kind = if recipient_scope == Some(Scope::Internal) {
            self.summary.change_total = add_amount(self.summary.change_total, output_value)?;
            OutputKind::InternalChange
        } else {
            self.summary.payment_total = add_amount(self.summary.payment_total, output_value)?;
            OutputKind::Payment
        };
        let reviewed = ReviewedOutput {
            action_index: index,
            receiver: recipient.to_raw_address_bytes(),
            value: output_value,
            kind,
            memo,
        };
        self.summary.outputs.push(reviewed.clone());
        Ok((kind == OutputKind::Payment).then_some(reviewed))
    }
}

/// Parses the action as `Pczt::parse` does. `progress` is called while a
/// dummy spend's FVK is checked.
fn parse(
    action: &scanner::Action<'_>,
    fvk: Option<[u8; 96]>,
    progress: &mut dyn FnMut(),
) -> Result<Action> {
    let spend = &action.spend;
    let output = &action.output;
    let spend = Spend::parse_with_progress(
        *spend.nullifier,
        *spend.rk,
        spend.spend_auth_sig.copied(),
        Some(*spend.recipient),
        Some(spend.value),
        Some(*spend.rho),
        Some(*spend.rseed),
        fvk,
        None,
        Some(*spend.alpha),
        None,
        None,
        NoteVersion::V3,
        BTreeMap::new(),
        progress,
    )
    .map_err(|_| Error::Malformed)?;
    let output = Output::parse(
        *spend.nullifier(),
        *output.cmx,
        *output.ephemeral_key,
        output.enc_ciphertext.to_vec(),
        output.out_ciphertext.to_vec(),
        Some(*output.recipient),
        Some(output.value),
        Some(*output.rseed),
        output.ock.copied(),
        None,
        None,
        NoteVersion::V3,
        BTreeMap::new(),
    )
    .map_err(|_| Error::Malformed)?;
    Action::parse(*action.cv_net, spend, output, Some(*action.rcv)).map_err(|_| Error::Malformed)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The trailer requires the default flags, under which cross-address
    /// transfers are allowed, so there is no cross-address check.
    #[test]
    fn test_default_flags_allow_cross_address_transfers() {
        let version = BundleVersion::ironwood_v3();
        assert!(version.default_flags().cross_address_enabled());
        assert_eq!(version.default_flags().to_byte(version), Some(0b111));
    }
}
