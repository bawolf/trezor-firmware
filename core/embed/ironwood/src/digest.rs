//! Streaming V6 shielded signature digest over Ironwood actions.
//!
//! Reproduces `effects::sighash` byte for byte from running BLAKE2b states, so
//! no action is retained. The tree, field order and personalizations follow the
//! pinned `zcash_primitives::transaction::txid` and
//! `orchard::bundle::commitments`; the transparent, Sapling and Orchard nodes
//! are their fixed empty digests, and the anchor is deliberately absent because
//! it belongs to the authorizing-data digest.

use blake2b_simd::{Params, State};
use orchard::ValuePool;
use orchard::bundle::TxVersion;
use orchard::bundle::commitments::hash_bundle_txid_empty;
use zcash_note_encryption::{
    COMPACT_NOTE_SIZE, ENC_CIPHERTEXT_SIZE, NOTE_PLAINTEXT_SIZE, OUT_CIPHERTEXT_SIZE,
};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::constants::{V6_TX_VERSION, V6_VERSION_GROUP_ID};

use crate::wire::Header;
use crate::{Error, Result};

// Private in zcash_primitives::transaction::txid; retyped here.
const TX_HASH_PREFIX: &[u8; 12] = b"ZcashTxHash_";
const HEADERS_HASH: &[u8; 16] = b"ZTxIdHeadersHash";
const TRANSPARENT_HASH: &[u8; 16] = b"ZTxIdTranspaHash";
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

fn finalize(state: &State) -> [u8; 32] {
    state
        .finalize()
        .as_bytes()
        .try_into()
        .expect("configured 32-byte hash")
}

fn empty(pool: ValuePool) -> [u8; 32] {
    hash_bundle_txid_empty(pool, TxVersion::V6)
        .expect("Orchard and Ironwood commitments exist in V6")
        .as_bytes()
        .try_into()
        .expect("configured 32-byte hash")
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

/// Running digest of one Ironwood-only V6 transaction.
pub(crate) struct Digest {
    branch: u32,
    header: [u8; 32],
    actions: usize,
    compact: State,
    memos: State,
    noncompact: State,
}

impl Digest {
    /// Like `effects::sighash`, fixes the V6 header constants rather than
    /// hashing the header's own version fields; admission enforces those.
    pub(crate) fn new(header: &Header) -> Result<Self> {
        BranchId::try_from(header.branch).map_err(|_| Error::policy())?;
        let mut h = hasher(HEADERS_HASH);
        h.update(&(OVERWINTERED | V6_TX_VERSION).to_le_bytes())
            .update(&V6_VERSION_GROUP_ID.to_le_bytes())
            .update(&header.branch.to_le_bytes())
            .update(&header.lock_time.to_le_bytes())
            .update(&header.expiry.to_le_bytes());
        Ok(Self {
            branch: header.branch,
            header: finalize(&h),
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

    // Reached only by tests/digest_equivalence.rs, which compiles this file in.
    #[cfg(feature = "test")]
    #[allow(dead_code)]
    pub(crate) fn header_digest(&self) -> [u8; 32] {
        self.header
    }

    /// Without actions the bundle is absent and its empty digest stands in, as
    /// upstream extraction does.
    pub(crate) fn ironwood_digest(&self, flags: u8, value_balance: i64) -> [u8; 32] {
        if self.actions == 0 {
            return empty(ValuePool::Ironwood);
        }
        let mut h = hasher(IRONWOOD_HASH);
        h.update(&finalize(&self.compact))
            .update(&finalize(&self.memos))
            .update(&finalize(&self.noncompact))
            .update(&[flags])
            .update(&value_balance.to_le_bytes());
        finalize(&h)
    }

    pub(crate) fn finish(self, flags: u8, value_balance: i64) -> [u8; 32] {
        let mut personal = [0; 16];
        personal[..12].copy_from_slice(TX_HASH_PREFIX);
        personal[12..].copy_from_slice(&self.branch.to_le_bytes());
        let mut h = hasher(&personal);
        h.update(&self.header)
            .update(&finalize(&hasher(TRANSPARENT_HASH)))
            .update(&finalize(&hasher(SAPLING_HASH)))
            .update(&empty(ValuePool::Orchard))
            .update(&self.ironwood_digest(flags, value_balance));
        finalize(&h)
    }
}

/// Fixture-free checks of the header, empty-pool and root nodes against the
/// upstream digest of a bundle-less transaction. The Ironwood node is covered
/// over the conformance corpus by `tests/digest_equivalence.rs`.
#[cfg(test)]
mod tests {
    use zcash_primitives::transaction::sighash::SignableInput;
    use zcash_primitives::transaction::sighash_v6::v6_signature_hash;
    use zcash_primitives::transaction::txid::TxIdDigester;
    use zcash_primitives::transaction::{Authorization, TransactionData};
    use zcash_protocol::consensus::BranchId;

    use super::Digest;
    use crate::Error;
    use crate::wire::Header;

    struct EffectsOnly;
    impl Authorization for EffectsOnly {
        type TransparentAuth = transparent::bundle::EffectsOnly;
        type SaplingAuth = sapling::bundle::EffectsOnly;
        type OrchardAuth = orchard::bundle::EffectsOnly;
    }

    fn header(branch: u32, lock_time: u32, expiry: u32) -> Header {
        Header {
            version: 6,
            group: 0xD884_B698,
            branch,
            lock_time,
            expiry,
            coin_type: 1,
        }
    }

    fn upstream(branch: BranchId, lock_time: u32, expiry: u32) -> [u8; 32] {
        let tx: TransactionData<EffectsOnly> = TransactionData::from_parts_v6(
            branch,
            lock_time,
            expiry.into(),
            None,
            None,
            None,
            None,
        );
        let parts = tx.digest(TxIdDigester);
        assert!(parts.transparent_digests.is_none());
        assert!(parts.sapling_digest.is_none());
        assert!(parts.orchard_digest.is_none());
        assert!(parts.ironwood_digest.is_none());
        v6_signature_hash(&tx, &SignableInput::Shielded, &parts)
            .as_bytes()
            .try_into()
            .unwrap()
    }

    const BRANCHES: [BranchId; 9] = [
        BranchId::Sapling,
        BranchId::Blossom,
        BranchId::Heartwood,
        BranchId::Canopy,
        BranchId::Nu5,
        BranchId::Nu6,
        BranchId::Nu6_1,
        BranchId::Nu6_2,
        BranchId::Nu6_3,
    ];
    const HEADERS: [(u32, u32); 4] = [(0, 0), (0, 10_000_100), (1, u32::MAX), (u32::MAX, 7)];

    #[test]
    fn bundle_less_transaction_matches_upstream_for_every_branch_and_header() {
        let mut seen = [[0; 32]; BRANCHES.len() * HEADERS.len()];
        let mut count = 0;
        for branch in BRANCHES {
            for (lock_time, expiry) in HEADERS {
                let digest = Digest::new(&header(branch.into(), lock_time, expiry)).unwrap();
                // Flags and balance are not hashed when no action was fed.
                let streamed = digest.finish(0xff, i64::MIN);
                assert_eq!(streamed, upstream(branch, lock_time, expiry));
                assert!(!seen[..count].contains(&streamed));
                seen[count] = streamed;
                count += 1;
            }
        }
    }

    #[test]
    fn unknown_branch_is_rejected_like_effects() {
        for branch in [1, 2, 0xdead_beef, u32::MAX] {
            assert!(BranchId::try_from(branch).is_err());
            assert_eq!(
                Digest::new(&header(branch, 0, 1)).err(),
                Some(Error::policy())
            );
        }
    }

    #[test]
    fn header_version_fields_are_fixed_to_v6() {
        let mut other = header(BranchId::Nu6_3.into(), 0, 1);
        other.version = 5;
        other.group = 0;
        other.coin_type = 133;
        assert_eq!(
            Digest::new(&other).unwrap().finish(0, 0),
            Digest::new(&header(BranchId::Nu6_3.into(), 0, 1))
                .unwrap()
                .finish(0, 0)
        );
    }
}
