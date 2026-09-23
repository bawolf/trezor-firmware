//! Nonallocating admission for the pinned v2 Postcard layout. Not a general
//! PCZT parser.

#[cfg(feature = "test")]
use zcash_note_encryption::{ENC_CIPHERTEXT_SIZE, OUT_CIPHERTEXT_SIZE};
#[cfg(feature = "test")]
use zcash_protocol::value::MAX_MONEY;

#[cfg(feature = "test")]
use crate::{Error, Result, ZIP32_HARDENED};

pub const MAX_PCZT_BYTES: usize = 65_536;
// Bundles of up to 32 actions are admitted. The cross-action state is all
// fixed-capacity `[T; MAX_ACTIONS]` (records ~98 B, nullifiers 32 B, outputs
// 64 B, signature records 66 B per action), so the retained base is ~+5 KB at
// this cap and stays O(1) in N; peak RAM is dominated by the single live
// action's transient working set, not this cap.
pub const MAX_ACTIONS: usize = 32;
/// Largest admitted transparent output count.
///
/// ZIP-317 counts transparent outputs as logical actions alongside the
/// shielded ones -- `logical_actions = max(ceil(tx_in_total_size / 150),
/// ceil(tx_out_total_size / 34)) + ... + orchard_actions`, and a standard
/// 34-byte transparent output is therefore exactly one of them. The device
/// keeps the same arithmetic and the same single cap: transparent outputs and
/// Ironwood actions share [`MAX_ACTIONS`], so what the 32 bounds is what the
/// fee rule charges for, not two unrelated numbers. A transparent-only
/// transaction is not an Ironwood signing request (§4 requires a real
/// Ironwood spend), so at least one action is always taken.
pub const MAX_TRANSPARENT_OUTPUTS: usize = MAX_ACTIONS - 1;
/// Longest `output.user_address` admitted, in bytes. The stock SDK stamps the
/// ZIP-321 recipient string on every payment output; a unified address with
/// every receiver type is about 213 characters, so this bounds the scanner's
/// section buffer without refusing any real address.
pub const USER_ADDRESS_BUDGET: usize = 512;

/// The `hash160` both admitted `scriptPubKey` shapes carry, and the only part
/// of either that is not a fixed opcode.
pub const HASH160_BYTES: usize = 20;
/// Serialized length of the P2PKH `scriptPubKey`
/// `OP_DUP OP_HASH160 <20> OP_EQUALVERIFY OP_CHECKSIG`.
pub const P2PKH_SCRIPT_BYTES: usize = 25;
/// Serialized length of the P2SH `scriptPubKey` `OP_HASH160 <20> OP_EQUAL`.
pub const P2SH_SCRIPT_BYTES: usize = 23;
/// Longest `scriptPubKey` admitted, in bytes.
pub const MAX_SCRIPT_PUBKEY_BYTES: usize = P2PKH_SCRIPT_BYTES;

// `version`, `group` and `coin_type` are read only by the host-only reference
// scanner below; the device checks the same fields on `stream::Header`.
#[cfg_attr(not(feature = "test"), allow(dead_code))]
#[derive(Clone, Copy)]
pub(crate) struct Header {
    pub version: u32,
    pub group: u32,
    pub branch: u32,
    pub lock_time: u32,
    pub expiry: u32,
    pub coin_type: u32,
}

#[cfg(feature = "test")]
fn malformed(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::malformed()) }
}

#[cfg(feature = "test")]
fn policy(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::policy()) }
}

#[cfg(feature = "test")]
struct Reader<'a> {
    rest: &'a [u8],
}

#[cfg(feature = "test")]
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let bytes = self.rest.get(..n).ok_or(Error::malformed())?;
        self.rest = &self.rest[n..];
        Ok(bytes)
    }

    fn byte(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn tag(&mut self) -> Result<bool> {
        match self.byte()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(Error::malformed()),
        }
    }

    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for i in 0..10 {
            let b = self.byte()?;
            malformed(i < 9 || b <= 1)?;
            value |= u64::from(b & 127) << (i * 7);
            if b & 128 == 0 {
                malformed(i == 0 || b != 0)?;
                return Ok(value);
            }
        }
        Err(Error::malformed())
    }

    fn u32(&mut self) -> Result<u32> {
        self.varint()?.try_into().map_err(|_| Error::malformed())
    }

    fn absent(&mut self) -> Result<()> {
        policy(!self.tag()?)
    }

    fn required(&mut self, n: usize) -> Result<()> {
        malformed(self.tag()?)?;
        self.take(n)?;
        Ok(())
    }

    fn optional(&mut self, n: usize) -> Result<()> {
        if self.tag()? {
            self.take(n)?;
        }
        Ok(())
    }

    fn empty_map(&mut self) -> Result<()> {
        policy(self.varint()? == 0)
    }

    fn value(&mut self) -> Result<()> {
        malformed(self.tag()?)?;
        malformed(self.varint()? <= MAX_MONEY)
    }

    fn bytes(&mut self, n: usize) -> Result<()> {
        malformed(self.varint()? == n as u64)?;
        self.take(n)?;
        Ok(())
    }

    /// Absent, or present and empty (`stream.rs::empty_sapling`).
    fn empty_sapling(&mut self) -> Result<()> {
        if !self.tag()? {
            return Ok(());
        }
        policy(self.varint()? == 0)?; // spends
        policy(self.varint()? == 0)?; // outputs
        policy(self.varint()? == 0)?; // value_sum
        self.optional(32)?; // anchor
        self.optional(32)?; // bsk
        Ok(())
    }

    /// One `transparent::Output` (`stream.rs::transparent_output`).
    fn transparent_output(&mut self) -> Result<()> {
        let value = self.varint()?;
        malformed(value <= MAX_MONEY)?;
        policy(value > 0)?;
        let length = self.varint()?;
        policy(length == P2PKH_SCRIPT_BYTES as u64 || length == P2SH_SCRIPT_BYTES as u64)?;
        let script = self.take(length as usize)?;
        // Twin of `stream.rs::transparent_script`. The hash length is bound
        // here too: an unbound `..` would admit a 23-byte script wearing P2PKH
        // opcodes, or a 25-byte one wearing P2SH's, both of which the other
        // twin refuses -- and the whole point of this reader is that the two
        // accept the same language.
        policy(matches!(
            script,
            [0x76, 0xa9, 0x14, hash @ .., 0x88, 0xac] | [0xa9, 0x14, hash @ .., 0x87]
                if hash.len() == HASH160_BYTES
        ))?;
        self.absent()?; // redeem_script
        self.empty_map()?; // bip32_derivation
        self.user_address()?;
        self.empty_map()?; // proprietary
        Ok(())
    }

    /// `Option<transparent::Bundle>` (`stream.rs::transparent`): absent, or
    /// present with no inputs and at least one output. Returns the count.
    fn transparent(&mut self) -> Result<usize> {
        if !self.tag()? {
            return Ok(0);
        }
        policy(self.varint()? == 0)?; // inputs
        let outputs = self.varint()?;
        policy(outputs > 0)?;
        if outputs > MAX_TRANSPARENT_OUTPUTS as u64 {
            return Err(Error::capacity());
        }
        for _ in 0..outputs {
            self.transparent_output()?;
        }
        Ok(outputs as usize)
    }

    /// Fingerprint and a three-index hardened path
    /// (`stream.rs::zip32_derivation`).
    fn zip32_derivation(&mut self) -> Result<()> {
        if !self.tag()? {
            return Ok(());
        }
        self.take(32)?; // seed_fingerprint
        policy(self.varint()? == 3)?;
        for _ in 0..3 {
            malformed(self.u32()? & ZIP32_HARDENED != 0)?;
        }
        Ok(())
    }

    /// Bounded UTF-8 string, ignored (`stream.rs::user_address`).
    fn user_address(&mut self) -> Result<()> {
        if !self.tag()? {
            return Ok(());
        }
        let length = self.varint()?;
        policy(length <= USER_ADDRESS_BUDGET as u64)?;
        let bytes = self.take(length as usize)?;
        malformed(core::str::from_utf8(bytes).is_ok())
    }

    fn action(&mut self) -> Result<()> {
        self.required(32)?; // cv_net

        self.required(32)?; // spend.nullifier
        self.required(32)?; // spend.rk
        self.optional(64)?; // spend.spend_auth_sig; checked against value and sighash
        self.required(43)?; // spend.recipient
        self.value()?; // spend.value
        self.required(32)?; // spend.rho
        self.required(32)?; // spend.rseed
        self.required(96)?; // spend.fvk
        self.absent()?; // spend.witness
        self.required(32)?; // spend.alpha
        self.zip32_derivation()?; // spend.zip32_derivation
        self.absent()?; // spend.dummy_sk
        self.empty_map()?;

        self.required(32)?; // output.cmx
        self.take(32)?; // output.ephemeral_key
        policy(self.varint()? == 0)?;
        self.bytes(ENC_CIPHERTEXT_SIZE)?;
        self.bytes(OUT_CIPHERTEXT_SIZE)?;
        self.required(43)?; // output.recipient
        self.value()?; // output.value
        self.required(32)?; // output.rseed
        self.optional(32)?; // output.ock
        self.zip32_derivation()?; // output.zip32_derivation
        self.user_address()?;
        self.empty_map()?;

        self.required(32)?; // rcv
        Ok(())
    }
}

#[cfg(feature = "test")]
pub(crate) fn scan(bytes: &[u8]) -> Result<Header> {
    if bytes.len() > MAX_PCZT_BYTES {
        return Err(Error::capacity());
    }
    let mut r = Reader { rest: bytes };
    malformed(r.take(8)? == b"PCZT\x02\0\0\0")?;
    let header = Header {
        version: r.u32()?,
        group: r.u32()?,
        branch: r.u32()?,
        lock_time: if r.tag()? { r.u32()? } else { 0 },
        expiry: r.u32()?,
        coin_type: r.u32()?,
    };
    policy(r.byte()? == 0)?;
    r.empty_map()?;
    let transparent_outputs = r.transparent()?;
    r.empty_sapling()?;
    r.absent()?; // orchard
    policy(r.tag()?)?;
    let count = r.varint()?;
    if count == 0 {
        return Err(Error::policy());
    }
    // One shared ZIP-317 logical-action cap (see `MAX_TRANSPARENT_OUTPUTS`).
    if count > (MAX_ACTIONS - transparent_outputs) as u64 {
        return Err(Error::capacity());
    }
    for _ in 0..count {
        r.action()?;
    }
    r.byte()?; // flags checked using upstream's versioned flag implementation
    malformed(r.varint()? <= MAX_MONEY)?;
    policy(!r.tag()?)?;
    r.optional(32)?; // anchor, allowed to be deferred in v6
    policy(r.varint()? == 1)?;
    r.absent()?; // zkproof
    r.optional(32)?; // bsk, ignored
    malformed(r.rest.is_empty())?;
    Ok(header)
}

/// Bounds and shape check only. Success is not semantic validation or
/// permission to sign.
#[cfg(feature = "test")]
pub fn preflight(bytes: &[u8]) -> Result<()> {
    scan(bytes).map(|_| ())
}
