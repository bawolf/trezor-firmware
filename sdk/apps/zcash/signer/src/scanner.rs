//! Incremental scanner for the PCZT v2 encoding of an Ironwood-only
//! transaction. Each section (header, transparent output, shielded prefix,
//! action, trailer) is parsed from one fixed buffer, so at most one action is
//! held at a time.

use pasta_curves::group::ff::PrimeField;
use pasta_curves::pallas;
use zcash_note_encryption::{ENC_CIPHERTEXT_SIZE, OUT_CIPHERTEXT_SIZE};
use zcash_protocol::value::MAX_MONEY;
use zeroize::Zeroize;

use crate::error::{Error, Result, ensure};
use crate::limits::{
    MAX_ACTIONS, MAX_PCZT_BYTES, MAX_SCRIPT_PUBKEY_BYTES, MAX_TRANSPARENT_OUTPUTS,
    MAX_USER_ADDRESS_BYTES, P2PKH_SCRIPT_BYTES, P2SH_SCRIPT_BYTES,
};
use crate::{TransparentKind, ZIP32_HARDENED};

/// The address a P2PKH or P2SH `scriptPubKey` pays. Any other script is an
/// address the device cannot show, so it is refused.
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
pub(crate) const HEADER_BUDGET: usize =
    MAGIC.len() + 6 * VARINT + TAG + 1 + VARINT + TAG + 2 * VARINT;

/// Transparent output bytes: the value, the length-prefixed `scriptPubKey`,
/// the absent `redeem_script`, the `bip32_derivation` count, the optional
/// bounded `user_address` and the proprietary count.
pub(crate) const TRANSPARENT_OUTPUT_BUDGET: usize = VARINT
    + (VARINT + MAX_SCRIPT_PUBKEY_BYTES)
    + TAG
    + VARINT
    + (TAG + VARINT + MAX_USER_ADDRESS_BYTES)
    + VARINT;

/// Shielded-prefix bytes: an empty Sapling bundle (its tag, three zero counts
/// and two optional 32-byte fields), the absent Orchard tag, the Ironwood tag
/// and the action count.
pub(crate) const SHIELDED_BUDGET: usize = TAG + 3 * VARINT + 2 * (TAG + 32) + TAG + TAG + VARINT;

/// Action bytes: nine tagged 32-byte fields, the optional signature, two
/// recipients, two values, the FVK, two absent tags, two optional ZIP-32
/// derivations (fingerprint, count, three indices), two proprietary counts,
/// the ephemeral key, the ciphertext variant, both length-prefixed
/// ciphertexts, the optional OCK and the optional bounded `user_address`.
pub(crate) const ACTION_BUDGET: usize = 9 * (TAG + 32)
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
    + (TAG + VARINT + MAX_USER_ADDRESS_BYTES);

/// Trailer bytes: flags, value-sum magnitude and sign, the optional anchor,
/// the note version, the absent proof tag and the optional `bsk`.
pub(crate) const TRAILER_BUDGET: usize = 1 + VARINT + TAG + (TAG + 32) + VARINT + TAG + (TAG + 32);

/// The largest single section; the scanner's only buffer.
pub(crate) const SECTION_BUDGET: usize = ACTION_BUDGET;

// The buffer is part of the app's heap budget; a change must be deliberate.
const _: () = assert!(SECTION_BUDGET == 2015);

const _: () = assert!(
    HEADER_BUDGET <= SECTION_BUDGET
        && TRANSPARENT_OUTPUT_BUDGET <= SECTION_BUDGET
        && SHIELDED_BUDGET <= SECTION_BUDGET
        && TRAILER_BUDGET <= SECTION_BUDGET
);

/// Transaction-level fields and the transparent output count.
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

/// One transparent output, with the address its `scriptPubKey` pays.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransparentOutput<'a> {
    pub value: u64,
    pub kind: TransparentKind,
    pub hash: [u8; 20],
    /// Fed verbatim to the outputs hash; `kind` and `hash` are its solution.
    pub script_pubkey: &'a [u8],
}

/// The Ironwood action count.
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

/// A host's claim of the ZIP-32 derivation of a spend or an output: a seed
/// fingerprint and a three-index account path.
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
    /// The wallet's recipient string, unverified.
    pub user_address: Option<&'a str>,
}

/// Bundle fields after the actions. Yielded only once the last declared byte
/// has been consumed, so it also marks the end of input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Trailer<'a> {
    /// Checked by the caller with orchard's `Flags`.
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
    ensure(ok, Error::Malformed)
}

fn policy(ok: bool) -> Result<()> {
    ensure(ok, Error::Policy)
}

/// Refuses a field element encoding at or above the modulus. librustzcash
/// parses the Sapling anchor and both binding keys this way, although only the
/// host uses them; the Ironwood anchor is checked in `Session::review_stream`.
fn canonical<F: PrimeField<Repr = [u8; 32]>>(bytes: &[u8; 32]) -> Result<()> {
    malformed(F::from_repr(*bytes).is_some().into())
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

    /// `Option<sapling::Bundle>`: absent, or present and empty.
    /// librustzcash's signer redaction keeps an empty Sapling bundle's anchor
    /// and `bsk`, which must be canonical. The value sum is a zigzag `i128`
    /// whose zero is the one-byte encoding.
    fn empty_sapling(&mut self) -> Parse<()> {
        if !self.tag()? {
            return Ok(());
        }
        policy(self.varint()? == 0)?; // spends
        policy(self.varint()? == 0)?; // outputs
        policy(self.varint()? == 0)?; // value_sum
        if let Some(anchor) = self.optional::<32>()? {
            canonical::<jubjub::Base>(anchor)?;
        }
        if let Some(bsk) = self.optional::<32>()? {
            canonical::<jubjub::Fr>(bsk)?;
        }
        Ok(())
    }

    /// `Option<Zip32Derivation>`: the fingerprint, then a path of the three
    /// hardened indices an account path has. Any other length cannot be the
    /// device's own, so it is refused before the path is read.
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
    /// wallets set on payment outputs. It is not covered by the sighash.
    fn user_address(&mut self) -> Parse<Option<&'a str>> {
        if !self.tag()? {
            return Ok(None);
        }
        let length = self.varint()?;
        policy(length <= MAX_USER_ADDRESS_BYTES as u64)?;
        let bytes = self.slice(length as usize)?;
        let text = core::str::from_utf8(bytes).map_err(|_| Error::Malformed)?;
        Ok(Some(text))
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

/// `Option<transparent::Bundle>`: absent, or present with no inputs and at
/// least one output. Returns the output count; each output is its own section.
///
/// The device cannot sign transparent inputs, and without them ZIP-244 S.2
/// reduces to T.2. An empty bundle is encoded as absent, so a present one
/// with no outputs is refused.
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
/// `proprietary` must be empty; `user_address` is ignored, since the address
/// shown is solved from `scriptPubKey`.
fn transparent_output<'a>(r: &mut Reader<'a>) -> Parse<TransparentOutput<'a>> {
    let value = r.varint()?;
    malformed(value <= MAX_MONEY)?;
    // A zero-value payment is refused.
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
/// Ironwood action count, which shares [`MAX_ACTIONS`] with the transparent
/// outputs.
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
    let user_address = r.user_address()?;
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
            user_address,
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
    if let Some(bsk) = r.optional::<32>()? {
        canonical::<pallas::Scalar>(bsk)?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reader(bytes: &[u8]) -> Reader<'_> {
        Reader {
            rest: bytes,
            position: 0,
        }
    }

    fn varint(bytes: &[u8]) -> core::result::Result<u64, Option<Error>> {
        reader(bytes).varint().map_err(|halt| match halt {
            Halt::More(_) => None,
            Halt::Fail(error) => Some(error),
        })
    }

    #[test]
    fn test_varint() {
        assert_eq!(varint(&[0]), Ok(0));
        assert_eq!(varint(&[0x7f]), Ok(127));
        assert_eq!(varint(&[0x80, 0x01]), Ok(128));
        assert_eq!(
            varint(&[0xff; 9].iter().copied().chain([1]).collect::<Vec<_>>()),
            Ok(u64::MAX)
        );
        // Not the shortest encoding.
        assert_eq!(varint(&[0x80, 0x00]), Err(Some(Error::Malformed)));
        // Above u64::MAX.
        let over: Vec<u8> = [0xff; 9].iter().copied().chain([2]).collect();
        assert_eq!(varint(&over), Err(Some(Error::Malformed)));
        assert_eq!(varint(&[0x80]), Err(None));
    }

    #[test]
    fn test_tag() {
        assert!(matches!(reader(&[0]).tag(), Ok(false)));
        assert!(matches!(reader(&[1]).tag(), Ok(true)));
        assert!(matches!(
            reader(&[2]).tag(),
            Err(Halt::Fail(Error::Malformed))
        ));
    }

    #[test]
    fn test_transparent_script() {
        let hash = [7; 20];
        let mut p2pkh = vec![0x76, 0xa9, 0x14];
        p2pkh.extend_from_slice(&hash);
        p2pkh.extend_from_slice(&[0x88, 0xac]);
        let mut p2sh = vec![0xa9, 0x14];
        p2sh.extend_from_slice(&hash);
        p2sh.push(0x87);
        assert_eq!(
            transparent_script(&p2pkh),
            Some((TransparentKind::P2pkh, hash))
        );
        assert_eq!(
            transparent_script(&p2sh),
            Some((TransparentKind::P2sh, hash))
        );
        assert_eq!(transparent_script(&p2pkh[..24]), None);
        assert_eq!(transparent_script(&[0x6a, 0x01, 0x00]), None);
    }

    #[test]
    fn test_declared_length() {
        assert!(matches!(
            Scanner::new(MAX_PCZT_BYTES + 1),
            Err(Error::Capacity)
        ));
        assert!(matches!(
            Scanner::new(MAGIC.len() - 1),
            Err(Error::Malformed)
        ));
        let mut scanner = Scanner::new(MAGIC.len()).unwrap();
        assert!(matches!(scanner.feed(&[0; 9]), Err(Error::State)));
        // Any error is final.
        assert!(matches!(scanner.feed(&[]), Err(Error::State)));
    }

    #[test]
    fn test_magic() {
        let mut scanner = Scanner::new(HEADER_BUDGET).unwrap();
        let mut bytes = [0; HEADER_BUDGET];
        bytes[..8].copy_from_slice(b"PCZT\x01\0\0\0");
        assert!(matches!(scanner.feed(&bytes), Err(Error::Malformed)));
    }
}
