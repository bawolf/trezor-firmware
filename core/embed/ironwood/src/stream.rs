//! Incremental, chunk-fed scanner for the exact v2 Ironwood-only PCZT grammar
//! that `wire::scan` admits. The encoding is
//! consumed one section at a time (header, each transparent output, the
//! shielded prefix, each action, trailer) in a fixed
//! buffer, so the device never holds more than one action's bytes; see
//! docs/common/zcash-ironwood-signing.md §3-4.
//!
//! The field grammar mirrors `wire::scan` line for line. Agreement on both
//! acceptance and error class for every corpus PCZT, mutation, truncation and
//! chunking is proven by tests/stream_equivalence.rs. Each section budget
//! bounds what `wire::scan`'s reader can consume before it decides, so the
//! fixed buffer never changes a verdict.

use zcash_note_encryption::{ENC_CIPHERTEXT_SIZE, OUT_CIPHERTEXT_SIZE};
use zcash_protocol::value::MAX_MONEY;
use zeroize::Zeroize;

use crate::{
    Error, MAX_ACTIONS, MAX_PCZT_BYTES, MAX_SCRIPT_PUBKEY_BYTES, MAX_TRANSPARENT_OUTPUTS,
    P2PKH_SCRIPT_BYTES, P2SH_SCRIPT_BYTES, Result, TransparentKind, USER_ADDRESS_BUDGET,
    ZIP32_HARDENED,
};

/// The transparent address a `scriptPubKey` pays, or `None` if the device
/// cannot render one.
///
/// Only the two standard shapes are admitted, because an unrecognised script
/// is an address the device cannot display and therefore cannot obtain
/// consent for. This is
/// `zcash_transparent::address::TransparentAddress::from_script_pubkey`
/// restricted to those two `solver::ScriptKind`s, without the script parser:
/// the bytes are fully determined by the 20-byte hash. `wire.rs` checks the
/// same two shapes.
pub(crate) fn transparent_script(script: &[u8]) -> Option<(TransparentKind, [u8; 20])> {
    match script {
        [0x76, 0xa9, 0x14, hash @ .., 0x88, 0xac] => {
            Some((TransparentKind::P2pkh, hash.try_into().ok()?))
        }
        [0xa9, 0x14, hash @ .., 0x87] => Some((TransparentKind::P2sh, hash.try_into().ok()?)),
        _ => None,
    }
}

const MAGIC: [u8; 8] = *b"PCZT\x02\0\0\0";
/// Longest encoding the canonical varint reader consumes before deciding.
const VARINT: usize = 10;
const TAG: usize = 1;

/// Header bytes: magic, six `u32` varints (one behind the lock-time tag),
/// `tx_modifiable`, the proprietary count, and the transparent bundle's tag
/// with its input and output vector lengths. The bundle's outputs are their
/// own sections, so the header stops at the output count.
pub const HEADER_BUDGET: usize = MAGIC.len() + 6 * VARINT + TAG + 1 + VARINT + TAG + 2 * VARINT;

/// Transparent output bytes: the value, the length-prefixed `scriptPubKey`,
/// the absent `redeem_script`, the `bip32_derivation` count, the optional
/// bounded `user_address` and the proprietary count.
pub const TRANSPARENT_OUTPUT_BUDGET: usize = VARINT
    + (VARINT + MAX_SCRIPT_PUBKEY_BYTES)
    + TAG
    + VARINT
    + (TAG + VARINT + USER_ADDRESS_BUDGET)
    + VARINT;

/// Shielded-prefix bytes: an empty Sapling bundle (its tag, three zero counts
/// and two optional 32-byte fields), the absent Orchard tag, the Ironwood tag
/// and the action count.
pub const SHIELDED_BUDGET: usize = TAG + 3 * VARINT + 2 * (TAG + 32) + TAG + TAG + VARINT;

/// Action bytes: nine tagged 32-byte fields, the optional signature, two
/// recipients, two values, the FVK, two absent tags, two optional ZIP-32
/// derivations (fingerprint, count, three indices), two proprietary counts,
/// the ephemeral key, the ciphertext variant, both length-prefixed
/// ciphertexts, the optional OCK and the optional bounded `user_address`.
pub const ACTION_BUDGET: usize = 9 * (TAG + 32)
    + (TAG + 64)
    + 2 * (TAG + 43)
    + 2 * (TAG + VARINT)
    + (TAG + 96)
    + 2 * TAG
    + 2 * (TAG + 32 + 4 * VARINT)
    + 2 * VARINT
    + 32
    + VARINT
    + (VARINT + ENC_CIPHERTEXT_SIZE)
    + (VARINT + OUT_CIPHERTEXT_SIZE)
    + (TAG + 32)
    + (TAG + VARINT + USER_ADDRESS_BUDGET);

/// Trailer bytes: flags, value-sum magnitude and sign, the optional anchor,
/// the note version, the absent proof tag and the optional `bsk`.
pub const TRAILER_BUDGET: usize = 1 + VARINT + TAG + (TAG + 32) + VARINT + TAG + (TAG + 32);

/// The largest single section; the scanner's only buffer.
pub const SECTION_BUDGET: usize = ACTION_BUDGET;

// Tripwire. The three budgets above are derived from named field sizes, so
// they are already correct by construction; these literals exist to make any
// grammar change surface as an explicit diff, because `SECTION_BUDGET` is the
// scanner's single buffer and therefore a line item in the region's RAM
// budget. A failure here is not a bug by itself: re-derive the number from
// the field list in the doc comment above, check the new `SECTION_BUDGET`
// against the region, and update the literal in the same commit.
const _: () = assert!(
    HEADER_BUDGET == 101
        && TRANSPARENT_OUTPUT_BUDGET == 589
        && SHIELDED_BUDGET == 109
        && ACTION_BUDGET == 2015
        && TRAILER_BUDGET == 89
);
const _: () = assert!(
    HEADER_BUDGET <= SECTION_BUDGET
        && TRANSPARENT_OUTPUT_BUDGET <= SECTION_BUDGET
        && SHIELDED_BUDGET <= SECTION_BUDGET
        && TRAILER_BUDGET <= SECTION_BUDGET
);

/// The global fields `wire::scan` returns plus the admitted transparent
/// output count. The Ironwood action count is not here: the encoding puts it
/// after the transparent bundle, so it arrives with [`Shielded`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u32,
    pub group: u32,
    pub branch: u32,
    pub lock_time: u32,
    pub expiry: u32,
    pub coin_type: u32,
    pub transparent_outputs: usize,
}

/// One transparent output: the value and the `scriptPubKey` the digest hashes
/// (ZIP-244 T.2c), with the address the device renders already solved out of
/// the script.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransparentOutput<'a> {
    pub value: u64,
    pub kind: TransparentKind,
    pub hash: [u8; 20],
    /// Fed verbatim to the outputs hash; `kind` and `hash` are its solution.
    pub script_pubkey: &'a [u8],
}

/// The Ironwood bundle's declared action count, read after the transparent
/// bundle because that is where the encoding puts it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Shielded {
    pub actions: usize,
}

/// One action's fields, borrowed from the scanner until the next feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action<'a> {
    pub cv_net: &'a [u8; 32],
    pub spend: Spend<'a>,
    pub output: Output<'a>,
    pub rcv: &'a [u8; 32],
}

/// A host's claim of the ZIP-32 derivation behind a spend or an output:
/// the seed fingerprint and an account-level path of three hardened indices,
/// the only shape the device's own `m/32'/coin_type'/account'` can take.
/// Checked against the session's own derivation by the caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Zip32Derivation<'a> {
    pub seed_fingerprint: &'a [u8; 32],
    pub path: [u32; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spend<'a> {
    pub nullifier: &'a [u8; 32],
    pub rk: &'a [u8; 32],
    /// Checked against the spend value and the sighash by the caller.
    pub spend_auth_sig: Option<&'a [u8; 64]>,
    pub recipient: &'a [u8; 43],
    pub value: u64,
    pub rho: &'a [u8; 32],
    pub rseed: &'a [u8; 32],
    pub fvk: &'a [u8; 96],
    pub alpha: &'a [u8; 32],
    pub zip32_derivation: Option<Zip32Derivation<'a>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Output<'a> {
    pub cmx: &'a [u8; 32],
    pub ephemeral_key: &'a [u8; 32],
    pub enc_ciphertext: &'a [u8; ENC_CIPHERTEXT_SIZE],
    pub out_ciphertext: &'a [u8; OUT_CIPHERTEXT_SIZE],
    pub recipient: &'a [u8; 43],
    pub value: u64,
    pub rseed: &'a [u8; 32],
    pub ock: Option<&'a [u8; 32]>,
    pub zip32_derivation: Option<Zip32Derivation<'a>>,
}

/// Bundle fields after the actions. Yielded only once the last declared byte
/// has been consumed, so it also marks the end of input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trailer<'a> {
    /// Checked using upstream's versioned flag implementation by the caller.
    pub flags: u8,
    pub value_sum: u64,
    /// Allowed to be deferred in v6.
    pub anchor: Option<&'a [u8; 32]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item<'a> {
    Header(Header),
    TransparentOutput(TransparentOutput<'a>),
    Shielded(Shielded),
    Action(Action<'a>),
    Trailer(Trailer<'a>),
}

fn malformed(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::Malformed) }
}

fn policy(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::Policy) }
}

/// Why a section parse stopped: it needs the buffer to hold at least this
/// many bytes, or it reached a verdict.
enum Halt {
    More(usize),
    Fail(Error),
}

impl From<Error> for Halt {
    fn from(error: Error) -> Self {
        Self::Fail(error)
    }
}

type Parse<T> = core::result::Result<T, Halt>;

struct Reader<'a> {
    rest: &'a [u8],
    position: usize,
}

impl<'a> Reader<'a> {
    fn fixed<const N: usize>(&mut self) -> Parse<&'a [u8; N]> {
        let (bytes, rest) = self
            .rest
            .split_first_chunk::<N>()
            .ok_or(Halt::More(self.position + N))?;
        self.rest = rest;
        self.position += N;
        Ok(bytes)
    }

    fn slice(&mut self, n: usize) -> Parse<&'a [u8]> {
        let (bytes, rest) = self
            .rest
            .split_at_checked(n)
            .ok_or(Halt::More(self.position + n))?;
        self.rest = rest;
        self.position += n;
        Ok(bytes)
    }

    fn byte(&mut self) -> Parse<u8> {
        Ok(self.fixed::<1>()?[0])
    }

    fn tag(&mut self) -> Parse<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::Malformed.into()),
        }
    }

    fn varint(&mut self) -> Parse<u64> {
        let mut value = 0u64;
        for i in 0..VARINT {
            let b = self.byte()?;
            malformed(i < 9 || b <= 1)?;
            value |= u64::from(b & 127) << (i * 7);
            if b & 128 == 0 {
                malformed(i == 0 || b != 0)?;
                return Ok(value);
            }
        }
        Err(Error::Malformed.into())
    }

    fn u32(&mut self) -> Parse<u32> {
        Ok(self.varint()?.try_into().map_err(|_| Error::Malformed)?)
    }

    fn absent(&mut self) -> Parse<()> {
        Ok(policy(!self.tag()?)?)
    }

    fn required<const N: usize>(&mut self) -> Parse<&'a [u8; N]> {
        malformed(self.tag()?)?;
        self.fixed()
    }

    fn optional<const N: usize>(&mut self) -> Parse<Option<&'a [u8; N]>> {
        if self.tag()? {
            Ok(Some(self.fixed()?))
        } else {
            Ok(None)
        }
    }

    fn empty_map(&mut self) -> Parse<()> {
        Ok(policy(self.varint()? == 0)?)
    }

    fn value(&mut self) -> Parse<u64> {
        malformed(self.tag()?)?;
        let value = self.varint()?;
        malformed(value <= MAX_MONEY)?;
        Ok(value)
    }

    fn bytes<const N: usize>(&mut self) -> Parse<&'a [u8; N]> {
        malformed(self.varint()? == N as u64)?;
        self.fixed()
    }

    /// `Option<sapling::Bundle>`: absent, or present and empty. The stock SDK
    /// signer view (`redact_pczt_for_signer(Full)`) keeps the empty Sapling
    /// bundle's anchor and `bsk`, which makes v2 encode the bundle present.
    /// Spends, outputs and a nonzero value sum stay refused; the anchor and
    /// `bsk` are skipped. The value sum is a zigzag `i128` whose zero is the
    /// one-byte encoding; the `u64` reader's ten-byte limit is reached only
    /// by values this refuses anyway.
    fn empty_sapling(&mut self) -> Parse<()> {
        if !self.tag()? {
            return Ok(());
        }
        policy(self.varint()? == 0)?; // spends
        policy(self.varint()? == 0)?; // outputs
        policy(self.varint()? == 0)?; // value_sum
        self.optional::<32>()?; // anchor
        self.optional::<32>()?; // bsk
        Ok(())
    }

    /// `Option<Zip32Derivation>`: the fingerprint, then a path whose length
    /// must be three, the only length the device's own derivation can have.
    /// Any other length can never match and is refused before the path is
    /// read, so the budget stays fixed; a non-hardened index is what
    /// `orchard::pczt::Zip32Derivation::parse` refuses, hence `Malformed`.
    fn zip32_derivation(&mut self) -> Parse<Option<Zip32Derivation<'a>>> {
        if !self.tag()? {
            return Ok(None);
        }
        let seed_fingerprint = self.fixed::<32>()?;
        policy(self.varint()? == 3)?;
        let mut path = [0; 3];
        for index in &mut path {
            *index = self.u32()?;
            malformed(*index & ZIP32_HARDENED != 0)?;
        }
        Ok(Some(Zip32Derivation {
            seed_fingerprint,
            path,
        }))
    }

    /// `Option<String>`: the recipient string the wallet showed its user, which
    /// the stock SDK sets on every payment output. Untrusted metadata outside
    /// the sighash: admitted within [`USER_ADDRESS_BUDGET`], required to be
    /// UTF-8 as postcard's `String` is, and not used. The device shows the
    /// receiver it verified, never this string.
    fn user_address(&mut self) -> Parse<()> {
        if !self.tag()? {
            return Ok(());
        }
        let length = self.varint()?;
        policy(length <= USER_ADDRESS_BUDGET as u64)?;
        let bytes = self.slice(length as usize)?;
        malformed(core::str::from_utf8(bytes).is_ok())?;
        Ok(())
    }
}

fn header(r: &mut Reader<'_>) -> Parse<Header> {
    malformed(*r.fixed::<8>()? == MAGIC)?;
    let version = r.u32()?;
    let group = r.u32()?;
    let branch = r.u32()?;
    let lock_time = if r.tag()? { r.u32()? } else { 0 };
    let expiry = r.u32()?;
    let coin_type = r.u32()?;
    policy(r.byte()? == 0)?; // tx_modifiable
    r.empty_map()?; // global proprietary
    let transparent_outputs = transparent(r)?;
    Ok(Header {
        version,
        group,
        branch,
        lock_time,
        expiry,
        coin_type,
        transparent_outputs,
    })
}

/// `Option<transparent::Bundle>`: absent, or present with NO inputs and at
/// least one output. Returns the declared output count; each output is its
/// own section.
///
/// Zero inputs is what keeps the whole transparent digest to one running hash:
/// ZIP-244 §S.2 makes `transparent_sig_digest` identical to the txid node §T.2
/// for a transaction with no transparent inputs, so none of the S.2
/// amounts/scripts/txin sub-hashes and no `hash_type` byte exist. An input is
/// therefore `Policy`, not a parse failure: the device cannot authorize a
/// transparent spend at all (no BIP-44 keychain, no script solving), and a
/// zero-length input vector is the structural statement of that.
///
/// A present bundle with no outputs is refused too: the v2 encoder writes the
/// empty bundle as an absent tag, so it would be a second encoding of the same
/// transaction.
fn transparent(r: &mut Reader<'_>) -> Parse<usize> {
    if !r.tag()? {
        return Ok(0);
    }
    policy(r.varint()? == 0)?; // inputs
    let outputs = r.varint()?;
    policy(outputs > 0)?;
    if outputs > MAX_TRANSPARENT_OUTPUTS as u64 {
        return Err(Error::Capacity.into());
    }
    Ok(outputs as usize)
}

/// One `transparent::Output`. `redeem_script`, `bip32_derivation` and
/// `proprietary` are refused because a device that cannot spend a transparent
/// output has no use for them and must not be asked to display what it did not
/// check; `user_address` is admitted, bounded and ignored, the same rule the
/// Ironwood output has.
fn transparent_output<'a>(r: &mut Reader<'a>) -> Parse<TransparentOutput<'a>> {
    let value = r.varint()?;
    malformed(value <= MAX_MONEY)?;
    // Every transparent output here is a payment (§13), and a payment of
    // nothing is not one: it would put "0 ZEC" and a public address in front of
    // the user, and satisfy "at least one value-bearing output the user
    // reviewed" without bearing any value.
    policy(value > 0)?;
    let length = r.varint()?;
    policy(length == P2PKH_SCRIPT_BYTES as u64 || length == P2SH_SCRIPT_BYTES as u64)?;
    let script_pubkey = r.slice(length as usize)?;
    let (kind, hash) = transparent_script(script_pubkey).ok_or(Error::Policy)?;
    r.absent()?; // redeem_script
    r.empty_map()?; // bip32_derivation
    r.user_address()?;
    r.empty_map()?; // proprietary
    Ok(TransparentOutput {
        value,
        kind,
        hash,
        script_pubkey,
    })
}

/// The pools between the transparent bundle and the actions, ending at the
/// Ironwood action count. `transparent_outputs` is already admitted, and the
/// two share one ZIP-317 logical-action cap (`MAX_TRANSPARENT_OUTPUTS`).
fn shielded(r: &mut Reader<'_>, transparent_outputs: usize) -> Parse<Shielded> {
    r.empty_sapling()?;
    r.absent()?; // orchard
    policy(r.tag()?)?; // ironwood
    let actions = r.varint()?;
    if actions == 0 {
        return Err(Error::Policy.into());
    }
    if actions > (MAX_ACTIONS - transparent_outputs) as u64 {
        return Err(Error::Capacity.into());
    }
    Ok(Shielded {
        actions: actions as usize,
    })
}

fn action<'a>(r: &mut Reader<'a>) -> Parse<Action<'a>> {
    let cv_net = r.required::<32>()?;

    let nullifier = r.required::<32>()?;
    let rk = r.required::<32>()?;
    let spend_auth_sig = r.optional::<64>()?;
    let spend_recipient = r.required::<43>()?;
    let spend_value = r.value()?;
    let rho = r.required::<32>()?;
    let spend_rseed = r.required::<32>()?;
    let fvk = r.required::<96>()?;
    r.absent()?; // witness
    let alpha = r.required::<32>()?;
    let spend_zip32_derivation = r.zip32_derivation()?;
    r.absent()?; // dummy_sk
    r.empty_map()?; // proprietary

    let cmx = r.required::<32>()?;
    let ephemeral_key = r.fixed::<32>()?;
    policy(r.varint()? == 0)?; // enc_ciphertext is the Encrypted variant
    let enc_ciphertext = r.bytes::<ENC_CIPHERTEXT_SIZE>()?;
    let out_ciphertext = r.bytes::<OUT_CIPHERTEXT_SIZE>()?;
    let output_recipient = r.required::<43>()?;
    let output_value = r.value()?;
    let output_rseed = r.required::<32>()?;
    let ock = r.optional::<32>()?;
    let output_zip32_derivation = r.zip32_derivation()?;
    r.user_address()?;
    r.empty_map()?; // proprietary

    let rcv = r.required::<32>()?;
    Ok(Action {
        cv_net,
        spend: Spend {
            nullifier,
            rk,
            spend_auth_sig,
            recipient: spend_recipient,
            value: spend_value,
            rho,
            rseed: spend_rseed,
            fvk,
            alpha,
            zip32_derivation: spend_zip32_derivation,
        },
        output: Output {
            cmx,
            ephemeral_key,
            enc_ciphertext,
            out_ciphertext,
            recipient: output_recipient,
            value: output_value,
            rseed: output_rseed,
            ock,
            zip32_derivation: output_zip32_derivation,
        },
        rcv,
    })
}

fn trailer<'a>(r: &mut Reader<'a>) -> Parse<Trailer<'a>> {
    let flags = r.byte()?;
    let value_sum = r.varint()?;
    malformed(value_sum <= MAX_MONEY)?;
    policy(!r.tag()?)?; // value sum is non-negative
    let anchor = r.optional::<32>()?;
    policy(r.varint()? == 1)?; // note_version V3
    r.absent()?; // zkproof
    // The host's binding signing key, kept by the Full signer view. The
    // device never needs it: the host computes the binding signature.
    r.optional::<32>()?; // bsk
    Ok(Trailer {
        flags,
        value_sum,
        anchor,
    })
}

#[derive(Clone, Copy)]
enum Section {
    Header,
    Transparent { next: usize, count: usize },
    Shielded { transparent_outputs: usize },
    Action { next: usize, count: usize },
    Trailer,
}

impl Section {
    const fn budget(self) -> usize {
        match self {
            Self::Header => HEADER_BUDGET,
            Self::Transparent { .. } => TRANSPARENT_OUTPUT_BUDGET,
            Self::Shielded { .. } => SHIELDED_BUDGET,
            Self::Action { .. } => ACTION_BUDGET,
            Self::Trailer => TRAILER_BUDGET,
        }
    }

    /// What follows a header that declared `count` transparent outputs.
    const fn after_transparent_count(count: usize) -> Self {
        if count == 0 {
            Self::Shielded {
                transparent_outputs: 0,
            }
        } else {
            Self::Transparent { next: 0, count }
        }
    }
}

#[derive(Clone, Copy)]
enum State {
    Scan(Section),
    Done,
    Failed,
}

/// Consumes a PCZT of declared length in arbitrary chunks and yields one
/// section at a time. Any error is final.
pub struct Scanner {
    total: usize,
    /// Stream offset where the current section begins.
    start: usize,
    /// Bytes of the current section buffered so far.
    len: usize,
    /// Buffered length needed before the section is parsed again.
    wanted: usize,
    state: State,
    buffer: [u8; SECTION_BUDGET],
}

impl Drop for Scanner {
    fn drop(&mut self) {
        self.buffer.zeroize();
    }
}

impl Scanner {
    /// `total` is the transport's declared PCZT length: the capacity bound and
    /// the exact point where the grammar must end.
    pub fn new(total: usize) -> Result<Self> {
        if total > MAX_PCZT_BYTES {
            return Err(Error::Capacity);
        }
        malformed(total >= MAGIC.len())?;
        Ok(Self {
            total,
            start: 0,
            len: 0,
            wanted: 0,
            state: State::Scan(Section::Header),
            buffer: [0; SECTION_BUDGET],
        })
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.state, State::Done)
    }

    /// Feeds the next bytes. Returns how many were consumed and the section
    /// they completed, if any; bytes not consumed belong to the next section
    /// and must be fed again. A completed section borrows the scanner until
    /// the next call.
    pub fn feed(&mut self, chunk: &[u8]) -> Result<(usize, Option<Item<'_>>)> {
        let section = match self.state {
            State::Scan(section) => section,
            State::Done if chunk.is_empty() => return Ok((0, None)),
            State::Done | State::Failed => return Err(Error::State),
        };
        if self.start + self.len + chunk.len() > self.total {
            self.state = State::Failed;
            return Err(Error::State);
        }
        let budget = section.budget();
        let copy = chunk.len().min(budget - self.len);
        self.buffer[self.len..self.len + copy].copy_from_slice(&chunk[..copy]);
        let old = self.len;
        self.len += copy;
        let ended = self.start + self.len == self.total;
        if self.len < self.wanted && !ended {
            return Ok((copy, None));
        }

        let mut reader = Reader {
            rest: &self.buffer[..self.len],
            position: 0,
        };
        let parsed = match section {
            Section::Header => header(&mut reader).map(|header| {
                let next = Section::after_transparent_count(header.transparent_outputs);
                (Item::Header(header), State::Scan(next))
            }),
            Section::Transparent { next, count } => transparent_output(&mut reader).map(|output| {
                let next = if next + 1 == count {
                    Section::Shielded {
                        transparent_outputs: count,
                    }
                } else {
                    Section::Transparent {
                        next: next + 1,
                        count,
                    }
                };
                (Item::TransparentOutput(output), State::Scan(next))
            }),
            Section::Shielded {
                transparent_outputs,
            } => shielded(&mut reader, transparent_outputs).map(|shielded| {
                let next = Section::Action {
                    next: 0,
                    count: shielded.actions,
                };
                (Item::Shielded(shielded), State::Scan(next))
            }),
            Section::Action { next, count } => action(&mut reader).map(|action| {
                let next = if next + 1 == count {
                    Section::Trailer
                } else {
                    Section::Action {
                        next: next + 1,
                        count,
                    }
                };
                (Item::Action(action), State::Scan(next))
            }),
            Section::Trailer => {
                trailer(&mut reader).map(|trailer| (Item::Trailer(trailer), State::Done))
            }
        };
        match parsed {
            Ok((item, next)) => {
                let used = reader.position;
                self.start += used;
                self.len = 0;
                self.wanted = 0;
                // The grammar and the declared length must end together:
                // otherwise the input is truncated or has trailing bytes.
                if (self.start == self.total) != matches!(next, State::Done) {
                    self.state = State::Failed;
                    return Err(Error::Malformed);
                }
                self.state = next;
                Ok((used - old, Some(item)))
            }
            Err(Halt::More(needed)) if !ended && needed <= budget => {
                self.wanted = needed;
                Ok((copy, None))
            }
            Err(Halt::More(_)) => {
                self.state = State::Failed;
                self.len = 0;
                Err(Error::Malformed)
            }
            Err(Halt::Fail(error)) => {
                self.state = State::Failed;
                self.len = 0;
                Err(error)
            }
        }
    }
}
