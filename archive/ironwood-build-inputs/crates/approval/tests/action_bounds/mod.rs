use super::common::{build_actions, encode, engine, json, keys};
use orchard::{Anchor, ValuePool, keys::Scope, note_encryption::IronwoodDomain};
use pczt::{
    Pczt,
    roles::{
        signer::{Signer, SpendAuthSignature},
        verifier::{OrchardError, Verifier},
    },
};
use serde_json::{Value, json};
use zcash_note_encryption::{Domain, EphemeralKeyBytes};

fn populate_output_keys(value: &mut Value) {
    let ovk = keys().0.to_ovk(Scope::External);
    Verifier::new(Pczt::parse(&encode(value.clone())).unwrap())
        .with_ironwood(|bundle| -> Result<(), OrchardError<()>> {
            for (index, action) in bundle.actions().iter().enumerate() {
                let ock = IronwoodDomain::derive_ock(
                    &ovk,
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
    let review = engine.begin(bytes).unwrap();
    assert_eq!(*review.sighash(), expected_digest);
    assert_eq!(review.projection().outputs.len(), action_count);
    assert_eq!(review.projection().fee, action_count.max(2) as u64 * 5_000);
    engine.approve(review.token()).unwrap();
    let signed = engine.sign(review.token(), &keys().1).unwrap();
    assert_eq!(&signed.token, review.token());
    assert_eq!(signed.sighash, expected_digest);
    assert_eq!(
        signed
            .signatures
            .iter()
            .map(|s| s.action_index())
            .collect::<Vec<_>>(),
        real_indices,
    );

    // Only the previously unsigned real spends may change. This compares every
    // other field, including dummy FVKs/signatures, OCKs, anchors and action order.
    let mut expected = before;
    for signature in &signed.signatures {
        assert_eq!(signature.value_pool(), ValuePool::Ironwood);
        let index = signature.action_index();
        let field = &mut expected["ironwood"]["actions"][index]["spend"]["spend_auth_sig"];
        assert!(field.is_null());
        *field = json!(signature.signature().to_vec());
        oracle
            .apply_orchard_spend_auth_signature(&SpendAuthSignature::from_parts(
                signature.value_pool(),
                index,
                *signature.signature(),
            ))
            .unwrap();
    }
    assert_eq!(json(&signed.pczt.clone().serialize().unwrap()), expected);
    assert_eq!(
        Signer::new(signed.pczt.clone()).unwrap().shielded_sighash(),
        expected_digest,
    );

    let fvk = keys().0;
    Verifier::new(signed.pczt)
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
