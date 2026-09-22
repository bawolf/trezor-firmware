//! Streaming counterpart of [`Engine`](crate::Engine): consumes a PCZT in
//! chunks and enforces the approval semantics of `Engine::begin`, `approve`
//! and `sign` while never holding more than one action
//! (docs/common/zcash-ironwood-signing.md §4 and §6).
//!
//! Every check names the engine line it reproduces. Agreement with the engine
//! on projection, sighash, error class and signature bytes over the
//! conformance corpus under every chunking is proven by
//! tests/session_equivalence.rs.
//!
//! Order of checks. The engine admits the whole encoding, then parses the
//! whole bundle, then verifies action by action. The session cannot: header
//! policy runs at the header, parse and verification of action `i` run before
//! action `i + 1` is read, and the trailer's flag, anchor and bundle-level
//! checks run last. Within an action the wire-FVK byte comparison and the
//! identity-`rk` check are hoisted before the value sums, and a dummy
//! signature waits for the sighash. A single fault yields the engine's class;
//! two faults in different sections can be reported in the other order.
//!
//! The FVK is never retained: `begin` binds its encoding and every `feed`
//! borrows the caller's key again, so the only key material this module owns
//! is zeroized with the stream (finding 2 of the streaming-core review).

// `lib.rs` exports `Session` unconditionally for the phase-2 retained link and
// the allocator probe that drives it; the device wire handler is design §10
// phase 4 and does not exist yet.

use alloc::boxed::Box;
use alloc::collections::BTreeMap;
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
use zcash_protocol::consensus::BranchId;
use zcash_protocol::constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID};
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

use crate::digest::{ActionEffects, Digest};
use crate::stream::{self, Item, Scanner};
use crate::{
    Error, MAX_ACTIONS, OutputKind, OwnDerivation, Policy, Projection, Result, Review,
    ReviewedOutput, Token, add, ensure_malformed, ensure_policy, ensure_state, same_bytes,
    verify_encryption, wire,
};

/// Inner personalization of the consent token's byte commitment (design §6).
const STREAM_PERSONAL: &[u8; 15] = b"IWStreamBytesV1";

/// `Spend::parse` requires a nullifier; the signing path never reads it.
const UNUSED_NULLIFIER: [u8; 32] = [0; 32];

/// What the session yields after consuming bytes.
#[derive(Debug)]
pub enum Event {
    /// Bytes were consumed without completing a reviewable item.
    None,
    /// A verified value-bearing payment output. Not consent: no token exists
    /// yet and any later rejection discards everything (design §4 step 8).
    ConfirmOutput(ReviewedOutput),
    /// The whole PCZT verified; carries the token the trusted UI may approve.
    Review(Review),
}

/// One retained action (design §4 `ActionRecord`).
#[derive(Clone, Copy, Zeroize)]
enum Record {
    /// Real spend: signed after approval once `rk` is rechecked against the
    /// caller's `ask` and this `alpha`.
    Real { rk: [u8; 32], alpha: [u8; 32] },
    /// Dummy spend: signed by the host; verified once the sighash exists.
    /// A dummy note is host-chosen, so its `rk` is checked against the
    /// host-chosen wire FVK, not the session's: what authorizes it is the
    /// host's signature over the sighash, not key ownership. Do not "fix"
    /// this by comparing the dummy FVK to the session FVK.
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

    /// Copies the records out and zeroizes them where they were. A plain move
    /// or `Option::take` rewrites only the discriminant and leaves `rk`,
    /// `alpha` and the dummy signatures in the source storage.
    fn take_zeroizing(&mut self) -> Self {
        let taken = Self(self.0);
        self.zeroize();
        taken
    }
}

/// One signature record of the response message (design §3).
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
        let slot = self.records.get_mut(self.len).ok_or(Error::internal())?;
        *slot = record;
        self.len += 1;
        Ok(())
    }

    /// Records in ascending action order.
    pub fn records(&self) -> &[SignatureRecord] {
        &self.records[..self.len]
    }
}

/// Engine's `Pending` (`lib.rs`) without the retained PCZT.
struct Pending {
    approved: bool,
    sighash: [u8; 32],
    expected_ak: SpendValidatingKey,
    records: Records,
}

/// Engine's `PendingSlot` (`lib.rs`) with the same contract: the
/// returned token is duplicated here so the session compares and clears its
/// own copy. The slot is only ever cleared in place, never moved out, so
/// `Token` and `Records` zeroize on drop in the storage they occupied.
struct Slot {
    /// Boxed (MUST-FIX #1): the CAP-sized `Records` inside `Pending` stay in
    /// the region so `review_stream` never builds `Pending` by value on the
    /// stack.
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
            session: token.session,
            request: token.request,
            context: token.context,
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

/// MUST-FIX #3: secret-bearing wrapper around the session-cached
/// [`ScopeClassifier`] (the device FVK's external+internal `ivk` scalars).
/// `ScopeClassifier` implements no `Zeroize`, so without this its bytes would
/// linger in the freed region block (and then in the GC heap) on every
/// cancel/error/review/normal-end path. This newtype has a wiping `Drop` and an
/// in-place [`IvkCache::wipe`] for the review path, mirroring
/// [`Records::take_zeroizing`]. Both the ironwood crate and the orchard fork
/// `#![forbid(unsafe_code)]`, so the actual overwrite lives in
/// `ScopeClassifier::wipe` (a fixed-constant store rooted with `black_box` to
/// defeat dead-store elimination); this newtype simply drives it on drop and
/// before the `Body` is released.
struct IvkCache(ScopeClassifier);

impl IvkCache {
    /// The wrapped classifier, for the read-only Sinsemilla-free scope checks.
    fn classifier(&self) -> &ScopeClassifier {
        &self.0
    }

    /// Overwrites the two cached `ivk` scalars where they live.
    fn wipe(&mut self) {
        self.0.wipe();
    }
}

impl Drop for IvkCache {
    fn drop(&mut self) {
        self.wipe();
    }
}

/// Per-stream state after the header (design §4 "Action `i`").
struct Body {
    digest: Digest,
    projection: Projection,
    output_total: u64,
    /// Declared action count; the scanner yields exactly this many.
    count: usize,
    seen: usize,
    nullifiers: [[u8; 32]; MAX_ACTIONS],
    records: Records,
    /// DEDUP LEVER 3: the device FVK's external+internal ivk cache, built once
    /// on the first action and reused for every later action, so the two
    /// `Commit^ivk` Sinsemilla evaluations are paid once per bundle instead of
    /// per action. Applied by `verify_nullifier_with_classifier` only to spends
    /// validated under the device FVK (real spends); dummy spends fall back to
    /// `fvk.scope_for_address` inside orchard (MUST-FIX #3). Derived from the
    /// FVK, which the stream already retains as bytes. Wiped on every teardown
    /// path via [`IvkCache`] (MUST-FIX #3).
    scope_classifier: Option<IvkCache>,
}

struct Stream {
    scanner: Scanner,
    /// `IWStreamBytesV1` over LE64(declared length) ‖ every consumed byte ‖
    /// LE64(action count) (design §6).
    bytes: State,
    declared_len: usize,
    /// Encoding of the FVK bound at `begin`: what every `feed` must present
    /// again, what real spends must carry on the wire, and what the token
    /// commits to. `orchard::keys::FullViewingKey` cannot be zeroized, so the
    /// key object itself is only ever borrowed for one call.
    fvk: Zeroizing<[u8; 96]>,
    expected_ak: SpendValidatingKey,
    /// The device's seed fingerprint with the consented account path: what
    /// any `zip32_derivation` on the wire must name: a claim is admitted
    /// only when it names this seed and the consented account path.
    own: OwnDerivation,
    /// Present once the header has been verified. Boxed (MUST-FIX #1) so the
    /// CAP-sized `Body` is never moved by value on the stack; it lives in the
    /// region and is only ever reached through this pointer.
    body: Option<Box<Body>>,
}

#[derive(Clone, Copy)]
struct Trailer {
    flags: u8,
    value_sum: u64,
    anchor: Option<[u8; 32]>,
}

/// Owned request state, one PCZT at a time. Only a future trusted UI may call
/// [`Session::approve`].
pub struct Session<R> {
    rng: R,
    policy: Policy,
    session: [u8; 32],
    counter: u64,
    /// Boxed (MUST-FIX #1): the CAP-sized `Stream`/`Body` live in the region
    /// and are reached through this pointer, so
    /// `session_begin`/`session_feed` never stage them by value on the 32
    /// KB device stack.
    stream: Option<Box<Stream>>,
    slot: Slot,
}

impl<R> Drop for Session<R> {
    fn drop(&mut self) {
        self.reset();
        self.session.zeroize();
        self.counter.zeroize();
    }
}

impl<R> Session<R> {
    /// Idempotently cancels the current request: any in-flight stream, its
    /// records and the pending consent are dropped.
    pub fn cancel(&mut self) {
        self.reset();
    }

    fn reset(&mut self) {
        self.stream = None;
        self.slot.clear();
    }

    #[cfg(feature = "test")]
    pub fn test_has_pending_request(&self) -> bool {
        self.slot.pending.is_some()
    }

    #[cfg(feature = "test")]
    pub fn test_request_binding_is_zero(&self) -> bool {
        self.slot.token.is_none()
    }

    #[cfg(feature = "test")]
    pub fn test_is_streaming(&self) -> bool {
        self.stream.is_some()
    }
}

impl<R: RngCore + CryptoRng> Session<R> {
    /// As [`crate::Engine::with_rng`].
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
            stream: None,
            slot: Slot::empty(),
        })
    }

    /// Starts a request of exactly `declared_len` bytes for the device-derived
    /// FVK and seed fingerprint ([`crate::seed_fingerprint`]). As
    /// [`crate::Engine::begin`] the pending consent is cleared and the
    /// counter advances before anything is read; the length bound is
    /// `wire::scan`'s.
    pub fn begin(
        &mut self,
        declared_len: usize,
        fvk: &FullViewingKey,
        seed_fingerprint: &[u8; 32],
    ) -> Result<()> {
        self.reset();
        self.counter = self.counter.checked_add(1).ok_or(Error::state())?;
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
            own: OwnDerivation::new(seed_fingerprint, self.policy.request),
            body: None,
        }));
        Ok(())
    }

    /// Consumes the next bytes and returns how many were taken together with
    /// the event they completed. Bytes not consumed belong after that event
    /// and must be fed again, exactly as with `Scanner::feed`; this is how one
    /// chunk that completes several outputs yields one confirmation per call
    /// without buffering events. `fvk` must be the key given to `begin`: the
    /// session keeps only its encoding, so the caller lends the key for every
    /// call and a different key is a `State` error. Any error, including
    /// bytes after `Review`, resets the session.
    pub fn feed(&mut self, chunk: &[u8], fvk: &FullViewingKey) -> Result<(usize, Event)> {
        let result = self.advance(chunk, fvk);
        if result.is_err() {
            self.reset();
        }
        result
    }

    fn advance(&mut self, chunk: &[u8], fvk: &FullViewingKey) -> Result<(usize, Event)> {
        let bound = self.stream.as_deref().ok_or(Error::state())?;
        let offered = Zeroizing::new(fvk.to_bytes());
        ensure_state(same_bytes(offered.as_slice(), bound.fvk.as_slice()))?;
        let mut consumed = 0;
        while consumed < chunk.len() {
            let stream = self.stream.as_deref_mut().ok_or(Error::state())?;
            let (used, item) = stream.scanner.feed(&chunk[consumed..])?;
            stream.bytes.update(&chunk[consumed..consumed + used]);
            consumed += used;
            let trailer = match item {
                None => continue,
                Some(Item::Header(header)) => {
                    if stream.body.is_some() {
                        return Err(Error::internal());
                    }
                    // `Body::new` returns a `Box<Body>` built in the region, so the
                    // CAP-sized `Body` never materialises on this frame (MUST-FIX #1).
                    stream.body = Some(Body::new(header, &self.policy)?);
                    continue;
                }
                Some(Item::Action(action)) => {
                    let body = stream.body.as_deref_mut().ok_or(Error::internal())?;
                    match body.action(&action, fvk, &stream.fvk, &stream.own)? {
                        Some(output) => return Ok((consumed, Event::ConfirmOutput(output))),
                        None => continue,
                    }
                }
                Some(Item::Trailer(trailer)) => Trailer {
                    flags: trailer.flags,
                    value_sum: trailer.value_sum,
                    anchor: trailer.anchor.copied(),
                },
            };
            let review = self.review(trailer)?;
            return Ok((consumed, Event::Review(review)));
        }
        Ok((consumed, Event::None))
    }

    /// Trailer, VerifyDummies and Totals of design §4, then the token. The
    /// stream is processed where it lives and dropped there on every exit:
    /// `Scanner::drop` and `Zeroizing` then clear the section buffer and the
    /// FVK encoding in place, and the records leave through
    /// [`Records::take_zeroizing`].
    fn review(&mut self, trailer: Trailer) -> Result<Review> {
        let result = self.review_stream(trailer);
        self.stream = None;
        result
    }

    // MUST-FIX #1: kept out of line so its (cold, once-per-stream) records /
    // pending temporaries are NOT reserved in the always-live `session_feed`
    // frame that the per-action verify runs under.
    #[inline(never)]
    fn review_stream(&mut self, trailer: Trailer) -> Result<Review> {
        let stream = self.stream.as_deref_mut().ok_or(Error::internal())?;
        if !stream.scanner.is_finished() {
            return Err(Error::internal());
        }
        let body = stream.body.as_deref_mut().ok_or(Error::internal())?;
        if body.seen != body.count {
            return Err(Error::internal());
        }
        // The records are the only secret-bearing part of `Body`; once they
        // are out and their source zeroized, what stays behind is the digest
        // states, the projection and the nullifiers, all host-known.
        let records = body.records.take_zeroizing();
        // MUST-FIX #3: volatile-zero the cached external+internal `ivk` scalars
        // in place BEFORE the `Body` is released, mirroring
        // `Records::take_zeroizing`. Dropping the box below re-wipes via
        // `IvkCache::drop`; wiping here covers the classifier on the review
        // path even if the drop-in-place path is ever refactored away.
        if let Some(cache) = body.scope_classifier.as_mut() {
            cache.wipe();
        }
        // Move only the small, host-known projection state out of the boxed
        // `Body`. The CAP-sized fields (`nullifiers`, the emptied `records`,
        // the wiped classifier) stay in the region and drop in place when the
        // box is freed, so nothing CAP-sized lands on the stack (MUST-FIX #1).
        let Body {
            digest,
            mut projection,
            output_total,
            count,
            seen: _,
            nullifiers: _,
            records: _,
            scope_classifier: _,
        } = *stream.body.take().ok_or(Error::internal())?;

        // Flags: `Flags::from_byte` is what `Bundle::parse` runs (parse.rs:41-42)
        // and `Pczt::parse` turns into `Malformed` (`validate`); the exact
        // default-flag gate is the `flag_byte()` check opening `verify_bundle`.
        // The cross-address restriction the engine checks next
        // (`verify_cross_address_restriction`) is a no-op under these flags.
        let version = BundleVersion::ironwood_v3();
        Flags::from_byte(trailer.flags, version).ok_or(Error::malformed())?;
        ensure_policy(
            trailer.flags
                == version
                    .default_flags()
                    .to_byte(version)
                    .expect("valid default"),
        )?;
        // Anchor validity as `Bundle::parse` (parse.rs:63-65); an absent anchor
        // is the zero placeholder there, which is always valid.
        if let Some(anchor) = trailer.anchor {
            Anchor::from_bytes(anchor)
                .into_option()
                .ok_or(Error::malformed())?;
        }
        // The value sum admission already bounded (non-negative, at most
        // MAX_MONEY) is what the engine's digest hashes (`effects::sighash`).
        let value_balance = i64::try_from(trailer.value_sum).map_err(|_| Error::malformed())?;
        let sighash = digest.finish(trailer.flags, value_balance);

        // VerifyDummies: `verify_bundle`'s dummy-spend arm, deferred to here
        // because a dummy signature verifies against the finished sighash.
        for (_, record) in records.iter() {
            if let Record::Dummy { rk, signature } = record {
                VerificationKey::<SpendAuth>::try_from(*rk)
                    .map_err(|_| Error::internal())?
                    .verify(&sighash, &Signature::from(*signature))
                    .map_err(|_| Error::malformed())?;
            }
        }

        // Bundle-level checks: the tail of `verify_bundle` (a real spend and
        // at least one reviewed output, the fee, the value-sum balance).
        let real = records
            .iter()
            .any(|(_, record)| matches!(record, Record::Real { .. }));
        ensure_policy(real && !projection.outputs.is_empty())?;
        projection.fee = projection
            .input_total
            .checked_sub(output_total)
            .ok_or(Error::malformed())?;
        ensure_malformed(i64::try_from(trailer.value_sum).ok() == Some(projection.fee as i64))?;
        ensure_policy(projection.fee <= self.policy.limits.maximum_fee)?;
        ensure_malformed(
            add(
                add(projection.payment_total, projection.change_total)?,
                projection.fee,
            )? == projection.input_total,
        )?;

        // Token as `Engine::begin` builds it, with the byte string replaced by the
        // running byte digest (design §6).
        stream.bytes.update(&(count as u64).to_le_bytes());
        let stream_digest: [u8; 32] = stream
            .bytes
            .finalize()
            .as_bytes()
            .try_into()
            .expect("configured 32-byte hash");
        let request = self.policy.request;
        let limits = self.policy.limits;
        let mut h = Params::new()
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
            .update(&*stream.fvk)
            .update(&sighash)
            .update(&(stream.declared_len as u64).to_le_bytes())
            .update(&stream_digest);
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
            Box::new(Pending {
                approved: false,
                sighash,
                expected_ak: stream.expected_ak.clone(),
                records,
            }),
            &token,
        );
        Ok(Review {
            token,
            projection,
            #[cfg(feature = "test")]
            sighash,
        })
    }

    /// As [`crate::Engine::approve`]. A consent call while bytes are
    /// still streaming is a protocol violation and discards the stream.
    pub fn approve(&mut self, token: &Token) -> Result<()> {
        self.stream = None;
        let Some(pending) = self.slot.pending.as_ref() else {
            return Err(Error::state());
        };
        if !self.slot.matches(token) || pending.approved {
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

    /// Consumes consent on every signing attempt, including all failures, as
    /// [`crate::Engine::sign`] does. Signatures are released only when every
    /// real spend signed. The slot is borrowed, never moved out, and
    /// cleared in place on every exit, so the records and the retained
    /// token zeroize where they were stored.
    pub fn sign(&mut self, token: &Token, ask: &SpendAuthorizingKey) -> Result<Signatures> {
        self.stream = None;
        let result = match (&self.slot.pending, &self.slot.token) {
            (Some(pending), Some(retained)) => {
                Self::sign_records(pending, ask, token, retained, &mut self.rng)
            }
            _ => Err(Error::state()),
        };
        self.slot.clear();
        result
    }

    fn sign_records(
        pending: &Pending,
        ask: &SpendAuthorizingKey,
        token: &Token,
        retained_token: &Token,
        rng: &mut R,
    ) -> Result<Signatures> {
        ensure_state(pending.approved && *retained_token == *token)?;
        if SpendValidatingKey::from(ask) != pending.expected_ak {
            return Err(Error::signing());
        }
        let mut signatures = Signatures::empty();
        for (index, record) in pending.records.iter() {
            let Record::Real { rk, alpha } = record else {
                continue;
            };
            // `Action::sign` (orchard-0.15.3/src/pczt/signer.rs:24-37) reads the
            // parsed `alpha` and `rk`; the record is that sub-parse of the wire
            // fields, re-derived here because `pallas::Scalar` is not nameable
            // in this crate. A wrong key is `Error::signing()`, as in
            // [`crate::Engine::sign`].
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
            .map_err(|_| Error::internal())?;
            let alpha = spend.alpha().as_ref().ok_or(Error::internal())?;
            let rsk = ask.randomize(alpha);
            if VerificationKey::from(&rsk) != *spend.rk() {
                return Err(Error::signing());
            }
            let signature = rsk.sign(&mut *rng, &pending.sighash);
            signatures.push(SignatureRecord {
                action_index: u8::try_from(index).map_err(|_| Error::internal())?,
                signature: <[u8; 64]>::from(&signature),
            })?;
        }
        Ok(signatures)
    }
}

impl Body {
    /// Header checks of [`crate::validate`], then the digest. Returns a
    /// `Box<Body>` built in the region and stays out of line (MUST-FIX #1) so
    /// the CAP-sized `Body` value never materialises in the caller's
    /// `session_feed` frame.
    #[inline(never)]
    fn new(header: stream::Header, policy: &Policy) -> Result<Box<Self>> {
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

        let digest = Digest::new(&wire::Header {
            version: header.version,
            group: header.group,
            branch: header.branch,
            lock_time: header.lock_time,
            expiry: header.expiry,
            coin_type: header.coin_type,
        })?;
        // The projection `validate` builds from the parsed header.
        let projection = Projection {
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
            outputs: Vec::with_capacity(MAX_ACTIONS),
        };
        Ok(Box::new(Self {
            digest,
            projection,
            output_total: 0,
            count: header.actions,
            seen: 0,
            nullifiers: [[0; 32]; MAX_ACTIONS],
            records: Records::default(),
            scope_classifier: None,
        }))
    }

    /// Design §4 steps 1-9: the per-action checks of [`crate::verify_bundle`]
    /// in the engine's order, with the dummy signature deferred to
    /// VerifyDummies.
    #[inline(never)]
    fn action(
        &mut self,
        action: &stream::Action<'_>,
        fvk: &FullViewingKey,
        fvk_bytes: &[u8; 96],
        own: &OwnDerivation,
    ) -> Result<Option<ReviewedOutput>> {
        let index = self.seen;
        if index >= self.count {
            return Err(Error::internal());
        }
        let spend = &action.spend;
        let output = &action.output;
        let input_value = spend.value;
        let output_value = output.value;

        // lib.rs `verify_bundle`: a derivation claim must be the device's own.
        for claim in [spend.zip32_derivation, output.zip32_derivation]
            .into_iter()
            .flatten()
        {
            own.admit(claim.seed_fingerprint, claim.path.into_iter())?;
        }

        // Design §4 step 3. For a real spend `Spend::parse` would derive the
        // wire FVK (two Sinsemilla IVK derivations) only for
        // `fvk_for_validation` to compare it with the session FVK
        // (orchard-0.15.3/src/pczt/verify.rs:80-84). Equal keys have equal
        // canonical encodings and any other bytes fail as `Malformed` whether
        // or not they parse, so comparing bytes decides the same; leaving the
        // parsed `fvk` absent makes the verifier use the session FVK
        // (verify.rs:85). A dummy spend keeps the full parse because its own,
        // host-chosen FVK is what the verifier checks (verify.rs:83); that is
        // sound because a dummy is authorized by the host's signature over
        // the sighash (VerifyDummies), not by ownership of any key.
        let wire_fvk = if input_value == 0 {
            Some(*spend.fvk)
        } else {
            ensure_malformed(same_bytes(fvk_bytes, spend.fvk))?;
            None
        };
        // `orchard::Action::from_parts` (orchard-0.15.3/src/action.rs:65-67)
        // refuses an identity `rk`; the engine reaches it through
        // `effects::sighash` (called by `validate`) before any
        // verification, as `Malformed`. Nothing below reproduces it: a real
        // spend cannot reach it (`verify_rk` would need `alpha = -ask`), but a
        // dummy spend's `rk` is checked against the host-chosen wire FVK, so
        // the host sets `alpha = -ask_dummy` and `rk = ak_dummy + alpha·G = O`.
        // `[0; 32]` is the identity's only encoding: pasta_curves-0.5.1/
        // src/curves.rs:693-696 (`to_bytes` of the identity is all zeros) and
        // :663-670 (`from_bytes` maps x = 0 with the sign bit clear to the
        // identity, and any other encoding to a non-identity point or
        // nothing), and reddsa-0.5.1/src/verification_key.rs:99-111 admits it.
        // The other `from_parts` condition, a non-identity `epk`, is enforced
        // by `verify_encryption` below. Proven by
        // tests/session_equivalence.rs `identity_rk_dummy_spend...`.
        ensure_malformed(*spend.rk != [0; 32])?;
        let parsed = parse(action, wire_fvk)?;

        // Step 1: the running input and output totals.
        self.projection.input_total = add(self.projection.input_total, input_value)?;
        self.output_total = add(self.output_total, output_value)?;
        // Step 2: no nullifier repeats within the bundle.
        ensure_malformed(!self.nullifiers[..index].contains(spend.nullifier))?;
        self.nullifiers[index] = *spend.nullifier;
        // Step 3: the value commitment, then nullifier ownership.
        parsed.verify_cv_net().map_err(|_| Error::malformed())?;
        // DEDUP LEVER 3: build the ivk cache once (first action), reuse for the
        // rest of the bundle. Narrow borrows so the `&mut self.scope_classifier`
        // never spans the later `self` mutations.
        parsed
            .spend()
            .verify_nullifier_with_classifier(
                Some(fvk),
                Some(
                    self.scope_classifier
                        .get_or_insert_with(|| IvkCache(fvk.scope_classifier()))
                        .classifier(),
                ),
            )
            .map_err(|_| Error::malformed())?;
        parsed
            .spend()
            .verify_rk(Some(fvk))
            .map_err(|_| Error::malformed())?;
        // MUST-FIX #4/#2: reuse the cmx-validated note for output recovery.
        let note = parsed
            .output()
            .verify_note_commitment(parsed.spend())
            .map_err(|_| Error::malformed())?;
        // Step 4: the spend record; the dummy signature waits for the sighash.
        let record = if input_value == 0 {
            let signature = spend.spend_auth_sig.ok_or(Error::malformed())?;
            Record::Dummy {
                rk: *spend.rk,
                signature: *signature,
            }
        } else {
            ensure_policy(spend.spend_auth_sig.is_none())?;
            Record::Real {
                rk: *spend.rk,
                alpha: *spend.alpha,
            }
        };
        self.digest.action(&ActionEffects {
            cv_net: action.cv_net,
            nullifier: spend.nullifier,
            rk: spend.rk,
            cmx: output.cmx,
            ephemeral_key: output.ephemeral_key,
            enc_ciphertext: output.enc_ciphertext,
            out_ciphertext: output.out_ciphertext,
        });
        // Step 5: the recipient and its scope.
        let recipient = parsed.output().recipient().ok_or(Error::malformed())?;
        let recipient_scope = self
            .scope_classifier
            .get_or_insert_with(|| IvkCache(fvk.scope_classifier()))
            .classifier()
            .scope_for_address(&recipient);
        let outgoing_scope = if recipient_scope == Some(Scope::Internal) {
            Scope::Internal
        } else {
            Scope::External
        };
        let memo = verify_encryption(&parsed, fvk, outgoing_scope, &note)?;
        self.records.0[index] = Some(record);
        self.seen += 1;
        // Step 6: padding, change and payment outputs.
        if output_value == 0 {
            self.projection.padding_outputs += 1;
            return Ok(None);
        }
        let kind = if recipient_scope == Some(Scope::Internal) {
            self.projection.change_total = add(self.projection.change_total, output_value)?;
            OutputKind::InternalChange
        } else {
            self.projection.payment_total = add(self.projection.payment_total, output_value)?;
            OutputKind::Payment
        };
        let reviewed = ReviewedOutput {
            action_index: index,
            receiver: recipient.to_raw_address_bytes(),
            value: output_value,
            kind,
            memo,
        };
        self.projection.outputs.push(reviewed.clone());
        Ok((kind == OutputKind::Payment).then_some(reviewed))
    }
}

/// `Spend::parse`, `Output::parse` and `Action::parse` with the arguments the
/// `pczt` crate hands them (pczt-0.9.3/src/orchard.rs:2143-2233); the
/// redactable fields are never absent on this wire, so the crate's field
/// resolution is a no-op. A parse failure is what `Pczt::parse` turns into
/// `Malformed` (`validate`).
fn parse(action: &stream::Action<'_>, fvk: Option<[u8; 96]>) -> Result<Action> {
    let spend = &action.spend;
    let output = &action.output;
    let spend = Spend::parse(
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
    )
    .map_err(|_| Error::malformed())?;
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
    .map_err(|_| Error::malformed())?;
    Action::parse(*action.cv_net, spend, output, Some(*action.rcv)).map_err(|_| Error::malformed())
}

#[cfg(test)]
mod tests {
    use orchard::bundle::BundleVersion;

    /// The engine's `verify_cross_address_restriction` check is
    /// omitted here because the exact flag gate makes it vacuous; this pins
    /// that premise.
    #[test]
    fn pinned_default_flags_permit_cross_address_transfers() {
        let version = BundleVersion::ironwood_v3();
        assert!(version.default_flags().cross_address_enabled());
        assert_eq!(version.default_flags().to_byte(version), Some(0b111));
    }
}
