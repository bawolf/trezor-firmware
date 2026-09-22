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
    Account, Engine, ErrorCode, Limits, MAX_ACTIONS, MAX_PCZT_BYTES, MEMO_TEXT_BUDGET, Memo,
    Network, OutputKind, Policy, RequestContext,
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
fn host_change_labels_rejected_and_user_address_ignored() {
    let claimed_change = mutate(|v| {
        let i = payment(v);
        v["ironwood"]["actions"][i]["output"]["proprietary"] = json!({"change": [1]});
    });
    assert!(preflight(&claimed_change).is_err());
    // The stock SDK stamps the recipient string the wallet showed its user on
    // every payment output. It is admitted and ignored: the projection and
    // the sighash are those of the same bundle without it, and the receiver
    // shown is the one the device verified.
    let address = mutate(|v| {
        let i = payment(v);
        v["ironwood"]["actions"][i]["output"]["user_address"] = "attacker change".into();
    });
    let plain = engine().begin_test(&fixture()).unwrap();
    let labelled = engine().begin_test(&address).unwrap();
    assert_eq!(labelled.projection(), plain.projection());
    assert_eq!(labelled.sighash(), plain.sighash());
    assert_eq!(labelled.projection().change_total, 390_000);
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

/// The stock SDK default `OvkPolicy::Sender` encrypts change with no OVK
/// (`internal_ovk: None`). The receiver is classified internal by the device's
/// own IVK and the note is bound by the commitment and pk_d/esk recovery, so
/// the output is admitted as change; a payment without an OVK stays refused
/// (`nonzero_output_with_discarded_ovk_is_rejected`) and change under the
/// external OVK stays refused
/// (`internal_change_requires_internal_outgoing_viewing_key`).
#[test]
fn change_without_ovk_is_accepted() {
    let review = engine()
        .begin_test(&build_with_change_without_ovk())
        .unwrap();
    assert_eq!(review.projection().change_total, 390_000);
    assert_eq!(review.projection().payment_total, 600_000);
    assert_eq!(review.projection().outputs.len(), 2);
}

/// What a stock-SDK wallet hands over unmodified signs: change without an OVK,
/// the Full view's empty Sapling bundle and Ironwood `bsk`, and the
/// `user_address` on the payment. The `bsk` and the Sapling bundle are outside
/// the projection and the sighash is the bundle's own.
#[test]
fn stock_sdk_full_view_is_accepted_and_signs() {
    let bytes = build_stock_sdk_view();
    let view = json(&bytes);
    assert!(
        view["sapling"].is_object(),
        "Full view keeps the Sapling bundle"
    );
    assert!(
        view["ironwood"]["bsk"].is_array(),
        "Full view keeps the bsk"
    );
    let mut e = engine();
    let review = e.begin_test(&bytes).unwrap();
    assert_eq!(review.projection().payment_total, 600_000);
    assert_eq!(review.projection().change_total, 390_000);
    e.approve(review.token()).unwrap();
    let signed = e.sign(review.token(), &keys().1).unwrap();
    assert_eq!(signatures(&signed).len(), 1);
}

/// A `zip32_derivation` naming the device's own seed fingerprint and the
/// consented `m/32'/coin_type'/account'` is admitted on spends and outputs
/// and changes nothing; absent stays admitted. Zodl-style accounts imported
/// as `Spending { seed_fingerprint, index }` put it on every real spend.
#[test]
fn zip32_derivation_matching_the_device_is_accepted() {
    let plain = engine().begin_test(&fixture()).unwrap();
    for on_change in [false, true] {
        let review = engine()
            .begin_test(&with_own_derivation(on_change))
            .unwrap();
        assert_eq!(review.projection(), plain.projection(), "{on_change}");
        assert_eq!(review.sighash(), plain.sighash(), "{on_change}");
    }
}

/// Any other derivation claim is refused: a host cannot make the device sign
/// under a seed or an account the user did not consent to. A non-hardened
/// index is what the orchard parser refuses, so it is `Malformed` like any
/// other unparseable field.
#[test]
fn zip32_derivation_not_the_device_own_is_rejected() {
    let own = own_path(Network::Testnet);
    let other_seed = [0x5f; 32];
    let mut other_account = own;
    other_account[2] += 1;
    let mut other_coin = own;
    other_coin[1] += 1;
    let mut other_purpose = own;
    other_purpose[0] += 1;
    let mut non_hardened = own;
    non_hardened[2] = ACCOUNT;
    let cases: [(&str, [u8; 32], &[u32], ErrorCode); 8] = [
        ("other seed", other_seed, &own, ErrorCode::Policy),
        ("other account", SEED_FINGERPRINT, &other_account, ErrorCode::Policy),
        ("other coin type", SEED_FINGERPRINT, &other_coin, ErrorCode::Policy),
        ("other purpose", SEED_FINGERPRINT, &other_purpose, ErrorCode::Policy),
        ("short path", SEED_FINGERPRINT, &own[..2], ErrorCode::Policy),
        ("empty path", SEED_FINGERPRINT, &[], ErrorCode::Policy),
        (
            "long path",
            SEED_FINGERPRINT,
            &[own[0], own[1], own[2], own[2]],
            ErrorCode::Policy,
        ),
        ("non-hardened account", SEED_FINGERPRINT, &non_hardened, ErrorCode::Malformed),
    ];
    for (name, fingerprint, path, code) in cases {
        for part in ["spend", "output"] {
            let bytes = mutate(|v| {
                let i = real(v);
                v["ironwood"]["actions"][i][part]["zip32_derivation"] =
                    derivation_json(&fingerprint, path);
            });
            assert_error(
                engine().begin_test(&bytes).unwrap_err(),
                code,
            );
            let _ = name;
        }
    }
}

/// The Ironwood `bsk` the Full view keeps is ignored: same projection, same
/// sighash, same signatures as without it.
#[test]
fn ironwood_bsk_is_ignored() {
    let with_bsk = mutate(|v| v["ironwood"]["bsk"] = json!(vec![7u8; 32]));
    let plain = engine().begin_test(&fixture()).unwrap();
    let ignored = engine().begin_test(&with_bsk).unwrap();
    assert_eq!(ignored.projection(), plain.projection());
    assert_eq!(ignored.sighash(), plain.sighash());
}
/// Design §11: a payment's memo is shown. Text within the budget verbatim;
/// the projection carries what the device shows, recovered from the
/// `enc_ciphertext` the sighash covers.
#[test]
fn text_memo_on_payment_is_shown() {
    let memo = zcash_protocol::memo::MemoBytes::from_bytes(b"hello").unwrap();
    let review = engine()
        .begin_test(&build(600_000, 390_000, memo, false))
        .unwrap();
    let payment = review
        .projection()
        .outputs
        .iter()
        .find(|o| o.kind == OutputKind::Payment)
        .unwrap();
    match &payment.memo {
        Memo::Text(text) => assert_eq!(text.as_str(), "hello"),
        other => panic!("{other:?}"),
    }
    let change = review
        .projection()
        .outputs
        .iter()
        .find(|o| o.kind == OutputKind::InternalChange)
        .unwrap();
    assert_eq!(change.memo, Memo::Empty);
}

fn payment_memo(bytes: &[u8]) -> Memo {
    engine()
        .begin_test(bytes)
        .unwrap()
        .projection()
        .outputs
        .iter()
        .find(|o| o.kind == OutputKind::Payment)
        .unwrap()
        .memo
        .clone()
}

fn memo_digest(memo: &zcash_protocol::memo::MemoBytes) -> Memo {
    let mut digest = [0; 32];
    digest.copy_from_slice(
        blake2b_simd::Params::new()
            .hash_length(32)
            .hash(memo.as_array())
            .as_bytes(),
    );
    Memo::Digest(digest)
}

/// Text at the budget is shown; one byte over, binary, reserved, non-UTF-8 or
/// NUL-bearing memos are shown as the BLAKE2b-256 of the 512 memo bytes.
#[test]
fn memo_budget_and_binary_memos_are_hashed() {
    use zcash_protocol::memo::MemoBytes;
    let at_budget = MemoBytes::from_bytes(&[b'a'; MEMO_TEXT_BUDGET]).unwrap();
    match payment_memo(&build(600_000, 390_000, at_budget, false)) {
        Memo::Text(text) => assert_eq!(text.as_str().len(), MEMO_TEXT_BUDGET),
        other => panic!("{other:?}"),
    }
    let utf8 = MemoBytes::from_bytes("Zodl ✓ café ☕ — thanks!".as_bytes()).unwrap();
    match payment_memo(&build(600_000, 390_000, utf8, false)) {
        Memo::Text(text) => assert_eq!(text.as_str(), "Zodl ✓ café ☕ — thanks!"),
        other => panic!("{other:?}"),
    }
    let over_budget = MemoBytes::from_bytes(&[b'a'; MEMO_TEXT_BUDGET + 1]).unwrap();
    let mut arbitrary = [0x41u8; 512];
    arbitrary[0] = 0xff;
    let mut reserved = [0u8; 512];
    reserved[0] = 0xf5;
    let mut future = [0u8; 512];
    future[0] = 0xf6;
    future[1] = 1;
    let mut interior_nul = [0u8; 512];
    interior_nul[..3].copy_from_slice(b"a\0b");
    for memo in [
        over_budget,
        MemoBytes::from_bytes(&[b'a'; 512]).unwrap(),
        MemoBytes::from_bytes(&[0xc3, 0x28]).unwrap(),
        MemoBytes::from_bytes(&arbitrary).unwrap(),
        MemoBytes::from_bytes(&reserved).unwrap(),
        MemoBytes::from_bytes(&future).unwrap(),
        MemoBytes::from_bytes(&interior_nul).unwrap(),
    ] {
        assert_eq!(
            payment_memo(&build(600_000, 390_000, memo.clone(), false)),
            memo_digest(&memo)
        );
    }
    // The all-zero empty text memo is nothing to show.
    assert_eq!(
        payment_memo(&build(
            600_000,
            390_000,
            MemoBytes::from_bytes(&[0; 512]).unwrap(),
            false
        )),
        Memo::Empty
    );
}

/// Memos of hidden outputs must be empty: change with a memo is refused.
#[test]
fn change_output_with_memo_is_rejected() {
    let memo = zcash_protocol::memo::MemoBytes::from_bytes(b"hidden").unwrap();
    assert_error(
        engine().begin_test(&build_with_change_memo(memo)).unwrap_err(),
        ErrorCode::Policy,
    );
}

/// The memo shown is the memo signed: it is recovered from the
/// `enc_ciphertext` the digest hashes, so altering a memo byte on the wire
/// is a recovery failure, and a different memo is a different sighash.
#[test]
fn shown_memo_is_bound_to_the_signed_output() {
    use zcash_protocol::memo::MemoBytes;
    let hello = build(
        600_000,
        390_000,
        MemoBytes::from_bytes(b"hello").unwrap(),
        false,
    );
    let hallo = build(
        600_000,
        390_000,
        MemoBytes::from_bytes(b"hallo").unwrap(),
        false,
    );
    let a = engine().begin_test(&hello).unwrap();
    let b = engine().begin_test(&hallo).unwrap();
    assert_ne!(a.sighash(), b.sighash());
    // The memo occupies the ciphertext after the 52-byte note plaintext
    // (version, diversifier, value, rseed); flip one of its bytes.
    let mut value = json(&hello);
    let i = payment(&value);
    let ciphertext = &mut value["ironwood"]["actions"][i]["output"]["enc_ciphertext"]["Encrypted"];
    let byte = ciphertext[60].as_u64().unwrap();
    ciphertext[60] = (byte ^ 1).into();
    assert_error(
        engine().begin_test(&encode(value)).unwrap_err(),
        ErrorCode::Malformed,
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
fn mixed_pools_and_nonempty_bundles_rejected_empty_sapling_admitted() {
    for pool in ["orchard", "transparent"] {
        let bytes = mutate(|v| {
            v[pool] = match pool {
                "orchard" => v["ironwood"].clone(),
                _ => json!({"inputs":[], "outputs":[]}),
            };
        });
        assert!(preflight(&bytes).is_err(), "accepted {pool}");
    }
    // An empty Sapling bundle is what the Full signer view keeps, with or
    // without its anchor and `bsk`; it is admitted and changes nothing.
    let plain = engine().begin_test(&fixture()).unwrap();
    for (name, anchor, bsk) in [
        ("bare", Value::Null, Value::Null),
        ("full view", json!(vec![0u8; 32]), json!(vec![9u8; 32])),
    ] {
        let bytes = mutate(|v| {
            v["sapling"] =
                json!({"spends":[], "outputs":[], "value_sum":0, "anchor":anchor, "bsk":bsk});
        });
        let review = engine().begin_test(&bytes).unwrap();
        assert_eq!(review.projection(), plain.projection(), "{name}");
        assert_eq!(review.sighash(), plain.sighash(), "{name}");
    }
    // Anything in it is not.
    let nonzero_sum = mutate(|v| {
        v["sapling"] = json!({"spends":[], "outputs":[], "value_sum":1, "anchor":null, "bsk":null});
    });
    assert_error(
        engine().begin_test(&nonzero_sum).unwrap_err(),
        ErrorCode::Policy,
    );
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
fn over_action_cap_is_clean_capacity_not_malformed() {
    // Guardrail: a well-formed bundle with more than MAX_ACTIONS (32) actions
    // must reject as `Capacity` — a clean, user-comprehensible "too many
    // actions" class the adapter surfaces as such — and specifically NOT as a
    // raw `Malformed`. See `ironwood_signing::Failure::Capacity` and the native
    // `failure()` map for the user-facing message.
    let over_cap = mutate(|v| {
        let action = v["ironwood"]["actions"][0].clone();
        v["ironwood"]["actions"] = vec![action; MAX_ACTIONS + 1].into();
    });
    let code = preflight(&over_cap).unwrap_err().code();
    assert_eq!(code, ErrorCode::Capacity);
    assert_ne!(code, ErrorCode::Malformed);
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
