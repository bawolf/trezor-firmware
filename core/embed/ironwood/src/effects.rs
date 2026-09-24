//! Profile-specific assembly; consensus hashing and bundle extraction remain
//! upstream.

use alloc::vec::Vec;

use transparent::address::Script;
use transparent::bundle::{Bundle, TxOut};
use transparent::sighash::TransparentAuthorizingContext;
use zcash_primitives::transaction::sighash::SignableInput;
use zcash_primitives::transaction::sighash_v6::v6_signature_hash;
use zcash_primitives::transaction::txid::TxIdDigester;
use zcash_primitives::transaction::{Authorization, TransactionData};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::value::{ZatBalance, Zatoshis};

use crate::wire::Header;
use crate::{Error, Result, TransparentKind, TransparentOutput};

/// The transparent authorization of a bundle with NO transparent inputs.
///
/// `transparent::bundle::EffectsOnly` carries the input amounts and
/// scriptPubKeys that ZIP-244 §S.2 needs, and cannot be constructed outside
/// its crate. This profile admits no transparent input at all (`wire::scan`
/// and `stream.rs` refuse one), which is exactly what makes §S.2 collapse to
/// the txid node §T.2 -- so the two lists are empty by construction and this
/// type says so.
#[derive(Debug)]
struct NoTransparentInputs;

impl transparent::bundle::Authorization for NoTransparentInputs {
    type ScriptSig = ();
}

impl TransparentAuthorizingContext for NoTransparentInputs {
    fn input_amounts(&self) -> Vec<Zatoshis> {
        Vec::new()
    }

    fn input_scriptpubkeys(&self) -> Vec<Script> {
        Vec::new()
    }
}

struct EffectsOnly;
impl Authorization for EffectsOnly {
    type TransparentAuth = NoTransparentInputs;
    type SaplingAuth = sapling::bundle::EffectsOnly;
    type OrchardAuth = orchard::bundle::EffectsOnly;
}

/// The transparent bundle upstream hashes, rebuilt from the admitted outputs.
/// `TransparentAddress::script()` regenerates the very bytes the grammar
/// solved, so this is a round trip, not a second encoding.
fn transparent(outputs: &[TransparentOutput]) -> Result<Option<Bundle<NoTransparentInputs>>> {
    if outputs.is_empty() {
        return Ok(None);
    }
    let vout = outputs
        .iter()
        .map(|output| {
            let address = match output.kind {
                TransparentKind::P2pkh => {
                    transparent::address::TransparentAddress::PublicKeyHash(output.hash)
                }
                TransparentKind::P2sh => {
                    transparent::address::TransparentAddress::ScriptHash(output.hash)
                }
            };
            Ok(TxOut::new(
                Zatoshis::from_u64(output.value).map_err(|_| Error::malformed())?,
                Script::from(&address.script()),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(Bundle {
        vin: Vec::new(),
        vout,
        authorization: NoTransparentInputs,
    }))
}

pub(crate) fn sighash(
    transparent_outputs: &[TransparentOutput],
    bundle: &orchard::pczt::Bundle,
    header: &Header,
) -> Result<[u8; 32]> {
    let tx: TransactionData<EffectsOnly> = TransactionData::from_parts_v6(
        BranchId::try_from(header.branch).map_err(|_| Error::policy())?,
        header.lock_time,
        header.expiry.into(),
        transparent(transparent_outputs)?,
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
