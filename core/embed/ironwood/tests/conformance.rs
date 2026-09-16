#![cfg(feature = "test")]

mod action_bounds;
mod common;
mod randomness;
use common::*;
use pczt::Pczt;
use pczt::roles::signer::Signer;
use pczt::roles::verifier::{OrchardError, Verifier};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde_json::{Value, json};
use trezor_ironwood::testing::preflight;
use trezor_ironwood::{
    Account, Engine, ErrorCode, Limits, MAX_ACTIONS, MAX_PCZT_BYTES, Network, OutputKind, Policy,
    RequestContext,
};
use zcash_protocol::consensus::BranchId;

#[test]
fn positive_payment_change_and_verified_signatures() {
    let bytes = fixture();
    preflight(&bytes).unwrap();
    let mut engine = engine();
    let review = engine.begin_test(&bytes).unwrap();
    let p = review.projection();
    assert_eq!(
        (p.input_total, p.payment_total, p.change_total, p.fee),
        (1_000_000, 600_000, 390_000, 10_000)
    );
    assert_eq!(p.network, Network::Testnet);
    assert_eq!(p.account, Account::new(ACCOUNT).unwrap());
    assert_eq!(p.host_reference_height, HEIGHT);
    assert_eq!(p.outputs.len(), 2);
    assert!(
        p.outputs
            .iter()
            .any(|o| o.value == 390_000 && o.kind == OutputKind::InternalChange)
    );
    engine.approve(review.token()).unwrap();
    let digest = *review.sighash();
    let signed = engine.sign(review.token(), &keys().1).unwrap();
    assert_eq!(signatures(&signed).len(), 1);
    assert_eq!(
        Signer::new(signed.clone()).unwrap().shielded_sighash(),
        digest
    );
    Verifier::new(signed)
        .with_ironwood(|bundle| -> Result<(), OrchardError<()>> {
            for action in bundle.actions() {
                action
                    .spend()
                    .rk()
                    .verify(&digest, action.spend().spend_auth_sig().as_ref().unwrap())
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
    let r = e.begin_test(&fixture()).unwrap();
    assert!(e.sign(r.token(), &keys().1).is_err());
    assert!(e.approve(r.token()).is_err());
}
#[test]
fn cancellation_and_duplicate_approval_consume_consent() {
    let mut e = engine();
    let r = e.begin_test(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    e.cancel();
    assert!(e.sign(r.token(), &keys().1).is_err());
    let r = e.begin_test(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    assert!(e.approve(r.token()).is_err());
    assert!(e.sign(r.token(), &keys().1).is_err());
}
#[test]
fn wrong_key_failure_has_no_retry_or_partial_response() {
    let mut e = engine();
    let r = e.begin_test(&fixture()).unwrap();
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
    let r = e.begin_test(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    let r2 = e.begin_test(&fixture()).unwrap();
    assert_ne!(r.token(), r2.token());
    assert!(e.approve(r.token()).is_err());
    assert!(e.sign(r2.token(), &keys().1).is_err());
    let mut other = engine();
    other.begin_test(&fixture()).unwrap();
    assert!(other.approve(r2.token()).is_err());
}
#[test]
fn malformed_replacement_clears_previous_approval() {
    let mut e = engine();
    let r = e.begin_test(&fixture()).unwrap();
    e.approve(r.token()).unwrap();
    assert!(e.begin_test(b"PCZT").is_err());
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
    let old = e.begin_test(&bytes).unwrap();
    e.approve(old.token()).unwrap();
    let new = e.begin_test(&altered).unwrap();
    assert_eq!(old.sighash(), new.sighash());
    assert_ne!(old.token().context(), new.token().context());
    assert!(e.sign(old.token(), &keys().1).is_err());
    assert!(e.sign(new.token(), &keys().1).is_err());
}
#[test]
fn caller_mutation_cannot_change_owned_signing_context() {
    let mut bytes = fixture();
    let mut e = engine();
    let r = e.begin_test(&bytes).unwrap();
    bytes.fill(0);
    e.approve(r.token()).unwrap();
    assert_eq!(
        &Signer::new(e.sign(r.token(), &keys().1).unwrap())
            .unwrap()
            .shielded_sighash(),
        r.sighash(),
    );
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
        assert!(
            engine().begin_test(&bytes).is_err(),
            "accepted output {field}"
        );
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
        assert!(
            engine().begin_test(&bytes).is_err(),
            "accepted spend {field}"
        );
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
        assert!(engine().begin_test(&bytes).is_err(), "accepted {field}");
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
    let r = engine().begin_test(&bytes).unwrap();
    assert_eq!(r.projection().payment_total, 600_000);
    assert_eq!(r.projection().change_total, 390_000);
}

#[test]
fn internal_change_requires_internal_outgoing_viewing_key() {
    let review = engine().begin_test(&fixture()).unwrap();
    assert_eq!(review.projection().change_total, 390_000);
    assert_error(
        engine()
            .begin_test(&build_with_wrong_change_ovk())
            .unwrap_err(),
        ErrorCode::Malformed,
    );
}

#[test]
fn nonzero_output_with_discarded_ovk_is_rejected() {
    assert_error(
        engine()
            .begin_test(&build_with_discarded_payment_ovk())
            .unwrap_err(),
        ErrorCode::Malformed,
    );
}
#[test]
fn nonempty_memo_rejected_even_when_encryption_is_valid() {
    let memo = zcash_protocol::memo::MemoBytes::from_bytes(b"hello").unwrap();
    assert_error(
        engine()
            .begin_test(&build(600_000, 390_000, memo, false))
            .unwrap_err(),
        ErrorCode::Policy,
    );
}
#[test]
fn fee_cap_and_declared_balance_rejected() {
    let mut e = Engine::with_rng(
        policy(Network::Testnet, 9_999),
        ChaCha20Rng::from_seed([0x51; 32]),
    )
    .unwrap();
    assert_error(e.begin_test(&fixture()).unwrap_err(), ErrorCode::Policy);
    let bytes = mutate(|v| v["ironwood"]["value_sum"][0] = 10_001.into());
    assert!(engine().begin_test(&bytes).is_err());
    let bytes = mutate(|v| v["ironwood"]["value_sum"][1] = true.into());
    assert_eq!(preflight(&bytes).unwrap_err().code(), ErrorCode::Policy);
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
        assert!(
            engine().begin_test(&bytes).is_err(),
            "accepted {field}={value}"
        );
    }
}

#[test]
fn production_network_branch_height_and_account_are_validated_and_bound() {
    for network in [Network::Mainnet, Network::Testnet] {
        let bytes = network_fixture(network);
        let mut e = Engine::with_rng(
            policy(network, 100_000),
            ChaCha20Rng::from_seed([network as u8 + 0x60; 32]),
        )
        .unwrap();
        let review = e.begin_test(&bytes).unwrap();
        assert_eq!(review.projection().network, network);
        assert_eq!(review.projection().account.value(), ACCOUNT);
        assert_eq!(review.projection().host_reference_height, HEIGHT);
        assert_eq!(
            review.projection().consensus_branch_id,
            u32::from(BranchId::Nu6_3)
        );

        let opposite = match network {
            Network::Mainnet => Network::Testnet,
            Network::Testnet => Network::Mainnet,
        };
        let mut wrong_network = Engine::with_rng(
            policy(opposite, 100_000),
            ChaCha20Rng::from_seed([network as u8 + 0x70; 32]),
        )
        .unwrap();
        assert_error(
            wrong_network.begin_test(&bytes).unwrap_err(),
            ErrorCode::Policy,
        );

        let wrong_branch = {
            let mut value = json(&bytes);
            value["global"]["consensus_branch_id"] = 0u32.into();
            encode(value)
        };
        assert_error(e.begin_test(&wrong_branch).unwrap_err(), ErrorCode::Policy);
    }

    for (network, pre_activation_height) in
        [(Network::Mainnet, 3_428_142), (Network::Testnet, 4_133_999)]
    {
        assert_error(
            Policy::new(
                RequestContext::new(network, Account::new(0).unwrap(), pre_activation_height),
                Limits::new(100_000, 100).unwrap(),
            )
            .unwrap_err(),
            ErrorCode::Policy,
        );
    }

    assert_eq!(Account::new(0).unwrap().value(), 0);
    assert_eq!(Account::new(Account::MAX).unwrap().value(), Account::MAX);
    assert_error(
        Account::new(Account::MAX + 1).unwrap_err(),
        ErrorCode::Policy,
    );

    let bytes = network_fixture(Network::Testnet);
    let limits = Limits::new(100_000, 100).unwrap();
    let make = |account| {
        Engine::with_rng(
            Policy::new(
                RequestContext::new(Network::Testnet, Account::new(account).unwrap(), HEIGHT),
                limits,
            )
            .unwrap(),
            ChaCha20Rng::from_seed([0x7f; 32]),
        )
        .unwrap()
    };
    let mut account_zero = make(0);
    let mut account_three = make(3);
    let zero = account_zero.begin_test(&bytes).unwrap();
    let three = account_three.begin_test(&bytes).unwrap();
    assert_ne!(zero.token().context(), three.token().context());

    let mut prior_height = Engine::with_rng(
        Policy::new(
            RequestContext::new(Network::Testnet, Account::new(3).unwrap(), HEIGHT - 1),
            limits,
        )
        .unwrap(),
        ChaCha20Rng::from_seed([0x7f; 32]),
    )
    .unwrap();
    let prior = prior_height.begin_test(&bytes).unwrap();
    assert_ne!(three.token().context(), prior.token().context());
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
    assert_error(
        engine().begin_test(&missing).unwrap_err(),
        ErrorCode::Malformed,
    );
    let bad = mutate(|v| {
        let i = 1 - real(v);
        flip(&mut v["ironwood"]["actions"][i]["spend"]["spend_auth_sig"]);
    });
    assert!(engine().begin_test(&bad).is_err());
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
    assert_error(
        engine().begin_test(&bytes).unwrap_err(),
        ErrorCode::Malformed,
    );
}
#[test]
fn action_byte_and_money_bounds_rejected() {
    assert_eq!(
        preflight(&vec![0; MAX_PCZT_BYTES + 1]).unwrap_err().code(),
        ErrorCode::Capacity,
    );
    let with_action_count = |count| {
        mutate(|v| {
            let a = v["ironwood"]["actions"][0].clone();
            v["ironwood"]["actions"] = vec![a; count].into();
        })
    };
    assert_eq!(
        preflight(&with_action_count(0)).unwrap_err().code(),
        ErrorCode::Policy,
    );
    assert_eq!(
        preflight(&with_action_count(MAX_ACTIONS + 1))
            .unwrap_err()
            .code(),
        ErrorCode::Capacity,
    );
    let no_ironwood = mutate(|v| v["ironwood"] = Value::Null);
    assert_eq!(
        preflight(&no_ironwood).unwrap_err().code(),
        ErrorCode::Policy,
    );
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
    assert_error(preflight(&bytes).unwrap_err(), ErrorCode::Malformed);
    let mut bytes = fixture();
    bytes[4] = 1;
    assert!(preflight(&bytes).is_err());
}

#[test]
fn zero_output_padding_is_verified_and_counted() {
    let bytes = build(990_000, 0, zcash_protocol::memo::MemoBytes::empty(), false);
    let review = engine().begin_test(&bytes).unwrap();
    assert_eq!(review.projection().padding_outputs, 1);
    assert_eq!(review.projection().payment_total, 990_000);
    assert_eq!(review.projection().change_total, 0);
}

#[test]
fn padding_with_dummy_spend_rejects_out_ciphertext_tampering() {
    let bytes = build_with_dummy_spend_padding();
    let mut altered = json(&bytes);
    let padding = altered["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|action| action["output"]["value"] == 0)
        .unwrap();
    assert_eq!(
        altered["ironwood"]["actions"][padding]["spend"]["value"].as_u64(),
        Some(0)
    );
    assert_eq!(
        altered["ironwood"]["actions"][padding]["spend"]["spend_auth_sig"]
            .as_array()
            .map(Vec::len),
        Some(64)
    );
    assert!(altered["ironwood"]["actions"][padding]["output"]["ock"].is_null());
    flip(&mut altered["ironwood"]["actions"][padding]["output"]["out_ciphertext"]);
    let altered = encode(altered);

    engine().begin_test(&bytes).unwrap();
    assert_error(
        engine().begin_test(&altered).unwrap_err(),
        ErrorCode::Malformed,
    );
}

#[test]
fn padding_without_dummy_spend_is_digest_and_consent_bound() {
    let bytes = build_inputs(&[100_000, 100_000]);
    let mut altered = json(&bytes);
    let padding = altered["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|action| action["output"]["value"] == 0)
        .unwrap();
    assert!(
        altered["ironwood"]["actions"][padding]["spend"]["value"]
            .as_u64()
            .is_some_and(|value| value > 0)
    );
    assert!(altered["ironwood"]["actions"][padding]["spend"]["spend_auth_sig"].is_null());
    assert!(altered["ironwood"]["actions"][padding]["output"]["ock"].is_null());
    flip(&mut altered["ironwood"]["actions"][padding]["output"]["out_ciphertext"]);
    let altered = encode(altered);

    let make = || {
        Engine::with_rng(
            policy(Network::Testnet, 100_000),
            ChaCha20Rng::from_seed([0x91; 32]),
        )
        .unwrap()
    };
    let mut original_engine = make();
    let original_review = original_engine.begin_test(&bytes).unwrap();
    let mut altered_engine = make();
    let altered_review = altered_engine.begin_test(&altered).unwrap();
    assert_ne!(original_review.sighash(), altered_review.sighash());
    assert_ne!(
        original_review.token().context(),
        altered_review.token().context()
    );

    original_engine.approve(original_review.token()).unwrap();
    altered_engine.approve(altered_review.token()).unwrap();
    assert_eq!(
        signatures(
            &original_engine
                .sign(original_review.token(), &keys().1)
                .unwrap()
        )
        .len(),
        2
    );
    assert_eq!(
        signatures(
            &altered_engine
                .sign(altered_review.token(), &keys().1)
                .unwrap()
        )
        .len(),
        2
    );
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
    use orchard::keys::Scope;
    use orchard::note_encryption::IronwoodDomain;
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
    engine().begin_test(&encode(value.clone())).unwrap();
    flip(&mut value["ironwood"]["actions"][i]["output"]["ock"]);
    assert_error(
        engine().begin_test(&encode(value)).unwrap_err(),
        ErrorCode::Malformed,
    );
}

#[test]
fn zero_value_output_cannot_hide_nonempty_memo() {
    let memo = zcash_protocol::memo::MemoBytes::from_bytes(b"hidden text").unwrap();
    let bytes = build(0, 990_000, memo, false);
    assert_error(engine().begin_test(&bytes).unwrap_err(), ErrorCode::Policy);
}

#[test]
fn upstream_digest_assembly_matches_standard_signer_across_action_bounds() {
    for outputs in 1..=8 {
        let bytes = build_actions(outputs);
        let pczt = Pczt::parse(&bytes).unwrap();
        assert_eq!(pczt.ironwood().actions().len(), outputs);
        // Full validation establishes that this transaction belongs to profile 1.
        let review = engine().begin_test(&bytes).unwrap();
        assert_eq!(review.projection().outputs.len(), outputs);
        let standard = Signer::new(pczt).unwrap().shielded_sighash();
        assert_eq!(&standard, review.sighash());

        let mut value = json(&bytes);
        value["ironwood"]["anchor"] = json!(vec![0; 32]);
        let reanchored = encode(value);
        let reanchored_review = engine().begin_test(&reanchored).unwrap();
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
        let review = e.begin_test(&bytes).unwrap();
        assert_eq!(review.projection().input_total, inputs as u64 * 100_000);
        assert_eq!(review.projection().fee, inputs as u64 * 5_000);
        let digest = *review.sighash();
        e.approve(review.token()).unwrap();
        let signed = e.sign(review.token(), &keys().1).unwrap();
        let signatures = signatures(&signed);
        assert_eq!(signatures.len(), inputs);
        assert_eq!(
            signatures.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            (0..inputs).collect::<Vec<_>>()
        );
        let mut receiver = Signer::new(Pczt::parse(&bytes).unwrap()).unwrap();
        for (index, signature) in signatures {
            let signature = pczt::roles::signer::SpendAuthSignature::from_parts(
                orchard::ValuePool::Ironwood,
                index,
                signature,
            );
            receiver
                .apply_orchard_spend_auth_signature(&signature)
                .unwrap();
        }
        assert_eq!(receiver.shielded_sighash(), digest);
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
    assert_error(
        engine().begin_test(&bytes).unwrap_err(),
        ErrorCode::Capacity,
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
    let old = e.begin_test(&absent).unwrap();
    assert_eq!(*old.sighash(), digest);
    e.approve(old.token()).unwrap();
    let new = e.begin_test(&explicit).unwrap();
    assert_eq!(*new.sighash(), digest);
    assert_ne!(old.token().context(), new.token().context());
    // Test the new token first: the replacement itself must not inherit consent.
    assert!(e.sign(new.token(), &keys().1).is_err());
    assert!(e.sign(old.token(), &keys().1).is_err());
    let approved = e.begin_test(&explicit).unwrap();
    e.approve(approved.token()).unwrap();
    let signed = e.sign(approved.token(), &keys().1).unwrap();
    let mut oracle = Signer::new(Pczt::parse(&explicit).unwrap()).unwrap();
    for (index, signature) in signatures(&signed) {
        oracle
            .apply_orchard_spend_auth_signature(
                &pczt::roles::signer::SpendAuthSignature::from_parts(
                    orchard::ValuePool::Ironwood,
                    index,
                    signature,
                ),
            )
            .unwrap();
    }
    assert_eq!(oracle.shielded_sighash(), digest);
}
