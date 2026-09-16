//! Profile-specific assembly; consensus hashing and bundle extraction remain upstream.
use crate::{Error, Result, wire::Header};
use zcash_primitives::transaction::{
    Authorization, TransactionData, sighash::SignableInput, sighash_v6::v6_signature_hash,
    txid::TxIdDigester,
};
use zcash_protocol::{consensus::BranchId, value::ZatBalance};

struct EffectsOnly;
impl Authorization for EffectsOnly {
    type TransparentAuth = transparent::bundle::EffectsOnly;
    type SaplingAuth = sapling::bundle::EffectsOnly;
    type OrchardAuth = orchard::bundle::EffectsOnly;
}

pub(crate) fn sighash(bundle: &orchard::pczt::Bundle, header: &Header) -> Result<[u8; 32]> {
    // Callers enforce the v6/group/branch/empty-other-pools profile first.
    // The header stays private and immutable beside its corresponding PCZT.
    let tx: TransactionData<EffectsOnly> = TransactionData::from_parts_v6(
        BranchId::try_from(header.branch).map_err(|_| Error("unknown branch"))?,
        header.lock_time,
        header.expiry.into(),
        None,
        None,
        None,
        bundle
            .extract_effects::<ZatBalance>()
            .map_err(|_| Error("effect extraction failed"))?,
    );
    let parts = tx.digest(TxIdDigester);
    Ok(v6_signature_hash(&tx, &SignableInput::Shielded, &parts)
        .as_bytes()
        .try_into()
        .expect("upstream 32-byte signature digest"))
}
