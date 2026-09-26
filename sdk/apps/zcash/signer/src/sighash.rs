//! The ZIP-244 v6 signature digest of an Ironwood-only transaction, computed
//! incrementally so that no action is retained.
//!
//! The tree and personalizations are those of
//! `zcash_primitives::transaction::txid` and `orchard::bundle::commitments`.
//! The Sapling and Orchard nodes are their empty digests, and the transparent
//! node has no inputs, so ZIP-244 S.2 reduces to T.2.

use blake2b_simd::{Params, State};
use orchard::ValuePool;
use orchard::bundle::TxVersion;
use orchard::bundle::commitments::hash_bundle_txid_empty;
use zcash_note_encryption::{
    COMPACT_NOTE_SIZE, ENC_CIPHERTEXT_SIZE, NOTE_PLAINTEXT_SIZE, OUT_CIPHERTEXT_SIZE,
};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID};

use crate::error::{Error, Result};
use crate::limits::MAX_SCRIPT_PUBKEY_BYTES;
use crate::scanner::Header;

// The `scriptPubKey` length prefix is a one-byte `CompactSize` below 253.
const _: () = assert!(MAX_SCRIPT_PUBKEY_BYTES < 253);

// Private in zcash_primitives::transaction::txid; retyped here.
const TX_HASH_PREFIX: &[u8; 12] = b"ZcashTxHash_";
const HEADERS_HASH: &[u8; 16] = b"ZTxIdHeadersHash";
const TRANSPARENT_HASH: &[u8; 16] = b"ZTxIdTranspaHash";
const PREVOUTS_HASH: &[u8; 16] = b"ZTxIdPrevoutHash";
const SEQUENCE_HASH: &[u8; 16] = b"ZTxIdSequencHash";
const OUTPUTS_HASH: &[u8; 16] = b"ZTxIdOutputsHash";
const SAPLING_HASH: &[u8; 16] = b"ZTxIdSaplingHash";
// Private in orchard::bundle::commitments; retyped here.
const IRONWOOD_HASH: &[u8; 16] = b"ZTxIdIronwd_H_v6";
const IRONWOOD_COMPACT_HASH: &[u8; 16] = b"ZTxIdIrnActCH_v6";
const IRONWOOD_MEMOS_HASH: &[u8; 16] = b"ZTxIdIrnActMH_v6";
const IRONWOOD_NONCOMPACT_HASH: &[u8; 16] = b"ZTxIdIrnActNH_v6";

const OVERWINTERED: u32 = 1 << 31;

fn hasher(personal: &[u8; 16]) -> State {
    Params::new().hash_length(32).personal(personal).to_state()
}

/// The 32-byte digest of a state created with `hash_length(32)`.
pub(crate) fn finalize(state: &State) -> [u8; 32] {
    let mut digest = [0; 32];
    digest.copy_from_slice(state.finalize().as_bytes());
    digest
}

fn empty(pool: ValuePool) -> Result<[u8; 32]> {
    let hash = hash_bundle_txid_empty(pool, TxVersion::V6).map_err(|_| Error::Internal)?;
    let mut digest = [0; 32];
    digest.copy_from_slice(hash.as_bytes());
    Ok(digest)
}

/// Effecting fields of one action in wire encoding.
pub(crate) struct ActionEffects<'a> {
    pub cv_net: &'a [u8; 32],
    pub nullifier: &'a [u8; 32],
    pub rk: &'a [u8; 32],
    pub cmx: &'a [u8; 32],
    pub ephemeral_key: &'a [u8; 32],
    pub enc_ciphertext: &'a [u8; ENC_CIPHERTEXT_SIZE],
    pub out_ciphertext: &'a [u8; OUT_CIPHERTEXT_SIZE],
}

/// Running signature digest of one Ironwood-only v6 transaction.
pub(crate) struct Sighash {
    branch: u32,
    header: [u8; 32],
    transparent_outputs: usize,
    outputs: State,
    actions: usize,
    compact: State,
    memos: State,
    noncompact: State,
}

impl Sighash {
    /// Hashes the v6 version constants, not the header's own version fields,
    /// which the caller checks.
    pub(crate) fn new(header: &Header) -> Result<Self> {
        BranchId::try_from(header.branch).map_err(|_| Error::Policy)?;
        let mut h = hasher(HEADERS_HASH);
        h.update(&(OVERWINTERED | V6_TX_VERSION).to_le_bytes())
            .update(&V6_VERSION_GROUP_ID.to_le_bytes())
            .update(&header.branch.to_le_bytes())
            .update(&header.lock_time.to_le_bytes())
            .update(&header.expiry.to_le_bytes());
        Ok(Self {
            branch: header.branch,
            header: finalize(&h),
            transparent_outputs: 0,
            outputs: hasher(OUTPUTS_HASH),
            actions: 0,
            compact: hasher(IRONWOOD_COMPACT_HASH),
            memos: hasher(IRONWOOD_MEMOS_HASH),
            noncompact: hasher(IRONWOOD_NONCOMPACT_HASH),
        })
    }

    pub(crate) fn action(&mut self, action: &ActionEffects<'_>) {
        self.actions += 1;
        self.compact
            .update(action.nullifier)
            .update(action.cmx)
            .update(action.ephemeral_key)
            .update(&action.enc_ciphertext[..COMPACT_NOTE_SIZE]);
        self.memos
            .update(&action.enc_ciphertext[COMPACT_NOTE_SIZE..NOTE_PLAINTEXT_SIZE]);
        self.noncompact
            .update(action.cv_net)
            .update(action.rk)
            .update(&action.enc_ciphertext[NOTE_PLAINTEXT_SIZE..])
            .update(action.out_ciphertext);
    }

    /// Feeds one transparent output into the ZIP-244 T.2c outputs hash, as
    /// `TxOut::write` encodes it.
    pub(crate) fn transparent_output(&mut self, value: u64, script_pubkey: &[u8]) {
        self.transparent_outputs += 1;
        self.outputs
            .update(&value.to_le_bytes())
            .update(&[script_pubkey.len() as u8])
            .update(script_pubkey);
    }

    /// ZIP-244 T.2 with no inputs. Without outputs the bundle is absent and
    /// the empty transparent digest stands in.
    fn transparent_digest(&self) -> [u8; 32] {
        let mut h = hasher(TRANSPARENT_HASH);
        if self.transparent_outputs > 0 {
            h.update(&finalize(&hasher(PREVOUTS_HASH)))
                .update(&finalize(&hasher(SEQUENCE_HASH)))
                .update(&finalize(&self.outputs));
        }
        finalize(&h)
    }

    /// Without actions the bundle is absent and its empty digest stands in.
    fn ironwood_digest(&self, flags: u8, value_balance: i64) -> Result<[u8; 32]> {
        if self.actions == 0 {
            return empty(ValuePool::Ironwood);
        }
        let mut h = hasher(IRONWOOD_HASH);
        h.update(&finalize(&self.compact))
            .update(&finalize(&self.memos))
            .update(&finalize(&self.noncompact))
            .update(&[flags])
            .update(&value_balance.to_le_bytes());
        Ok(finalize(&h))
    }

    pub(crate) fn finish(self, flags: u8, value_balance: i64) -> Result<[u8; 32]> {
        let mut personal = [0; 16];
        personal[..12].copy_from_slice(TX_HASH_PREFIX);
        personal[12..].copy_from_slice(&self.branch.to_le_bytes());
        let mut h = hasher(&personal);
        h.update(&self.header)
            .update(&self.transparent_digest())
            .update(&finalize(&hasher(SAPLING_HASH)))
            .update(&empty(ValuePool::Orchard)?)
            .update(&self.ironwood_digest(flags, value_balance)?);
        Ok(finalize(&h))
    }
}
