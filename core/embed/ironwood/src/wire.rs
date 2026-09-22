//! Nonallocating admission for the pinned v2 Postcard layout. Not a general
//! PCZT parser.

use zcash_note_encryption::{ENC_CIPHERTEXT_SIZE, OUT_CIPHERTEXT_SIZE};
use zcash_protocol::value::MAX_MONEY;

use crate::{Error, Result, ZIP32_HARDENED};

pub const MAX_PCZT_BYTES: usize = 65_536;
// Raised 8 -> 32 to admit 16- and 32-action bundles. The cross-action state is
// all fixed-capacity `[T; MAX_ACTIONS]` (records ~98 B, nullifiers 32 B, outputs
// 64 B, signature records 66 B per action), so the retained base grows by only
// ~+5 KB at CAP=32 and stays O(1) in N; peak RAM is dominated by the single live
// action's transient working set, not this cap. NEW change pending Fable review.
pub const MAX_ACTIONS: usize = 32;
/// Longest `output.user_address` admitted, in bytes. The stock SDK stamps the
/// ZIP-321 recipient string on every payment output; a unified address with
/// every receiver type is about 213 characters, so this bounds the scanner's
/// section buffer without refusing any real address.
pub const USER_ADDRESS_BUDGET: usize = 512;

#[derive(Clone, Copy)]
pub(crate) struct Header {
    pub version: u32,
    pub group: u32,
    pub branch: u32,
    pub lock_time: u32,
    pub expiry: u32,
    pub coin_type: u32,
}

fn malformed(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::malformed()) }
}

fn policy(ok: bool) -> Result<()> {
    if ok { Ok(()) } else { Err(Error::policy()) }
}

struct Reader<'a> {
    rest: &'a [u8],
}

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

    /// Fingerprint and a three-index hardened path (`stream.rs::zip32_derivation`).
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
    r.absent()?; // transparent
    r.empty_sapling()?;
    r.absent()?; // orchard
    policy(r.tag()?)?;
    let count = r.varint()?;
    if count == 0 {
        return Err(Error::policy());
    }
    if count > MAX_ACTIONS as u64 {
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
