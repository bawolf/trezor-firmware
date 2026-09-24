use orchard::keys::Scope;
use orchard::note_encryption::IronwoodDomain;
use orchard::{Anchor, ValuePool};
use pczt::Pczt;
use pczt::roles::signer::{Signer, SpendAuthSignature};
use pczt::roles::verifier::{OrchardError, Verifier};
use serde_json::{Value, json};
use zcash_note_encryption::{Domain, EphemeralKeyBytes};

use super::common::{TestEngineExt, build_actions, encode, engine, json, keys, signatures};

fn populate_output_keys(value: &mut Value) {
    let fvk = keys().0;
    Verifier::new(Pczt::parse(&encode(value.clone())).unwrap())
        .with_ironwood(|bundle| -> Result<(), OrchardError<()>> {
            for (index, action) in bundle.actions().iter().enumerate() {
                if action.output().value().unwrap().inner() == 0 {
                    // Standard padding uses ovk=None and therefore has no
                    // recoverable outgoing ciphertext to accompany with OCK.
                    continue;
                }
                let recipient = action.output().recipient().unwrap();
                let scope = if fvk.scope_for_address(&recipient) == Some(Scope::Internal) {
                    Scope::Internal
                } else {
                    Scope::External
                };
                let ock = IronwoodDomain::derive_ock(
                    &fvk.to_ovk(scope),
                    action.cv_net(),
                    &action.output().cmx().to_bytes(),
                    &EphemeralKeyBytes(action.output().encrypted_note().epk_bytes),
                );
                value["ironwood"]["actions"][index]["output"]["ock"] = json!(ock.0);
            }
            Ok(())
        })
        .unwrap();
}

fn check_signing(bytes: &[u8], action_count: usize, expected_digest: [u8; 32]) {
    let pczt = Pczt::parse(bytes).unwrap();
    assert_eq!(pczt.ironwood().actions().len(), action_count);
    let before = json(bytes);
    let real_indices: Vec<_> = before["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter_map(|(index, action)| {
            (action["spend"]["value"].as_u64().unwrap() > 0).then_some(index)
        })
        .collect();
    assert_eq!(real_indices.len(), 1);

    let mut oracle = Signer::new(pczt).unwrap();
    assert_eq!(oracle.shielded_sighash(), expected_digest);
    let mut engine = engine();
    let review = engine.begin_test(bytes).unwrap();
    assert_eq!(*review.sighash(), expected_digest);
    assert_eq!(review.projection().outputs.len(), action_count);
    assert_eq!(review.projection().fee, action_count.max(2) as u64 * 5_000);
    engine.approve(review.token()).unwrap();
    let signed = engine.sign(review.token(), &keys().1).unwrap();
    let signatures = signatures(&signed);
    assert_eq!(
        signatures
            .iter()
            .map(|(index, _)| *index)
            .collect::<Vec<_>>(),
        real_indices,
    );

    // Only the previously unsigned real spends may change. This compares every
    // other field, including dummy FVKs/signatures, OCKs, anchors and action order.
    let mut expected = before;
    for (index, signature) in &signatures {
        let field = &mut expected["ironwood"]["actions"][*index]["spend"]["spend_auth_sig"];
        assert!(field.is_null());
        *field = json!(signature.to_vec());
        oracle
            .apply_orchard_spend_auth_signature(&SpendAuthSignature::from_parts(
                ValuePool::Ironwood,
                *index,
                *signature,
            ))
            .unwrap();
    }
    assert_eq!(json(&signed.clone().serialize().unwrap()), expected);
    assert_eq!(
        Signer::new(signed.clone()).unwrap().shielded_sighash(),
        expected_digest,
    );

    let fvk = keys().0;
    Verifier::new(signed)
        .with_ironwood(|bundle| -> Result<(), OrchardError<()>> {
            bundle.verify_cross_address_restriction()?;
            for action in bundle.actions() {
                action.verify_cv_net()?;
                action.spend().verify_nullifier(Some(&fvk))?;
                action.spend().verify_rk(Some(&fvk))?;
                action.output().verify_note_commitment(action.spend())?;
                action
                    .spend()
                    .rk()
                    .verify(
                        &expected_digest,
                        action.spend().spend_auth_sig().as_ref().unwrap(),
                    )
                    .unwrap();
            }
            Ok(())
        })
        .unwrap();
}

#[test]
fn signing_preserves_metadata_across_action_bounds() {
    for action_count in 1..=8 {
        let bytes = build_actions(action_count);
        let expected_digest = Signer::new(Pczt::parse(&bytes).unwrap())
            .unwrap()
            .shielded_sighash();
        let mut without_ock = json(&bytes);
        assert!(without_ock["ironwood"]["anchor"].is_null());
        for action in without_ock["ironwood"]["actions"].as_array_mut().unwrap() {
            action["output"]["ock"] = Value::Null;
        }
        let mut with_ock = without_ock.clone();
        populate_output_keys(&mut with_ock);

        for mut value in [without_ock, with_ock] {
            for anchor in [None, Some(Anchor::empty_tree().to_bytes())] {
                value["ironwood"]["anchor"] = json!(anchor);
                check_signing(&encode(value.clone()), action_count, expected_digest);
            }
        }
    }
}
