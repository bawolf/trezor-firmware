//! Nonallocating admission for the pinned v2 Postcard layout. Not a general PCZT parser.
use crate::{Error, Result, ensure};
use zcash_note_encryption::{ENC_CIPHERTEXT_SIZE, OUT_CIPHERTEXT_SIZE};
use zcash_protocol::value::MAX_MONEY;

pub const MAX_PCZT_BYTES: usize = 65_536;
pub const MAX_ACTIONS: usize = 8;

#[derive(Clone, Copy)]
pub(crate) struct Header {
    pub version: u32,
    pub group: u32,
    pub branch: u32,
    pub lock_time: u32,
    pub expiry: u32,
    pub coin_type: u32,
}

struct Reader<'a> {
    rest: &'a [u8],
}
impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let bytes = self.rest.get(..n).ok_or(Error("truncated frame"))?;
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
            _ => Err(Error("invalid option or boolean")),
        }
    }
    fn varint(&mut self) -> Result<u64> {
        let mut value = 0u64;
        for i in 0..10 {
            let b = self.byte()?;
            ensure(i < 9 || b <= 1, "integer overflow")?;
            value |= u64::from(b & 127) << (i * 7);
            if b & 128 == 0 {
                ensure(i == 0 || b != 0, "noncanonical integer")?;
                return Ok(value);
            }
        }
        Err(Error("integer overflow"))
    }
    fn u32(&mut self) -> Result<u32> {
        self.varint()?.try_into().map_err(|_| Error("u32 overflow"))
    }
    fn absent(&mut self) -> Result<()> {
        ensure(!self.tag()?, "unsupported optional field")
    }
    fn required(&mut self, n: usize) -> Result<()> {
        ensure(self.tag()?, "missing approval field")?;
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
        ensure(self.varint()? == 0, "proprietary metadata unsupported")
    }
    fn value(&mut self) -> Result<()> {
        ensure(self.tag()?, "missing approval value")?;
        ensure(self.varint()? <= MAX_MONEY, "value exceeds MAX_MONEY")
    }
    fn bytes(&mut self, n: usize) -> Result<()> {
        ensure(self.varint()? == n as u64, "invalid ciphertext length")?;
        self.take(n)?;
        Ok(())
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
        self.absent()?; // spend.zip32_derivation
        self.absent()?; // spend.dummy_sk
        self.empty_map()?; // spend.proprietary

        self.required(32)?; // output.cmx
        self.take(32)?; // output.ephemeral_key
        // output.enc_ciphertext variant: encrypted only
        ensure(self.varint()? == 0, "plaintext memo encoding unsupported")?;
        self.bytes(ENC_CIPHERTEXT_SIZE)?; // output.enc_ciphertext bytes
        self.bytes(OUT_CIPHERTEXT_SIZE)?; // output.out_ciphertext
        self.required(43)?; // output.recipient
        self.value()?; // output.value
        self.required(32)?; // output.rseed
        self.optional(32)?; // output.ock
        self.absent()?; // output.zip32_derivation
        self.absent()?; // output.user_address
        self.empty_map()?; // output.proprietary

        self.required(32)?; // rcv
        Ok(())
    }
}

pub(crate) fn scan(bytes: &[u8]) -> Result<Header> {
    ensure(bytes.len() <= MAX_PCZT_BYTES, "PCZT byte limit")?;
    let mut r = Reader { rest: bytes };
    ensure(r.take(8)? == b"PCZT\x02\0\0\0", "unsupported PCZT encoding")?;
    let header = Header {
        version: r.u32()?,
        group: r.u32()?,
        branch: r.u32()?,
        lock_time: if r.tag()? { r.u32()? } else { 0 },
        expiry: r.u32()?,
        coin_type: r.u32()?,
    };
    ensure(r.byte()? == 0, "modifiable transaction")?;
    r.empty_map()?;
    r.absent()?; // transparent
    r.absent()?; // sapling
    r.absent()?; // orchard
    ensure(r.tag()?, "missing Ironwood bundle")?;
    let count = r.varint()?;
    ensure(
        (1..=MAX_ACTIONS as u64).contains(&count),
        "Ironwood action limit",
    )?;
    for _ in 0..count {
        r.action()?;
    }
    r.byte()?; // flags checked using upstream's versioned flag implementation
    ensure(r.varint()? <= MAX_MONEY, "bundle balance exceeds MAX_MONEY")?;
    ensure(!r.tag()?, "negative bundle balance")?;
    r.optional(32)?; // anchor, allowed to be deferred in v6
    ensure(r.varint()? == 1, "unsupported note version")?;
    r.absent()?; // proof
    r.absent()?; // bsk
    ensure(r.rest.is_empty(), "trailing frame bytes")?;
    Ok(header)
}

/// Bounds/shape check only. Success is not semantic validation or permission to sign.
pub fn preflight(bytes: &[u8]) -> Result<()> {
    scan(bytes).map(|_| ())
}
