//! Profile-specific assembly; consensus hashing and bundle extraction remain
//! upstream.

use zcash_primitives::transaction::sighash::SignableInput;
use zcash_primitives::transaction::sighash_v6::v6_signature_hash;
use zcash_primitives::transaction::txid::TxIdDigester;
use zcash_primitives::transaction::{Authorization, TransactionData};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::value::ZatBalance;

use crate::wire::Header;
use crate::{Error, Result};

struct EffectsOnly;
impl Authorization for EffectsOnly {
    type TransparentAuth = transparent::bundle::EffectsOnly;
    type SaplingAuth = sapling::bundle::EffectsOnly;
    type OrchardAuth = orchard::bundle::EffectsOnly;
}

pub(crate) fn sighash(bundle: &orchard::pczt::Bundle, header: &Header) -> Result<[u8; 32]> {
    let tx: TransactionData<EffectsOnly> = TransactionData::from_parts_v6(
        BranchId::try_from(header.branch).map_err(|_| Error::policy())?,
        header.lock_time,
        header.expiry.into(),
        None,
        None,
        None,
        bundle
            .extract_effects::<ZatBalance>()
            .map_err(|_| Error::malformed())?,
    );
    let parts = tx.digest(TxIdDigester);
    Ok(v6_signature_hash(&tx, &SignableInput::Shielded, &parts)
        .as_bytes()
        .try_into()
        .expect("upstream 32-byte signature digest"))
}
