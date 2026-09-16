mod action_bounds;
mod common;
mod randomness;
use common::*;
use ironwood_approval::{
    Engine, Error, OutputKind, Policy,
    wire::{MAX_PCZT_BYTES, preflight},
};
use pczt::{
    Pczt,
    roles::{
        signer::Signer,
        verifier::{OrchardError, Verifier},
    },
};
use serde_json::{Value, json};

#[test]
fn positive_payment_change_and_verified_signatures() {
    let bytes = fixture();
    preflight(&bytes).unwrap();
    let mut engine = engine();
    let review = engine.begin(&bytes).unwrap();
    let p = review.projection();
    assert_eq!(
        (p.total_input, p.payments, p.change, p.fee),
        (1_000_000, 600_000, 390_000, 10_000)
    );
    assert_eq!(p.network, "regtest (synthetic)");
    assert_eq!(p.pool, "Ironwood");
    assert_eq!(p.outputs.len(), 2);
    assert!(
        p.outputs
            .iter()
            .any(|o| o.value == 390_000 && o.kind == OutputKind::InternalChange)
    );
    engine.approve(review.token()).unwrap();
    let signed = engine.sign(review.token(), &keys().1).unwrap();
    assert_eq!(signed.signatures.len(), 1);
    assert_eq!(&signed.sighash, review.sighash());
    assert_eq!(
        Signer::new(signed.pczt.clone()).unwrap().shielded_sighash(),
        signed.sighash
    );
    Verifier::new(signed.pczt)
        .with_ironwood(|bundle| -> Result<(), OrchardError<()>> {
            for action in bundle.actions() {
                action
                    .spend()
                    .rk()
                    .verify(
                        &signed.sighash,
                        action.spend().spend_auth_sig().as_ref().unwrap(),
                    )
                    .unwrap();
            }
            Ok(())
        })
        .unwrap();
    assert!(engine.sign(review.token(), &keys().1).is_err());
}

#[test]
fn no_signing_before_approval() {
    let mut e = engine();
    let r = e.begin(&fixture()).unwrap();
    assert!(e.sign(r.token(), &keys().1).is_err());
    assert!(e.approve(r.token()).is_err());
}
#[test]
fn cancellation_and_duplicate_approval_consume_consent() {
    let mut e = engine();
    let r = e.begin(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    e.cancel();
    assert!(e.sign(r.token(), &keys().1).is_err());
    let r = e.begin(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    assert!(e.approve(r.token()).is_err());
    assert!(e.sign(r.token(), &keys().1).is_err());
}
#[test]
fn wrong_key_failure_has_no_retry_or_partial_response() {
    let mut e = engine();
    let r = e.begin(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    let wrong = orchard::keys::SpendAuthorizingKey::from(
        &orchard::keys::SpendingKey::from_bytes([2; 32]).unwrap(),
    );
    assert!(e.sign(r.token(), &wrong).is_err());
    assert!(e.sign(r.token(), &keys().1).is_err());
}
#[test]
fn sessions_and_replacements_do_not_inherit_approval() {
    let mut e = engine();
    let r = e.begin(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    let r2 = e.begin(&fixture()).unwrap();
    assert_ne!(r.token(), r2.token());
    assert!(e.approve(r.token()).is_err());
    assert!(e.sign(r2.token(), &keys().1).is_err());
    let mut other = engine();
    other.begin(&fixture()).unwrap();
    assert!(other.approve(r2.token()).is_err());
}
#[test]
fn malformed_replacement_clears_previous_approval() {
    let mut e = engine();
    let r = e.begin(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    assert!(e.begin(b"PCZT").is_err());
    assert!(e.sign(r.token(), &keys().1).is_err());
}
#[test]
fn same_digest_anchor_substitution_requires_new_consent() {
    let bytes = fixture();
    let altered = mutate(|v| v["ironwood"]["anchor"] = json!(vec![0; 32]));
    assert_ne!(bytes, altered);
    let digest = |b: &[u8]| {
        Signer::new(Pczt::parse(b).unwrap())
            .unwrap()
            .shielded_sighash()
    };
    assert_eq!(digest(&bytes), digest(&altered));
    let mut e = engine();
    let old = e.begin(&bytes).unwrap();
    e.approve(old.token()).unwrap();
    let new = e.begin(&altered).unwrap();
    assert_eq!(old.sighash(), new.sighash());
    assert_ne!(old.token().context(), new.token().context());
    assert!(e.sign(old.token(), &keys().1).is_err());
    assert!(e.sign(new.token(), &keys().1).is_err());
}
#[test]
fn caller_mutation_cannot_change_owned_signing_context() {
    let mut bytes = fixture();
    let mut e = engine();
    let r = e.begin(&bytes).unwrap();
    bytes.fill(0);
    e.approve(r.token()).unwrap();
    assert_eq!(&e.sign(r.token(), &keys().1).unwrap().sighash, r.sighash());
}
#[test]
fn recipient_and_value_tampering_rejected() {
    for field in ["recipient", "value", "rseed", "cmx"] {
        let bytes = mutate(|v| {
            let i = payment(v);
            let f = &mut v["ironwood"]["actions"][i]["output"][field];
            if field == "value" {
                *f = 600_001.into();
            } else {
                flip(f);
            }
        });
        assert!(engine().begin(&bytes).is_err(), "accepted output {field}");
    }
}
#[test]
fn spend_ownership_nullifier_and_randomizer_tampering_rejected() {
    for field in [
        "recipient",
        "value",
        "rho",
        "rseed",
        "fvk",
        "alpha",
        "rk",
        "nullifier",
    ] {
        let bytes = mutate(|v| {
            let i = real(v);
            let f = &mut v["ironwood"]["actions"][i]["spend"][field];
            if field == "value" {
                *f = 1_000_001.into();
            } else {
                flip(f);
            }
        });
        assert!(engine().begin(&bytes).is_err(), "accepted spend {field}");
    }
}
#[test]
fn encrypted_output_and_outgoing_recovery_tampering_rejected() {
    for field in ["ephemeral_key", "enc_ciphertext", "out_ciphertext", "ock"] {
        let bytes = mutate(|v| {
            let i = payment(v);
            let f = &mut v["ironwood"]["actions"][i]["output"][field];
            if field == "enc_ciphertext" {
                flip(&mut f["Encrypted"]);
            } else if field == "ock" {
                *f = json!(vec![0; 32]);
            } else {
                flip(f);
            }
        });
        assert!(engine().begin(&bytes).is_err(), "accepted {field}");
    }
}
#[test]
fn missing_approval_fields_rejected_before_protocol_parsing() {
    for (part, fields) in [
        (
            "spend",
            vec![
                "recipient",
                "value",
                "rho",
                "rseed",
                "fvk",
                "alpha",
                "rk",
                "nullifier",
            ],
        ),
        ("output", vec!["recipient", "value", "rseed", "cmx"]),
    ] {
        for field in fields {
            let bytes = mutate(|v| {
                v["ironwood"]["actions"][0][part][field] = Value::Null;
            });
            assert!(preflight(&bytes).is_err(), "accepted {part}.{field}");
        }
    }
    for field in ["cv_net", "rcv"] {
        assert!(preflight(&mutate(|v| v["ironwood"]["actions"][0][field] = Value::Null)).is_err());
    }
}
#[test]
fn host_change_labels_and_addresses_rejected() {
    let claimed_change = mutate(|v| {
        let i = payment(v);
        v["ironwood"]["actions"][i]["output"]["proprietary"] = json!({"change": [1]});
    });
    assert!(preflight(&claimed_change).is_err());
    let address = mutate(|v| {
        let i = payment(v);
        v["ironwood"]["actions"][i]["output"]["user_address"] = "attacker change".into();
    });
    assert!(preflight(&address).is_err());
}
#[test]
fn external_self_payment_is_not_change() {
    let bytes = build(
        600_000,
        390_000,
        zcash_protocol::memo::MemoBytes::empty(),
        true,
    );
    let r = engine().begin(&bytes).unwrap();
    assert_eq!(r.projection().payments, 600_000);
    assert_eq!(r.projection().change, 390_000);
}
#[test]
fn nonempty_memo_rejected_even_when_encryption_is_valid() {
    let memo = zcash_protocol::memo::MemoBytes::from_bytes(b"hello").unwrap();
    assert_eq!(
        engine()
            .begin(&build(600_000, 390_000, memo, false))
            .unwrap_err(),
        Error("nonempty memo unsupported")
    );
}
#[test]
fn fee_cap_and_declared_balance_rejected() {
    let mut e = Engine::new(Policy::regtest(HEIGHT, 9_999).unwrap(), keys().0).unwrap();
    assert_eq!(
        e.begin(&fixture()).unwrap_err(),
        Error("fee exceeds policy")
    );
    let bytes = mutate(|v| v["ironwood"]["value_sum"][0] = 10_001.into());
    assert!(engine().begin(&bytes).is_err());
    let bytes = mutate(|v| v["ironwood"]["value_sum"][1] = true.into());
    assert!(preflight(&bytes).is_err());
}
#[test]
fn network_version_and_expiry_policy_rejected() {
    for (field, value) in [
        ("coin_type", 133u32),
        ("consensus_branch_id", 0),
        ("tx_version", 5),
        ("version_group_id", 0),
        ("fallback_lock_time", 1),
        ("expiry_height", HEIGHT),
        ("expiry_height", HEIGHT + 101),
        ("tx_modifiable", 128),
    ] {
        let bytes = mutate(|v| v["global"][field] = value.into());
        assert!(engine().begin(&bytes).is_err(), "accepted {field}={value}");
    }
}
#[test]
fn mixed_pools_and_noncanonical_empty_bundles_rejected() {
    for pool in ["orchard", "sapling", "transparent"] {
        let bytes = mutate(|v| {
            v[pool] = match pool {
                "orchard" => v["ironwood"].clone(),
                "transparent" => json!({"inputs":[], "outputs":[]}),
                _ => json!({"spends":[], "outputs":[], "value_sum":0, "anchor":null, "bsk":null}),
            };
        });
        assert!(preflight(&bytes).is_err(), "accepted {pool}");
    }
}
#[test]
fn dummy_signature_and_key_injection_rejected() {
    let missing = mutate(|v| {
        let i = 1 - real(v);
        v["ironwood"]["actions"][i]["spend"]["spend_auth_sig"] = Value::Null;
    });
    assert_eq!(
        engine().begin(&missing).unwrap_err(),
        Error("missing padding signature")
    );
    let bad = mutate(|v| {
        let i = 1 - real(v);
        flip(&mut v["ironwood"]["actions"][i]["spend"]["spend_auth_sig"]);
    });
    assert!(engine().begin(&bad).is_err());
    let key = mutate(|v| v["ironwood"]["actions"][0]["spend"]["dummy_sk"] = json!(vec![0; 32]));
    assert!(preflight(&key).is_err());
}
#[test]
fn duplicated_action_rejected() {
    let bytes = mutate(|v| {
        let i = real(v);
        let action = v["ironwood"]["actions"][i].clone();
        v["ironwood"]["actions"] = json!([action.clone(), action]);
    });
    assert_eq!(
        engine().begin(&bytes).unwrap_err(),
        Error("duplicate nullifier")
    );
}
#[test]
fn action_byte_and_money_bounds_rejected() {
    assert!(preflight(&vec![0; MAX_PCZT_BYTES + 1]).is_err());
    for count in [0, 9] {
        let bytes = mutate(|v| {
            let a = v["ironwood"]["actions"][0].clone();
            v["ironwood"]["actions"] = vec![a; count].into();
        });
        assert!(preflight(&bytes).is_err());
    }
    let bytes = mutate(|v| v["ironwood"]["actions"][0]["spend"]["value"] = u64::MAX.into());
    assert!(preflight(&bytes).is_err());
}
#[test]
fn every_truncation_and_trailing_byte_is_rejected() {
    let bytes = fixture();
    for n in 0..bytes.len() {
        assert!(preflight(&bytes[..n]).is_err(), "accepted length {n}");
    }
    let mut extra = bytes;
    extra.push(0);
    assert!(preflight(&extra).is_err());
}
#[test]
fn noncanonical_varint_and_unknown_encoding_rejected() {
    let mut bytes = fixture();
    bytes.splice(8..9, [0x86, 0]); // tx_version=6 with overlong encoding
    assert_eq!(preflight(&bytes), Err(Error("noncanonical integer")));
    let mut bytes = fixture();
    bytes[4] = 1;
    assert!(preflight(&bytes).is_err());
}

#[test]
fn zero_output_padding_is_verified_and_counted() {
    let bytes = build(990_000, 0, zcash_protocol::memo::MemoBytes::empty(), false);
    let review = engine().begin(&bytes).unwrap();
    assert_eq!(review.projection().padding_outputs, 1);
    assert_eq!(review.projection().payments, 990_000);
    assert_eq!(review.projection().change, 0);
}

#[test]
fn accepted_wire_mutations_agree_with_upstream_deserializer() {
    let bytes = fixture();
    let mut accepted = 0;
    for i in 0..bytes.len() {
        for replacement in [0, 255, bytes[i] ^ 128] {
            let mut mutated = bytes.clone();
            mutated[i] = replacement;
            if preflight(&mutated).is_ok() {
                Pczt::parse(&mutated).expect("preflight admitted an incompatible pinned layout");
                accepted += 1;
            }
        }
    }
    assert!(accepted > 1000);
}

#[test]
fn optional_ock_metadata_must_match_real_output_recovery() {
    use orchard::{keys::Scope, note_encryption::IronwoodDomain};
    use zcash_note_encryption::{Domain, EphemeralKeyBytes};
    let mut value = json(&fixture());
    let i = payment(&value);
    let mut ock = None;
    Verifier::new(Pczt::parse(&fixture()).unwrap())
        .with_ironwood(|bundle| -> Result<(), OrchardError<()>> {
            let a = &bundle.actions()[i];
            ock = Some(
                IronwoodDomain::derive_ock(
                    &keys().0.to_ovk(Scope::External),
                    a.cv_net(),
                    &a.output().cmx().to_bytes(),
                    &EphemeralKeyBytes(a.output().encrypted_note().epk_bytes),
                )
                .0,
            );
            Ok(())
        })
        .unwrap();
    value["ironwood"]["actions"][i]["output"]["ock"] = json!(ock.unwrap());
    engine().begin(&encode(value.clone())).unwrap();
    flip(&mut value["ironwood"]["actions"][i]["output"]["ock"]);
    assert_eq!(
        engine().begin(&encode(value)).unwrap_err(),
        Error("outgoing key mismatch")
    );
}

#[test]
fn zero_value_output_cannot_hide_nonempty_memo() {
    let memo = zcash_protocol::memo::MemoBytes::from_bytes(b"hidden text").unwrap();
    let bytes = build(0, 990_000, memo, false);
    assert_eq!(
        engine().begin(&bytes).unwrap_err(),
        Error("nonempty memo unsupported")
    );
}

#[test]
fn upstream_digest_assembly_matches_standard_signer_across_action_bounds() {
    for outputs in 1..=8 {
        let bytes = build_actions(outputs);
        let pczt = Pczt::parse(&bytes).unwrap();
        assert_eq!(pczt.ironwood().actions().len(), outputs);
        // Full validation establishes that this transaction belongs to profile 1.
        let review = engine().begin(&bytes).unwrap();
        assert_eq!(review.projection().outputs.len(), outputs);
        let standard = Signer::new(pczt).unwrap().shielded_sighash();
        assert_eq!(&standard, review.sighash());

        let mut value = json(&bytes);
        value["ironwood"]["anchor"] = json!(vec![0; 32]);
        let reanchored = encode(value);
        let reanchored_review = engine().begin(&reanchored).unwrap();
        let reanchored_standard = Signer::new(Pczt::parse(&reanchored).unwrap())
            .unwrap()
            .shielded_sighash();
        assert_eq!(&reanchored_standard, reanchored_review.sighash());
        assert_eq!(reanchored_standard, standard);
    }
}

#[test]
fn multiple_owned_inputs_produce_all_and_only_verified_signatures() {
    for inputs in [2, 8] {
        let bytes = build_inputs(&vec![100_000; inputs]);
        let mut e = engine();
        let review = e.begin(&bytes).unwrap();
        assert_eq!(review.projection().total_input, inputs as u64 * 100_000);
        assert_eq!(review.projection().fee, inputs as u64 * 5_000);
        e.approve(review.token()).unwrap();
        let signed = e.sign(review.token(), &keys().1).unwrap();
        assert_eq!(signed.signatures.len(), inputs);
        for (i, signature) in signed.signatures.iter().enumerate() {
            assert_eq!(signature.action_index(), i);
            assert_eq!(signature.value_pool(), orchard::ValuePool::Ironwood);
        }
        let mut receiver = Signer::new(Pczt::parse(&bytes).unwrap()).unwrap();
        for signature in signed.signatures {
            let signature = pczt::roles::signer::SpendAuthSignature::from_parts(
                signature.value_pool(),
                signature.action_index(),
                *signature.signature(),
            );
            receiver
                .apply_orchard_spend_auth_signature(&signature)
                .unwrap();
        }
        assert_eq!(receiver.shielded_sighash(), signed.sighash);
        assert!(e.sign(review.token(), &keys().1).is_err());
    }
}

#[test]
fn valid_individual_notes_cannot_overflow_total_input_policy() {
    let maximum = zcash_protocol::value::MAX_MONEY;
    let bytes = build_inputs(&[maximum / 2, maximum / 2 + 1]);
    // These note values, encodings and net balance are individually representable.
    preflight(&bytes).unwrap();
    Signer::new(Pczt::parse(&bytes).unwrap()).unwrap();
    assert_eq!(
        engine().begin(&bytes).unwrap_err(),
        Error("accounting overflow")
    );
}

#[test]
fn equivalent_zero_lock_times_keep_digest_but_require_new_consent() {
    let mut value = json(&fixture());
    value["global"]["fallback_lock_time"] = Value::Null;
    let absent = encode(value.clone());
    value["global"]["fallback_lock_time"] = json!(0);
    let explicit = encode(value);
    assert_ne!(absent, explicit);
    let digest = Signer::new(Pczt::parse(&absent).unwrap())
        .unwrap()
        .shielded_sighash();
    assert_eq!(
        Signer::new(Pczt::parse(&explicit).unwrap())
            .unwrap()
            .shielded_sighash(),
        digest
    );
    let mut e = engine();
    let old = e.begin(&absent).unwrap();
    assert_eq!(*old.sighash(), digest);
    e.approve(old.token()).unwrap();
    let new = e.begin(&explicit).unwrap();
    assert_eq!(*new.sighash(), digest);
    assert_ne!(old.token().context(), new.token().context());
    // Test the new token first: the replacement itself must not inherit consent.
    assert!(e.sign(new.token(), &keys().1).is_err());
    assert!(e.sign(old.token(), &keys().1).is_err());
    let approved = e.begin(&explicit).unwrap();
    e.approve(approved.token()).unwrap();
    let signed = e.sign(approved.token(), &keys().1).unwrap();
    let mut oracle = Signer::new(Pczt::parse(&explicit).unwrap()).unwrap();
    for signature in signed.signatures {
        oracle
            .apply_orchard_spend_auth_signature(
                &pczt::roles::signer::SpendAuthSignature::from_parts(
                    signature.value_pool(),
                    signature.action_index(),
                    *signature.signature(),
                ),
            )
            .unwrap();
    }
    assert_eq!(signed.sighash, digest);
}
