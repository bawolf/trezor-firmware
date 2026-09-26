//! Each action's spend and output: their encoding, their consistency, and the
//! claims a host makes about them.

use orchard::keys::SpendAuthorizingKey;
use orchard::primitives::redpallas::VerificationKey;
use pasta_curves::group::ff::{Field, PrimeField};
use pasta_curves::pallas;
use rand_chacha::ChaCha20Rng;
use rand_core::SeedableRng;
use serde_json::{Value, json};
use zcash_protocol::value::MAX_MONEY;
use zcash_signer::Error::{Malformed, Policy};
use zcash_signer::{Error, Event};
use zcash_signer_tests::*;

type Select = fn(&mut Value) -> &mut Value;

fn real(v: &mut Value) -> &mut Value {
    action(v, |a| a["spend"]["value"] != 0)
}

fn dummy(v: &mut Value) -> &mut Value {
    action(v, |a| a["spend"]["value"] == 0)
}

/// Asserts the outcome of each edit of `Tx::simple`: a real spend paying
/// 600_000 and 390_000 change, and a dummy spend.
fn assert_edits(edits: &[(&str, Edit)], expected: Error) {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    for (name, edit) in edits {
        assert_outcome(&wallet, name, &mutate(&bytes, edit), Err(expected));
    }
}

#[test]
fn test_real_spend_fields() {
    assert_edits(
        &[
            ("cv_net", |v| flip(&mut real(v)["cv_net"])),
            ("rcv", |v| flip(&mut real(v)["rcv"])),
            ("recipient", |v| flip(&mut real(v)["spend"]["recipient"])),
            ("value", |v| real(v)["spend"]["value"] = json!(1_000_001)),
            ("rho", |v| flip(&mut real(v)["spend"]["rho"])),
            ("rseed", |v| flip(&mut real(v)["spend"]["rseed"])),
            ("fvk", |v| flip(&mut real(v)["spend"]["fvk"])),
            ("alpha", |v| flip(&mut real(v)["spend"]["alpha"])),
            ("rk", |v| flip(&mut real(v)["spend"]["rk"])),
            ("nullifier", |v| flip(&mut real(v)["spend"]["nullifier"])),
        ],
        Malformed,
    );
}

#[test]
fn test_dummy_spend_fields() {
    assert_edits(
        &[
            ("recipient", |v| flip(&mut dummy(v)["spend"]["recipient"])),
            ("value", |v| dummy(v)["spend"]["value"] = json!(1)),
            ("rho", |v| flip(&mut dummy(v)["spend"]["rho"])),
            ("rseed", |v| flip(&mut dummy(v)["spend"]["rseed"])),
            ("fvk", |v| flip(&mut dummy(v)["spend"]["fvk"])),
            ("alpha", |v| flip(&mut dummy(v)["spend"]["alpha"])),
            ("rk", |v| flip(&mut dummy(v)["spend"]["rk"])),
            ("nullifier", |v| flip(&mut dummy(v)["spend"]["nullifier"])),
            ("signature", |v| {
                flip(&mut dummy(v)["spend"]["spend_auth_sig"])
            }),
            ("no signature", |v| {
                dummy(v)["spend"]["spend_auth_sig"] = Value::Null
            }),
        ],
        Malformed,
    );
}

#[test]
fn test_output_fields() {
    assert_edits(
        &[
            ("recipient", |v| {
                flip(&mut payment_action(v)["output"]["recipient"])
            }),
            ("value", |v| {
                payment_action(v)["output"]["value"] = json!(600_001)
            }),
            ("rseed", |v| flip(&mut payment_action(v)["output"]["rseed"])),
            ("cmx", |v| flip(&mut payment_action(v)["output"]["cmx"])),
            ("short enc_ciphertext", |v| {
                payment_action(v)["output"]["enc_ciphertext"] =
                    json!({ "Encrypted": vec![0u8; 579] })
            }),
            ("long enc_ciphertext", |v| {
                payment_action(v)["output"]["enc_ciphertext"] =
                    json!({ "Encrypted": vec![0u8; 581] })
            }),
            ("short out_ciphertext", |v| {
                payment_action(v)["output"]["out_ciphertext"] = json!(vec![0u8; 79])
            }),
        ],
        Malformed,
    );
}

#[test]
fn test_values_past_max_money() {
    assert_edits(
        &[
            ("spend value", |v| {
                real(v)["spend"]["value"] = json!(MAX_MONEY + 1)
            }),
            ("spend value u64::MAX", |v| {
                real(v)["spend"]["value"] = json!(u64::MAX)
            }),
            ("output value", |v| {
                payment_action(v)["output"]["value"] = json!(MAX_MONEY + 1)
            }),
            ("MAX_MONEY everywhere", max_money_everywhere),
        ],
        Malformed,
    );
}

#[test]
fn test_duplicated_action() {
    assert_edits(&[("duplicated action", duplicated_action)], Malformed);
}

/// Fields that only other roles fill in, or that the device cannot check.
#[test]
fn test_fields_of_other_roles() {
    assert_edits(
        &[
            ("signature on the real spend", real_spend_signed),
            ("witness", |v| {
                real(v)["spend"]["witness"] = json!([0, vec![vec![0u8; 32]; 32]])
            }),
            ("dummy_sk", |v| {
                dummy(v)["spend"]["dummy_sk"] = json!(vec![0u8; 32])
            }),
            ("spend proprietary", |v| {
                real(v)["spend"]["proprietary"] = json!({ "x": [1] })
            }),
            ("output proprietary", |v| {
                payment_action(v)["output"]["proprietary"] = json!({ "change": [1] })
            }),
            ("memo plaintext", |v| {
                payment_action(v)["output"]["enc_ciphertext"] = json!({ "MemoPlaintext": [1, 2] })
            }),
        ],
        Policy,
    );
}

const MISSING: &[(&str, &str)] = &[
    ("spend", "nullifier"),
    ("spend", "rk"),
    ("spend", "recipient"),
    ("spend", "value"),
    ("spend", "rho"),
    ("spend", "rseed"),
    ("spend", "fvk"),
    ("spend", "alpha"),
    ("output", "cmx"),
    ("output", "recipient"),
    ("output", "value"),
    ("output", "rseed"),
];

fn real_spend_signed(v: &mut Value) {
    let signature = dummy(v)["spend"]["spend_auth_sig"].clone();
    real(v)["spend"]["spend_auth_sig"] = signature;
}

fn max_money_everywhere(v: &mut Value) {
    for index in 0..2 {
        let action = &mut v["ironwood"]["actions"][index];
        action["spend"]["value"] = json!(MAX_MONEY);
        action["output"]["value"] = json!(MAX_MONEY);
    }
    v["ironwood"]["value_sum"][0] = json!(MAX_MONEY);
}

fn duplicated_action(v: &mut Value) {
    let action = real(v).clone();
    v["ironwood"]["actions"] = json!([action.clone(), action]);
}

#[test]
fn test_missing_fields() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    for (part, field) in MISSING {
        let mutated = mutate(&bytes, |v| real(v)[part][field] = Value::Null);
        assert_outcome(
            &wallet,
            &format!("{part}.{field}"),
            &mutated,
            Err(Malformed),
        );
    }
    for field in ["cv_net", "rcv"] {
        let mutated = mutate(&bytes, |v| real(v)[field] = Value::Null);
        assert_outcome(&wallet, field, &mutated, Err(Malformed));
    }
}

/// A host may claim the derivation of a spend or output. Only the device's
/// own seed and account path are accepted.
#[test]
fn test_zip32_derivation() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let own = wallet.account_path();
    let fingerprint = wallet.seed_fingerprint;
    let with = |index: usize, value: u32| {
        let mut path = own;
        path[index] = value;
        path
    };
    let cases: [(&str, [u8; 32], &[u32], Outcome); 9] = [
        ("own", fingerprint, &own, Ok(())),
        ("other seed", [0x5f; 32], &own, Err(Policy)),
        (
            "other account",
            fingerprint,
            &with(2, own[2] + 1),
            Err(Policy),
        ),
        (
            "other coin type",
            fingerprint,
            &with(1, own[1] + 1),
            Err(Policy),
        ),
        (
            "other purpose",
            fingerprint,
            &with(0, own[0] + 1),
            Err(Policy),
        ),
        ("short path", fingerprint, &own[..2], Err(Policy)),
        ("empty path", fingerprint, &[], Err(Policy)),
        (
            "long path",
            fingerprint,
            &[own[0], own[1], own[2], own[2]],
            Err(Policy),
        ),
        (
            "non-hardened account",
            fingerprint,
            &with(2, wallet.account),
            Err(Malformed),
        ),
    ];
    let parts: [(&str, Select, &str); 4] = [
        ("real spend", real, "spend"),
        ("dummy spend", dummy, "spend"),
        ("payment", payment_action, "output"),
        (
            "change",
            |v| action(v, |a| a["output"]["value"] == 390_000),
            "output",
        ),
    ];
    for (name, fingerprint, path, expected) in cases {
        for (part, select, field) in parts {
            let mutated = mutate(&bytes, |v| {
                select(v)[field]["zip32_derivation"] = derivation_json(&fingerprint, path);
            });
            assert_outcome(&wallet, &format!("{name} on {part}"), &mutated, expected);
        }
    }
}

/// The dummy spend's signature covers the sighash, so it is checked once the
/// trailer is read, after the payment was confirmed.
#[test]
fn test_dummy_signature_checked_at_trailer() {
    let wallet = Wallet::testnet();
    let tx = Tx::pay(vec![Output::payment(990_000)], Vec::new(), 10_000);
    let bytes = mutate(&build(&wallet, &tx).bytes, |v| {
        flip(&mut dummy(v)["spend"]["spend_auth_sig"]);
    });
    let mut session = wallet.session();
    wallet.begin(&mut session, bytes.len()).unwrap();
    let mut confirmed = 0;
    for (offset, byte) in bytes.iter().enumerate() {
        match session.feed(&[*byte], &wallet.fvk, &mut || {}) {
            Ok((1, Event::ConfirmOutput { .. })) => confirmed += 1,
            Ok((1, Event::NeedMore)) => {}
            Ok(other) => panic!("{other:?}"),
            Err(error) => {
                assert_eq!((offset, error), (bytes.len() - 1, Malformed));
                break;
            }
        }
    }
    assert_eq!(confirmed, 1);
}

/// `ask` as a scalar: a signing key encodes as its scalar, and randomizing by
/// zero leaves it unchanged.
fn scalar(ask: &SpendAuthorizingKey) -> pallas::Scalar {
    pallas::Scalar::from_repr(<[u8; 32]>::from(ask.randomize(&pallas::Scalar::ZERO))).unwrap()
}

/// The dummy spend of `built` rerandomized by `alpha`, with `rk` and the
/// signature of its randomized key over `message`.
fn rerandomized(
    built: &Built,
    alpha: pallas::Scalar,
    message: impl Fn(&[u8]) -> [u8; 32],
) -> Vec<u8> {
    let (index, sk) = built.dummies[0];
    let rsk = SpendAuthorizingKey::from(&sk).randomize(&alpha);
    let unsigned = mutate(&built.bytes, |v| {
        let spend = &mut v["ironwood"]["actions"][index]["spend"];
        spend["alpha"] = json!(alpha.to_repr());
        spend["rk"] = json!(<[u8; 32]>::from(VerificationKey::from(&rsk)));
    });
    let signature = rsk.sign(ChaCha20Rng::from_seed([1; 32]), &message(&unsigned));
    mutate(&unsigned, |v| {
        v["ironwood"]["actions"][index]["spend"]["spend_auth_sig"] =
            json!(<[u8; 64]>::from(&signature).to_vec());
    })
}

/// A host that rerandomizes a dummy spend to the identity key signs every
/// message; the Session refuses the identity at the action.
#[test]
fn test_identity_rk() {
    let wallet = Wallet::testnet();
    let built = build(&wallet, &Tx::simple());
    let ask = SpendAuthorizingKey::from(&built.dummies[0].1);
    // `rsk = ask - ask = 0`, whose signature verifies under any message.
    let identity = rerandomized(&built, -scalar(&ask), |_| [0; 32]);
    let rk = &json(&identity)["ironwood"]["actions"][built.dummies[0].0]["spend"]["rk"];
    assert_eq!(byte_array::<32>(rk), [0; 32]);
    assert_outcome(&wallet, "identity", &identity, Err(Malformed));
    // Refused at the dummy action, not at the trailer.
    let mut session = wallet.session();
    wallet.begin(&mut session, identity.len()).unwrap();
    let failed_at = identity
        .iter()
        .position(|byte| session.feed(&[*byte], &wallet.fvk, &mut || {}).is_err())
        .unwrap();
    assert!(failed_at < identity.len() - 1);

    // The same rerandomization to the basepoint is an ordinary key.
    let basepoint = rerandomized(&built, pallas::Scalar::ONE - scalar(&ask), sighash);
    let signatures = sign(&wallet, &basepoint).unwrap();
    assert_signatures_apply(&basepoint, &signatures);
}
