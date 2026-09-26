//! The pools around the Ironwood bundle, its trailer, and its size.

use orchard::bundle::BundleVersion;
use pasta_curves::group::ff::PrimeField;
use pasta_curves::pallas;
use serde_json::{Value, json};
use zcash_protocol::value::MAX_MONEY;
use zcash_signer::Error::{Amount, Capacity, Malformed, Policy};
use zcash_signer::{Error, MAX_ACTIONS, MAX_PCZT_BYTES};
use zcash_signer_tests::*;

fn sapling(value_sum: i64, full: bool) -> Value {
    let optional = if full {
        json!(vec![9u8; 32])
    } else {
        Value::Null
    };
    json!({
        "spends": [],
        "outputs": [],
        "value_sum": value_sum,
        "anchor": optional,
        "bsk": optional,
    })
}

/// Asserts the outcome of each edit of `Tx::two_notes`, which has no dummy
/// spend whose signature would refuse any edit of the sighash first. An
/// accepted edit is outside the sighash: the signatures still apply to the
/// original.
fn assert_edits(edits: &[(&str, Edit, Outcome)]) {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::two_notes()).bytes;
    for (name, edit, expected) in edits {
        let mutated = mutate(&bytes, edit);
        assert_outcome(&wallet, name, &mutated, *expected);
        if expected.is_ok() {
            let signatures = sign(&wallet, &mutated).unwrap();
            assert_signatures_apply(&bytes, &signatures);
        }
    }
}

#[test]
fn test_sapling_bundle() {
    assert_edits(&[
        ("empty", |v| v["sapling"] = sapling(0, false), Ok(())),
        (
            "empty, with anchor and bsk",
            |v| v["sapling"] = sapling(0, true),
            Ok(()),
        ),
        (
            "value sum",
            |v| v["sapling"] = sapling(1, false),
            Err(Policy),
        ),
    ]);
}

#[test]
fn test_other_pools() {
    assert_edits(&[
        (
            "Orchard bundle",
            |v| v["orchard"] = v["ironwood"].clone(),
            Err(Policy),
        ),
        (
            "transparent bundle without outputs",
            |v| v["transparent"] = json!({ "inputs": [], "outputs": [] }),
            Err(Policy),
        ),
    ]);
}

#[test]
fn test_action_count() {
    assert_edits(&[
        (
            "no Ironwood bundle",
            |v| v["ironwood"] = Value::Null,
            Err(Policy),
        ),
        (
            "no actions",
            |v| v["ironwood"]["actions"] = json!([]),
            Err(Policy),
        ),
        (
            "too many actions",
            |v| {
                v["ironwood"]["actions"] =
                    json!(vec![v["ironwood"]["actions"][0].clone(); MAX_ACTIONS + 1])
            },
            Err(Capacity),
        ),
    ]);
}

#[test]
fn test_flags() {
    assert_edits(&[
        ("zero", |v| v["ironwood"]["flags"] = json!(0), Err(Policy)),
        (
            "reserved flag",
            |v| v["ironwood"]["flags"] = json!(0x87),
            Err(Malformed),
        ),
    ]);
}

#[test]
fn test_value_sum() {
    assert_edits(&[
        (
            "off by one",
            |v| v["ironwood"]["value_sum"][0] = json!(10_001),
            Err(Malformed),
        ),
        (
            "negative",
            |v| v["ironwood"]["value_sum"][1] = json!(true),
            Err(Policy),
        ),
        (
            "past MAX_MONEY",
            |v| v["ironwood"]["value_sum"][0] = json!(MAX_MONEY + 1),
            Err(Malformed),
        ),
    ]);
}

#[test]
fn test_anchor() {
    assert_edits(&[
        (
            "zero",
            |v| v["ironwood"]["anchor"] = json!(vec![0u8; 32]),
            Ok(()),
        ),
        (
            "not a field element",
            |v| v["ironwood"]["anchor"] = json!(vec![0xffu8; 32]),
            Err(Malformed),
        ),
    ]);
}

#[test]
fn test_trailer_fields() {
    assert_edits(&[
        (
            "note version V2",
            |v| v["ironwood"]["note_version"] = json!("V2"),
            Err(Policy),
        ),
        (
            "proof",
            |v| v["ironwood"]["zkproof"] = json!(vec![0u8; 4]),
            Err(Policy),
        ),
        (
            "bsk",
            |v| v["ironwood"]["bsk"] = json!(vec![9u8; 32]),
            Ok(()),
        ),
    ]);
}

/// The largest canonical little-endian encoding of a field element, and the
/// modulus.
fn bounds<F: PrimeField<Repr = [u8; 32]>>() -> ([u8; 32], [u8; 32]) {
    let largest = (-F::ONE).to_repr();
    let mut modulus = largest;
    modulus[0] += 1; // `largest` is even, so this does not carry
    (largest, modulus)
}

/// librustzcash parses the binding keys and the Sapling anchor, which only the
/// host uses, and refuses non-canonical ones.
#[test]
fn test_unused_fields_parse() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::two_notes()).bytes;
    let fields = [
        ("sapling", "anchor", bounds::<jubjub::Base>()),
        ("sapling", "bsk", bounds::<jubjub::Fr>()),
        ("ironwood", "bsk", bounds::<pallas::Scalar>()),
    ];
    for (pool, field, (largest, modulus)) in fields {
        for (label, value, expected) in [
            ("largest", largest, Ok(())),
            ("modulus", modulus, Err(Malformed)),
            ("all ones", [0xff; 32], Err(Malformed)),
        ] {
            let name = &format!("{pool} {field} {label}");
            let mutated = mutate(&bytes, |v| {
                v["sapling"] = sapling(0, true);
                v[pool][field] = json!(value);
            });
            assert_eq!(
                librustzcash_verifies(&mutated, &wallet.fvk),
                expected.is_ok(),
                "{name}"
            );
            assert_outcome(&wallet, name, &mutated, expected);
        }
    }
}

/// The trailer is checked with orchard's flag parser against the default
/// flags, which permit cross-address transfers, so the Session need not check
/// the restriction.
#[test]
fn test_default_flags() {
    let version = BundleVersion::ironwood_v3();
    assert!(version.default_flags().cross_address_enabled());
    assert_eq!(version.default_flags().to_byte(version), Some(0b111));
}

#[test]
fn test_sapling_spends_and_outputs() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::two_notes()).bytes;
    let with_sapling = mutate(&bytes, |v| v["sapling"] = sapling(0, false));
    let tag = offset_of_difference(&bytes, &with_sapling);
    for (name, count) in [("spend", tag + 1), ("output", tag + 2)] {
        let mut mutated = with_sapling.clone();
        mutated[count] = 1;
        assert_outcome(&wallet, name, &mutated, Err(Policy));
    }
}

#[test]
fn test_fee_limit() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    for (max_fee, expected) in [(10_000, Ok(())), (9_999, Err(Policy))] {
        let policy = zcash_signer::Policy::new(
            wallet.network,
            wallet.account,
            HEIGHT,
            max_fee,
            EXPIRY_WINDOW,
        )
        .unwrap();
        let session = wallet.session_with(policy, 0);
        let outcome = sign_with(&wallet, session, &bytes).map(|_| ());
        assert_eq!(outcome, expected, "{max_fee}");
    }
}

/// Each note is within the money range, their sum is not.
#[test]
fn test_input_total_past_max_money() {
    let wallet = Wallet::testnet();
    let tx = Tx {
        notes: vec![MAX_MONEY / 2, MAX_MONEY / 2 + 1],
        ..Tx::pay(vec![Output::payment(MAX_MONEY - 9_999)], Vec::new(), 10_000)
    };
    let bytes = build(&wallet, &tx).bytes;
    assert!(librustzcash_verifies(&bytes, &wallet.fvk));
    assert_outcome(&wallet, "overflow", &bytes, Err(Amount));
}

/// Varints read to their tenth byte before the verdict, in the longest
/// shielded prefix and trailer.
#[test]
fn test_ten_byte_varints() {
    let wallet = Wallet::testnet();
    let tx = Tx {
        view: View::Full,
        ..Tx::two_notes()
    };
    let bytes = mutate(&build(&wallet, &tx).bytes, |v| {
        v["ironwood"]["anchor"] = json!(vec![0u8; 32]);
    });
    let fields: [(&str, Edit, &[u8], Error); 4] = [
        (
            "Sapling value sum",
            |v| v["sapling"]["value_sum"] = json!(1),
            &varint(1 << 63),
            Policy,
        ),
        (
            "action count",
            |v| v["ironwood"]["actions"] = json!([v["ironwood"]["actions"][0]]),
            &varint(1 << 63),
            Capacity,
        ),
        (
            "note version",
            |v| v["ironwood"]["note_version"] = json!("V2"),
            &varint(1 << 63),
            Policy,
        ),
        (
            "note version, overlong",
            |v| v["ironwood"]["note_version"] = json!("V2"),
            &[0x81, 0],
            Malformed,
        ),
    ];
    for (name, edit, wide, expected) in fields {
        let at = offset_of_difference(&bytes, &mutate(&bytes, edit));
        let mutated = [&bytes[..at], wide, &bytes[at + 1..]].concat();
        assert_outcome(&wallet, name, &mutated, Err(expected));
    }
}

/// A PCZT of `MAX_ACTIONS` actions with every optional field present at its
/// longest fits `MAX_PCZT_BYTES`, and signs.
#[test]
fn test_longest_pczt() {
    let wallet = Wallet::testnet();
    let outputs = (0..MAX_ACTIONS)
        .map(|_| Output {
            user_address: Some("u".repeat(512)),
            ..Output::payment(1_000_000)
        })
        .collect();
    let tx = Tx {
        notes: vec![1_010_000; MAX_ACTIONS],
        view: View::Full,
        ..Tx::pay(outputs, Vec::new(), 0)
    };
    let built = build(&wallet, &tx);
    let claim = derivation_json(&wallet.seed_fingerprint, &wallet.account_path());
    let bytes = mutate(&with_ocks(&built.bytes, &wallet.fvk), |v| {
        v["ironwood"]["anchor"] = json!(vec![0u8; 32]);
        for index in 0..MAX_ACTIONS {
            let action = &mut v["ironwood"]["actions"][index];
            action["spend"]["zip32_derivation"] = claim.clone();
            action["output"]["zip32_derivation"] = claim.clone();
        }
    });
    assert!(bytes.len() <= MAX_PCZT_BYTES, "{}", bytes.len());
    let signatures = sign(&wallet, &bytes).unwrap();
    assert_eq!(signatures.records().len(), MAX_ACTIONS);
    assert_signatures_apply(&bytes, &signatures);
}
