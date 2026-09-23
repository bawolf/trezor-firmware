#![no_std]
#![forbid(unsafe_code)]
#![deny(clippy::all)]
//! Bounded Ironwood PCZT approval core for the Safe 5 Zcash application.
//!
//! The rules this crate enforces are written down in
//! `docs/common/zcash-ironwood-signing.md`; the "§N" in the comments below
//! are that document's sections.
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

mod digest;
#[cfg(feature = "test")]
mod effects;
mod error;
mod hedge;
mod prewarm;
pub mod receive;
mod session;
mod stream;
mod wire;

/// Session-start Pasta square-root table pre-warm; the device
/// signing handler calls it once after installing the region and before the
/// per-action loop.
pub use prewarm::prewarm;

/// Test-only access to bounded wire admission; not a production signing API.
#[cfg(feature = "test")]
pub mod testing {
    pub use crate::wire::preflight;
}

pub use error::{Error, ErrorCode, Result};
pub use hedge::Hedged;
use orchard::Note;
#[cfg(feature = "test")]
use orchard::bundle::BundleVersion;
use orchard::keys::{FullViewingKey, Scope};
#[cfg(feature = "test")]
use orchard::keys::{SpendAuthorizingKey, SpendValidatingKey};
use orchard::note_encryption::IronwoodDomain;
#[cfg(feature = "test")]
use pczt::Pczt;
use pczt::roles::low_level_signer::OrchardParseError;
#[cfg(feature = "test")]
use pczt::roles::low_level_signer::Signer as LowLevelSigner;
#[cfg(feature = "test")]
use pczt::roles::verifier::{OrchardError, Verifier};
#[cfg(feature = "test")]
use rand_core::{CryptoRng, RngCore};
pub use session::{Event, Session, SignatureRecord, Signatures};
/// Maximum number of admitted Ironwood actions.
pub use wire::MAX_ACTIONS;
/// Maximum uploaded or returned PCZT size. A transport enforces this before
/// allocation; [`Engine::begin`] rechecks the exact assembled bytes.
pub use wire::MAX_PCZT_BYTES;
/// Maximum number of admitted transparent outputs. They share [`MAX_ACTIONS`]
/// with the Ironwood actions, because ZIP-317 counts both as logical actions.
pub use wire::MAX_TRANSPARENT_OUTPUTS;
/// Longest `output.user_address` string admitted on the wire.
pub use wire::USER_ADDRESS_BUDGET;
/// The only `scriptPubKey` lengths admitted on a transparent output.
pub use wire::{HASH160_BYTES, MAX_SCRIPT_PUBKEY_BYTES, P2PKH_SCRIPT_BYTES, P2SH_SCRIPT_BYTES};
use zcash_note_encryption::Domain;
use zcash_protocol::consensus::{
    BlockHeight, BranchId, MAIN_NETWORK, NetworkConstants, Parameters, TEST_NETWORK,
};
#[cfg(feature = "test")]
use zcash_protocol::constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID};
use zcash_protocol::value::MAX_MONEY;
use zeroize::{Zeroize, ZeroizeOnDrop};

const EMPTY_MEMO: [u8; 512] = {
    let mut memo = [0; 512];
    memo[0] = 0xf6;
    memo
};
const PADDING_MEMO: [u8; 512] = [0; 512];

/// Byte budget of a text memo the device shows verbatim (design §11: display
/// nonempty memos with a byte budget; text shown, binary or over-budget
/// memos shown as a hash). Half a ZIP-302 memo: enough for a wallet
/// "Reply-To: u1…" memo (a single-receiver unified address is about 106
/// characters) plus text, a few paginated screens, and 32 × 258 bytes of
/// retained projection at the action cap.
pub const MEMO_TEXT_BUDGET: usize = 256;

/// What the device shows for a payment output's memo (design §11).
///
/// The `Text` payload is inline by design, which is why the size difference
/// between the variants is allowed: the signing core is `no_std` and every
/// allocation comes out of the fixed boot-lifetime region, so boxing the memo
/// would trade 258 bytes of enum size for a region allocation per reviewed
/// output -- the one resource the streaming design is built to bound.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(clippy::large_enum_variant)]
pub enum Memo {
    /// No memo: the canonical `0xF6` marker or an all-zero (empty text) memo.
    Empty,
    /// A ZIP-302 text memo within [`MEMO_TEXT_BUDGET`], shown verbatim.
    Text(MemoText),
    /// Anything else (arbitrary `0xFF` data, a reserved leading byte, text
    /// that is not UTF-8, contains NUL, is over the budget, or holds a
    /// character the device cannot draw faithfully -- see
    /// [`renders_faithfully`]): the unkeyed BLAKE2b-256 of the full 512 memo
    /// bytes, shown as hex, which a wallet can recompute from the memo it
    /// built.
    Digest([u8; 32]),
}

/// The bytes of a text memo the device shows; valid UTF-8 by construction.
#[derive(Clone, PartialEq, Eq)]
pub struct MemoText {
    len: u16,
    bytes: [u8; MEMO_TEXT_BUDGET],
}

impl MemoText {
    pub fn as_str(&self) -> &str {
        // Validated in `classify_memo`; an invalid slice cannot be built.
        core::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or("")
    }
}

impl core::fmt::Debug for MemoText {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        core::fmt::Debug::fmt(self.as_str(), f)
    }
}

/// Whether the device can draw `c` as itself.
///
/// "Shown verbatim" has to mean it: a memo is the one host-controlled free
/// text the device displays, and a character that draws as a *different*
/// character, as nothing, or that reorders its neighbours would make the
/// screen disagree with what is signed. Such a memo is hashed instead
/// (`Memo::Digest`), which is honest and still lets the wallet reproduce the
/// value. Refused here, not in the UI, so every model behaves alike.
///
/// A BMP character the font simply lacks is *not* excluded: the glyph lookup
/// draws the nonprintable placeholder for it, which is visibly a placeholder
/// and cannot be mistaken for other text.
fn renders_faithfully(c: char) -> bool {
    let code = c as u32;
    // Supplementary planes alias onto the BMP: the glyph lookup truncates the
    // code point to `u16` (`ui::display::font::GlyphData::get_glyph`), so
    // U+10041 would draw as 'A' and U+1F600 as a random BMP glyph. Silent
    // substitution of a plausible character, which is the dangerous kind.
    if code > 0xFFFF {
        return false;
    }
    // C0/C1 controls. '\n' is the one control the text layout honours (it
    // breaks the line and paginates); '\r' is dropped and the rest draw as
    // the nonprintable placeholder, so none of them is the character the
    // signer wrote.
    if c != '\n' && c.is_control() {
        return false;
    }
    !matches!(
        code,
        // NBSP, which the glyph lookup substitutes with a plain space.
        0x00A0
        // Invisible marks: soft hyphen, combining grapheme joiner, Arabic
        // letter mark, Mongolian vowel separator, variation selectors, BOM,
        // interlinear annotation. They draw as nothing (or as a placeholder
        // that is not what was written) and can hide or fuse neighbours.
        | 0x00AD | 0x034F | 0x061C | 0x180E | 0xFE00..=0xFE0F | 0xFEFF | 0xFFF9..=0xFFFB
        // Zero-width and bidi controls, and the line/paragraph separators:
        // U+200B-U+200F, U+2028-U+202E (LRE/RLE/PDF/LRO/RLO) and
        // U+2060-U+2069 (word joiner, invisible operators, LRI/RLI/FSI/PDI).
        // An RLO can display a memo in an order the bytes do not have.
        | 0x200B..=0x200F | 0x2028..=0x202E | 0x2060..=0x2069
    )
}

/// ZIP-302 classification of a payment output's memo for display.
fn classify_memo(memo: &[u8; 512]) -> Memo {
    if *memo == EMPTY_MEMO || *memo == PADDING_MEMO {
        return Memo::Empty;
    }
    // A leading byte up to 0xF4 is UTF-8 text padded with NUL to 512 bytes.
    if memo[0] <= 0xF4 {
        let end = memo.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
        let text = &memo[..end];
        if text.len() <= MEMO_TEXT_BUDGET
            && !text.contains(&0)
            && core::str::from_utf8(text).is_ok_and(|s| s.chars().all(renders_faithfully))
        {
            let mut bytes = [0; MEMO_TEXT_BUDGET];
            bytes[..text.len()].copy_from_slice(text);
            return Memo::Text(MemoText {
                len: text.len() as u16,
                bytes,
            });
        }
    }
    let mut digest = [0; 32];
    digest.copy_from_slice(
        blake2b_simd::Params::new()
            .hash_length(32)
            .hash(memo)
            .as_bytes(),
    );
    Memo::Digest(digest)
}

/// The hardened bit of a ZIP-32 child index.
pub const ZIP32_HARDENED: u32 = 1 << 31;
/// ZIP-32 purpose of shielded account derivation (`m/32'/...`).
const ZIP32_PURPOSE: u32 = 32;

/// ZIP-32 seed fingerprint: `BLAKE2b-256("Zcash_HD_Seed_FP", I2LEOSP_8(len) ‖
/// seed)` (ZIP 32 §"Seed Fingerprints"), computed over exactly the bytes the
/// device feeds into ZIP-32 master derivation. It equals
/// `zip32::fingerprint::SeedFingerprint::from_seed` for every seed length ZIP
/// 32 admits (32..=252), pinned byte-for-byte by `tests/seed_fingerprint.rs`;
/// `None` for an empty seed and above ZIP 32's 252-byte maximum. A public
/// identifier of the seed, not key material: hosts attach it as the
/// `seed_fingerprint` of a `zip32_derivation` and wallets store it next to
/// the account index.
///
/// **Trezor-only extension, 16-byte seeds.** ZIP 32 defines no fingerprint
/// below 32 bytes and `zip32` returns `None` there, but the device also
/// derives from the 16-byte secret of a restored SLIP-39 backup (itself
/// already outside ZIP 32's master-derivation range). For those wallets this
/// applies the identical construction with length byte 16 rather than
/// refusing the export, so the exported value and the value the wire check
/// compares against can never disagree. The consequence for hosts: for a
/// 16-byte-seed wallet the device's value is authoritative and there is no
/// second implementation to derive it from -- take it from
/// `ZcashGetViewingKey(include_seed_fingerprint=true)`, do not recompute it.
/// If ZIP 32 ever defines a rule for sub-32-byte seeds, this is the decision
/// to revisit.
pub fn seed_fingerprint(seed: &[u8]) -> Option<[u8; 32]> {
    if seed.is_empty() || seed.len() > 252 {
        return None;
    }
    let length = seed.len() as u8;
    let mut fingerprint = [0; 32];
    fingerprint.copy_from_slice(
        blake2b_simd::Params::new()
            .hash_length(32)
            .personal(b"Zcash_HD_Seed_FP")
            .to_state()
            .update(&[length])
            .update(seed)
            .finalize()
            .as_bytes(),
    );
    Some(fingerprint)
}

/// Branch-free equality of two byte strings: the OR-fold has no
/// data-dependent exit, unlike the slice comparison's early return. Used for
/// every host-supplied value compared against a device-derived one (the
/// session FVK, the seed fingerprint).
pub(crate) fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

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

    /// The account derivation the device signs under, `m/32'/coin_type'/
    /// account'`, as hardened ZIP-32 child indices.
    pub fn zip32_path(self) -> [u32; 3] {
        [
            ZIP32_PURPOSE | ZIP32_HARDENED,
            self.network.coin_type() | ZIP32_HARDENED,
            self.account.value() | ZIP32_HARDENED,
        ]
    }
}

/// The derivation the device signs under: its seed fingerprint and the
/// consented account path. A `zip32_derivation` a host attaches to a spend or
/// an output (what the standard SDK does for an account imported as
/// `Spending { seed_fingerprint, index }`) is admitted only when it names
/// exactly this derivation; an absent claim is admitted because the request
/// already selected the account, and any other claim is `Policy`: a host
/// cannot make the device sign under a derivation it did not consent to.
#[derive(Clone, Copy)]
pub(crate) struct OwnDerivation {
    seed_fingerprint: [u8; 32],
    path: [u32; 3],
}

impl OwnDerivation {
    pub(crate) fn new(seed_fingerprint: &[u8; 32], request: RequestContext) -> Self {
        Self {
            seed_fingerprint: *seed_fingerprint,
            path: request.zip32_path(),
        }
    }

    /// A mismatch is `Policy` with no screen, by design.
    ///
    /// That makes the check an equality oracle: a host learns whether a
    /// candidate fingerprint or account path is the device's without any
    /// user interaction. It is acceptable because the oracle answers only
    /// about values the host must already hold to have built an admissible
    /// PCZT at all -- it received the fingerprint from a consented
    /// `ZcashGetViewingKey(include_seed_fingerprint=true)`, and it chose the
    /// account in `ZcashSignPczt` -- and because the same distinction is
    /// already observable from the pre-existing `spend.fvk` equality check
    /// (`Body::action`), which no attacker can avoid. Showing a screen
    /// instead would train users to approve a malformed-host error. The
    /// comparison is [`same_bytes`] so the answer costs the same time
    /// whichever byte differs.
    pub(crate) fn admit(
        &self,
        seed_fingerprint: &[u8; 32],
        path: impl ExactSizeIterator<Item = u32>,
    ) -> Result<()> {
        ensure_policy(
            same_bytes(seed_fingerprint, &self.seed_fingerprint)
                && path.len() == self.path.len()
                && path.zip(self.path).all(|(claimed, own)| claimed == own),
        )
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
    /// Recovered from this action's `enc_ciphertext` under the device's own
    /// derivation, so what is shown is what the signature covers.
    pub memo: Memo,
}

/// Which standard `scriptPubKey` a transparent output pays, and so which
/// Base58Check version byte the handler renders it under: `t1…`/`tm…` for a
/// public key hash, `t3…`/`t2…` for a script hash.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransparentKind {
    P2pkh,
    P2sh,
}

/// A transparent output the device shows. It is a payment by construction:
/// the device owns no transparent keys, so no transparent output can be its
/// own change, and every one of them is public.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransparentOutput {
    /// Position in the transparent bundle, which is the order the outputs
    /// hash and the order they are shown in.
    pub index: usize,
    pub kind: TransparentKind,
    /// The 20-byte hash the address encodes; solved from the `scriptPubKey`
    /// the digest hashes, so what is shown is what the signature covers.
    pub hash: [u8; 20],
    pub value: u64,
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
    /// Sum of [`transparent_outputs`](Self::transparent_outputs). Held apart
    /// from `payment_total` because it is the part of the payment that is
    /// public, which is the whole reason the handler warns about it.
    pub transparent_total: u64,
    pub fee: u64,
    pub padding_outputs: usize,
    pub outputs: Vec<ReviewedOutput>,
    pub transparent_outputs: Vec<TransparentOutput>,
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

/// The whole-PCZT reference implementation. It is the oracle the streaming
/// `Session` is differentially tested against (`tests/session_equivalence.rs`,
/// `tests/stream_equivalence.rs`), never a device path: the firmware calls
/// `Session` exclusively. Host-only, so the firmware image carries one
/// implementation and one PCZT parser.
#[cfg(feature = "test")]
struct Pending {
    pczt: Pczt,
    signing_indices: Vec<usize>,
    approved: bool,
    header: wire::Header,
    /// The admitted transparent outputs, kept so `sign` recomputes the same
    /// digest node it approved rather than an empty one.
    transparent_outputs: Vec<TransparentOutput>,
    sighash: [u8; 32],
    expected_ak: SpendValidatingKey,
}

#[cfg(feature = "test")]
struct PendingSlot {
    pending: Option<Pending>,
    // Deliberately duplicates the returned Token's binding. Review owns the
    // caller's token while the engine independently compares and clears this copy.
    session: [u8; 32],
    request: u64,
    context: [u8; 32],
}

#[cfg(feature = "test")]
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

/// The whole-PCZT reference implementation. It is the oracle the streaming
/// `Session` is differentially tested against (`tests/session_equivalence.rs`,
/// `tests/stream_equivalence.rs`), never a device path: the firmware calls
/// `Session` exclusively. Host-only, so the firmware image carries one
/// implementation and one PCZT parser.
#[cfg(feature = "test")]
pub struct Engine<R> {
    rng: R,
    policy: Policy,
    session: [u8; 32],
    counter: u64,
    slot: PendingSlot,
}

#[cfg(feature = "test")]
impl<R> Drop for Engine<R> {
    fn drop(&mut self) {
        self.slot.clear();
        self.session.zeroize();
        self.counter.zeroize();
    }
}

#[cfg(feature = "test")]
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
    /// FVK. `seed_fingerprint` is the device's own ([`seed_fingerprint`]):
    /// with the consented account it is what any `zip32_derivation` on the
    /// wire must name.
    pub fn begin(
        &mut self,
        bytes: &[u8],
        fvk: &FullViewingKey,
        seed_fingerprint: &[u8; 32],
    ) -> Result<Review> {
        self.slot.clear();
        self.counter = self.counter.checked_add(1).ok_or(Error::state())?;
        let own = OwnDerivation::new(seed_fingerprint, self.policy.request);
        let Validated {
            pczt,
            projection,
            signing_indices,
            sighash,
            header,
            transparent_outputs,
            expected_ak,
        } = validate(bytes, &self.policy, fvk, &own)?;

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
                transparent_outputs,
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
                if effects::sighash(&pending.transparent_outputs, bundle, &pending.header)?
                    != pending.sighash
                {
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

/// Running totals of note and output values, bounded by the money range.
/// `Amount`, not `Capacity`: nothing here is about how big the transaction is.
fn add(total: u64, value: u64) -> Result<u64> {
    total
        .checked_add(value)
        .filter(|sum| *sum <= MAX_MONEY)
        .ok_or(Error::amount())
}

#[cfg(feature = "test")]
struct Validated {
    pczt: Pczt,
    projection: Projection,
    signing_indices: Vec<usize>,
    sighash: [u8; 32],
    header: wire::Header,
    transparent_outputs: Vec<TransparentOutput>,
    expected_ak: SpendValidatingKey,
}

#[cfg(feature = "test")]
fn validate(
    bytes: &[u8],
    policy: &Policy,
    fvk: &FullViewingKey,
    own: &OwnDerivation,
) -> Result<Validated> {
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
    // `wire::scan` already admitted the transparent bundle's shape; this
    // re-reads the same bytes through the deserializer to build the rows the
    // device shows and the outputs the digest hashes.
    let mut transparent_outputs = Vec::new();
    let mut transparent_total = 0;
    for (index, output) in pczt.transparent().outputs().iter().enumerate() {
        let (kind, hash) =
            stream::transparent_script(output.script_pubkey()).ok_or(Error::policy())?;
        let value = *output.value();
        // Twin of `stream::transparent_output`: a payment of nothing is not a
        // payment (§13).
        ensure_policy(value > 0)?;
        transparent_total = add(transparent_total, value)?;
        transparent_outputs.push(TransparentOutput {
            index,
            kind,
            hash,
            value,
        });
    }
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
        transparent_total,
        fee: 0,
        padding_outputs: 0,
        outputs: Vec::new(),
        transparent_outputs: transparent_outputs.clone(),
    };
    let mut signing_indices = Vec::new();
    let verifier = Verifier::new(pczt)
        .with_ironwood(|bundle| -> core::result::Result<(), OrchardError<Error>> {
            let digest = effects::sighash(&transparent_outputs, bundle, &header)
                .map_err(OrchardError::Custom)?;
            verify_bundle(
                bundle,
                fvk,
                policy,
                own,
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
        transparent_outputs,
        expected_ak: SpendValidatingKey::from(fvk.clone()),
    })
}

#[inline(never)]
#[allow(clippy::too_many_arguments)]
#[cfg(feature = "test")]
fn verify_bundle(
    bundle: &orchard::pczt::Bundle,
    fvk: &FullViewingKey,
    policy: &Policy,
    own: &OwnDerivation,
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
    // `fvk.scope_for_address` for dummy spends.
    let scope_classifier = fvk.scope_classifier();
    for (index, action) in bundle.actions().iter().enumerate() {
        let spend = action.spend();
        let output = action.output();
        let input_value = spend.value().ok_or(Error::malformed())?.inner();
        let output_value = output.value().ok_or(Error::malformed())?.inner();
        // A derivation claim must be the device's own: same seed
        // fingerprint, same consented account path, or `Policy`.
        for claim in [spend.zip32_derivation(), output.zip32_derivation()]
            .into_iter()
            .flatten()
        {
            own.admit(
                claim.seed_fingerprint(),
                claim.derivation_path().iter().map(|index| index.index()),
            )?;
        }
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
        // Capture the validated output note (cmx already checked
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
        let memo = verify_encryption(action, fvk, outgoing_scope, &note)?;
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
            memo,
        });
    }
    // At least one value-bearing output the user reviewed. A deshield whose
    // whole payment is transparent has no value-bearing shielded output at
    // all, and the transparent rows are what it shows instead.
    let reviewed_an_output =
        !projection.outputs.is_empty() || !projection.transparent_outputs.is_empty();
    ensure_policy(!signing_indices.is_empty() && reviewed_an_output)?;
    // The transparent outputs leave the shielded pool through the Ironwood
    // value sum, so they are spent value, not fee. This subtraction is the one
    // place a sign error silently mis-states the fee.
    let transparent_total = projection.transparent_total;
    projection.fee = projection
        .input_total
        .checked_sub(output_total)
        .and_then(|left| left.checked_sub(transparent_total))
        .ok_or(Error::malformed())?;
    ensure_malformed(
        i64::try_from(*bundle.value_sum()).ok()
            == Some(add(projection.fee, transparent_total)? as i64),
    )?;
    ensure_policy(projection.fee <= policy.limits.maximum_fee)?;
    ensure_malformed(
        add(
            add(
                add(projection.payment_total, projection.change_total)?,
                transparent_total,
            )?,
            projection.fee,
        )? == projection.input_total,
    )
}

/// Verifies the output's encryption under the device's own derivation and
/// returns what the device shows for its memo. The memo is recovered from
/// the `enc_ciphertext` that the digest hashes, so the displayed bytes are
/// the signed bytes. Memo policy (design §11): a padding output carries the
/// empty marker or the all-zero empty text; a change output (hidden) must
/// carry the empty marker; a payment may carry any memo, classified for
/// display by [`classify_memo`].
fn verify_encryption(
    action: &orchard::pczt::Action,
    fvk: &FullViewingKey,
    outgoing_scope: Scope,
    // The already-validated output note, whose commitment
    // `verify_note_commitment` checked equal to `action.output().cmx()`. Binding
    // is therefore self-contained: recovery is bound to the cmx-validated note,
    // not to a separately rebuilt one.
    note: &Note,
) -> Result<Memo> {
    let output = action.output();
    let domain = IronwoodDomain::for_pczt_action(action);
    // Device-local recovery bound to `note` by field comparison.
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
    let display = if note.value().inner() == 0 {
        ensure_policy(memo == EMPTY_MEMO || memo == PADDING_MEMO)?;
        Memo::Empty
    } else if outgoing_scope == Scope::Internal {
        ensure_policy(memo == EMPTY_MEMO)?;
        Memo::Empty
    } else {
        classify_memo(&memo)
    };
    let out_ciphertext = &output.encrypted_note().out_ciphertext;
    if let Some(ock) = output.ock() {
        ensure_malformed(
            orchard::note_encryption::recover_output_bound_with_ock(
                &domain,
                ock,
                action,
                out_ciphertext,
                note,
            )
            .is_some_and(|m| m == memo),
        )?;
    }
    if note.value().inner() > 0 {
        let recovers_under = |scope| {
            orchard::note_encryption::recover_output_bound_with_ovk(
                &domain,
                &fvk.to_ovk(scope),
                action,
                action.cv_net(),
                out_ciphertext,
                note,
            )
            .is_some_and(|m| m == memo)
        };
        match outgoing_scope {
            // A payment, to a foreign address or to one of the wallet's own
            // external addresses, must stay recoverable under the device's
            // external OVK so the wallet can later see what it sent. A host
            // that built it with `OvkPolicy::Discard` (`ovk=None`) is rejected.
            Scope::External => ensure_malformed(recovers_under(Scope::External))?,
            // Change. `outgoing_scope` is `Internal` only when the session's
            // own scope classifier placed the receiver among the device's
            // internal addresses, and the note itself is already bound by the
            // commitment check and the pk_d/esk recovery above, so the wallet
            // reaches it through its internal IVK and needs no OVK. The stock
            // SDK default `OvkPolicy::Sender` therefore encrypts change with no
            // OVK at all (`internal_ovk: None`), and that is admitted. A change
            // output the external OVK recovers instead was built under the
            // wrong scope and is still refused.
            Scope::Internal => {
                if !recovers_under(Scope::Internal) {
                    ensure_malformed(!recovers_under(Scope::External))?;
                }
            }
        }
    }
    // The standard Orchard builder creates zero-value padding with `ovk=None`,
    // so its `out_ciphertext` is random and cannot be recovered unless the PCZT
    // carries an OCK. We still validate note encryption above and OCK recovery
    // whenever present. The exact random ciphertext is included in the
    // transaction digest, bound into the approval token and retained PCZT, and
    // covered by the real spend signatures produced after approval. When the
    // paired spend is dummy, its pre-existing signature is additionally verified
    // above. Requiring OVK recovery here would reject standard padding.
    Ok(display)
}
