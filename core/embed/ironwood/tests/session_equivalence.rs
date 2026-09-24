#![cfg(feature = "test")]
//! Differential tests of the streaming `Session` against `Engine`: over the
//! conformance corpus and its mutations, under every chunking, the session
//! must yield the same projection, output confirmations, sighash, error class
//! and (with an identically seeded RNG) the same signature bytes, and must
//! never hold a token the engine would not have issued.

mod common;
// The scanner is compiled in only to locate section boundaries for the
// truncation tests; it needs these names at the crate root.
#[allow(dead_code)]
#[path = "../src/stream.rs"]
mod stream;

use std::collections::BTreeSet;

use common::*;
use ironwood::{
    Engine, ErrorCode, Event, Network, OutputKind, Policy, Review, ReviewedOutput, Session,
    SignatureRecord,
};
pub use ironwood::{
    Error, MAX_ACTIONS, MAX_PCZT_BYTES, MAX_SCRIPT_PUBKEY_BYTES, MAX_TRANSPARENT_OUTPUTS,
    P2PKH_SCRIPT_BYTES, P2SH_SCRIPT_BYTES, Result, TransparentKind, USER_ADDRESS_BUDGET,
    ZIP32_HARDENED,
};
use ironwood_pasta_curves::group::ff::{Field, PrimeField};
use ironwood_pasta_curves::pallas;
use orchard::keys::{FullViewingKey, SpendAuthorizingKey, SpendingKey};
use orchard::primitives::redpallas::{Signature, SpendAuth, VerificationKey};
use pczt::Pczt;
use pczt::roles::signer::{Signer, SpendAuthSignature};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde_json::{Value, json};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::MAX_MONEY;

const WHOLE: usize = usize::MAX;
const CHUNKINGS: [usize; 5] = [1, 7, 64, 1024, WHOLE];

struct Case {
    name: String,
    bytes: Vec<u8>,
    policy: Policy,
}

fn case(name: impl Into<String>, bytes: Vec<u8>) -> Case {
    Case {
        name: name.into(),
        bytes,
        policy: policy(Network::Testnet, 100_000),
    }
}

fn with_action(v: &mut Value, index: usize) -> &mut Value {
    &mut v["ironwood"]["actions"][index]
}

fn mutate_bytes(bytes: &[u8], f: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = json(bytes);
    f(&mut value);
    encode(value)
}

fn padding_index(value: &Value) -> usize {
    value["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|action| action["output"]["value"] == 0)
        .unwrap()
}

/// Every PCZT the conformance and scanner suites exercise, plus the mutations
/// `tests/conformance.rs` applies through the engine. The engine decides
/// per case whether it is accepted or which class rejects it.
fn corpus() -> Vec<Case> {
    let mut corpus = vec![
        case("fixture", fixture()),
        case("testnet", network_fixture(Network::Testnet)),
        Case {
            name: "mainnet".into(),
            bytes: network_fixture(Network::Mainnet),
            policy: policy(Network::Mainnet, 100_000),
        },
        Case {
            name: "mainnet under testnet policy".into(),
            bytes: network_fixture(Network::Mainnet),
            policy: policy(Network::Testnet, 100_000),
        },
        Case {
            name: "testnet under mainnet policy".into(),
            bytes: network_fixture(Network::Testnet),
            policy: policy(Network::Mainnet, 100_000),
        },
        Case {
            name: "mainnet wrong branch".into(),
            bytes: mutate_bytes(&network_fixture(Network::Mainnet), |v| {
                v["global"]["consensus_branch_id"] = json!(0)
            }),
            policy: policy(Network::Mainnet, 100_000),
        },
        Case {
            name: "fee above cap".into(),
            bytes: fixture(),
            policy: policy(Network::Testnet, 9_999),
        },
        case("wrong change ovk", build_with_wrong_change_ovk()),
        case("discarded payment ovk", build_with_discarded_payment_ovk()),
        case("change without ovk", build_with_change_without_ovk()),
        case("stock sdk view", build_stock_sdk_view()),
        case("own zip32 derivation on spend", with_own_derivation(false)),
        case(
            "own zip32 derivation on spend and change",
            with_own_derivation(true),
        ),
        case(
            "other seed zip32 derivation",
            mutate(|v| {
                let i = real(v);
                with_action(v, i)["spend"]["zip32_derivation"] =
                    derivation_json(&[0x5f; 32], &own_path(Network::Testnet));
            }),
        ),
        case(
            "other account zip32 derivation",
            mutate(|v| {
                let i = real(v);
                let mut path = own_path(Network::Testnet);
                path[2] += 1;
                with_action(v, i)["spend"]["zip32_derivation"] =
                    derivation_json(&SEED_FINGERPRINT, &path);
            }),
        ),
        case(
            "non-hardened zip32 derivation",
            mutate(|v| {
                let i = real(v);
                let mut path = own_path(Network::Testnet);
                path[2] = ACCOUNT;
                with_action(v, i)["spend"]["zip32_derivation"] =
                    derivation_json(&SEED_FINGERPRINT, &path);
            }),
        ),
        Case {
            name: "own zip32 derivation under mainnet policy".into(),
            bytes: mutate_bytes(&network_fixture(Network::Mainnet), |v| {
                let i = real(v);
                with_action(v, i)["spend"]["zip32_derivation"] =
                    derivation_json(&SEED_FINGERPRINT, &own_path(Network::Mainnet));
            }),
            policy: policy(Network::Mainnet, 100_000),
        },
        Case {
            name: "testnet zip32 derivation under mainnet policy".into(),
            bytes: mutate_bytes(&network_fixture(Network::Mainnet), |v| {
                let i = real(v);
                with_action(v, i)["spend"]["zip32_derivation"] =
                    derivation_json(&SEED_FINGERPRINT, &own_path(Network::Testnet));
            }),
            policy: policy(Network::Mainnet, 100_000),
        },
        case(
            "sapling full view",
            mutate(|v| {
                v["sapling"] = json!({
                    "spends": [], "outputs": [], "value_sum": 0,
                    "anchor": vec![0u8; 32], "bsk": vec![9u8; 32]
                })
            }),
        ),
        case(
            "sapling value sum",
            mutate(|v| {
                v["sapling"] = json!({
                    "spends": [], "outputs": [], "value_sum": 1, "anchor": null, "bsk": null
                })
            }),
        ),
        case(
            "user_address over budget",
            mutate(|v| {
                let i = payment(v);
                with_action(v, i)["output"]["user_address"] =
                    json!("u".repeat(USER_ADDRESS_BUDGET + 1))
            }),
        ),
        case("dummy spend padding", build_with_dummy_spend_padding()),
        case("zero change", build(990_000, 0, MemoBytes::empty(), false)),
        case(
            "self payment",
            build(600_000, 390_000, MemoBytes::empty(), true),
        ),
        case(
            "nonempty memo",
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes(b"hello").unwrap(),
                false,
            ),
        ),
        case(
            "memo at budget",
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes(&[b'a'; ironwood::MEMO_TEXT_BUDGET]).unwrap(),
                false,
            ),
        ),
        case(
            "memo over budget",
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes(&[b'a'; ironwood::MEMO_TEXT_BUDGET + 1]).unwrap(),
                false,
            ),
        ),
        case(
            "utf-8 memo",
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes("Zodl ✓ café ☕".as_bytes()).unwrap(),
                false,
            ),
        ),
        case(
            "unrenderable memo",
            build(
                600_000,
                390_000,
                // Supplementary plane and a bidi override: hashed, not shown.
                MemoBytes::from_bytes("pay \u{1F600} \u{202E}bob".as_bytes()).unwrap(),
                false,
            ),
        ),
        case(
            "arbitrary memo",
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes(&{
                    let mut m = [0x41u8; 512];
                    m[0] = 0xff;
                    m
                })
                .unwrap(),
                false,
            ),
        ),
        case(
            "change memo",
            build_with_change_memo(MemoBytes::from_bytes(b"hidden").unwrap()),
        ),
        case(
            "zero-value nonempty memo",
            build(
                0,
                990_000,
                MemoBytes::from_bytes(b"hidden text").unwrap(),
                false,
            ),
        ),
        case("two inputs", build_inputs(&[100_000; 2])),
        case("eight inputs", build_inputs(&[100_000; 8])),
        case(
            "overflowing inputs",
            build_inputs(&[MAX_MONEY / 2, MAX_MONEY / 2 + 1]),
        ),
        case(
            "padding tampered without dummy spend",
            mutate_bytes(&build_inputs(&[100_000, 100_000]), |v| {
                let i = padding_index(v);
                flip(&mut with_action(v, i)["output"]["out_ciphertext"]);
            }),
        ),
        case(
            "padding tampered with dummy spend",
            mutate_bytes(&build_with_dummy_spend_padding(), |v| {
                let i = padding_index(v);
                flip(&mut with_action(v, i)["output"]["out_ciphertext"]);
            }),
        ),
        case(
            "explicit zero lock time",
            mutate(|v| v["global"]["fallback_lock_time"] = json!(0)),
        ),
        case(
            "zero anchor",
            mutate(|v| v["ironwood"]["anchor"] = json!(vec![0; 32])),
        ),
        case(
            "absent anchor",
            mutate(|v| v["ironwood"]["anchor"] = Value::Null),
        ),
        case(
            "invalid anchor",
            mutate(|v| v["ironwood"]["anchor"] = json!(vec![0xff; 32])),
        ),
        case(
            "ock",
            mutate(|v| {
                let i = payment(v);
                with_action(v, i)["output"]["ock"] = json!(vec![7; 32]);
            }),
        ),
        case("valid ock", with_valid_ock(false)),
        case("flipped valid ock", with_valid_ock(true)),
        case(
            "absent dummy signature",
            mutate(|v| {
                let i = 1 - real(v);
                with_action(v, i)["spend"]["spend_auth_sig"] = Value::Null;
            }),
        ),
        case(
            "flipped dummy signature",
            mutate(|v| {
                let i = 1 - real(v);
                flip(&mut with_action(v, i)["spend"]["spend_auth_sig"]);
            }),
        ),
        case(
            "signature on real spend",
            mutate(|v| {
                let i = real(v);
                let signature = with_action(v, 1 - i)["spend"]["spend_auth_sig"].clone();
                with_action(v, i)["spend"]["spend_auth_sig"] = signature;
            }),
        ),
        // Constructed, not bit-flipped.
        case("dummy spend rk identity", identity_rk_dummy()),
        case(
            "dummy spend rerandomized to the basepoint",
            basepoint_rk_dummy(),
        ),
        case("flags zero", mutate(|v| v["ironwood"]["flags"] = json!(0))),
        case(
            "flags reserved bit",
            mutate(|v| v["ironwood"]["flags"] = json!(0x87)),
        ),
        case(
            "duplicated action",
            mutate(|v| {
                let action = with_action(v, real(v)).clone();
                v["ironwood"]["actions"] = json!([action.clone(), action]);
            }),
        ),
        case(
            "five-byte header varints",
            mutate(|v| {
                for field in [
                    "tx_version",
                    "version_group_id",
                    "consensus_branch_id",
                    "fallback_lock_time",
                    "expiry_height",
                    "coin_type",
                ] {
                    v["global"][field] = json!(u32::MAX);
                }
            }),
        ),
        case(
            "max money everywhere",
            mutate(|v| {
                for i in 0..2 {
                    with_action(v, i)["spend"]["value"] = json!(MAX_MONEY);
                    with_action(v, i)["output"]["value"] = json!(MAX_MONEY);
                }
                v["ironwood"]["value_sum"][0] = json!(MAX_MONEY);
            }),
        ),
        case(
            "declared balance off by one",
            mutate(|v| v["ironwood"]["value_sum"][0] = json!(10_001)),
        ),
        case(
            "nonempty memo in real output",
            mutate(|v| {
                let i = payment(v);
                flip(&mut with_action(v, i)["output"]["enc_ciphertext"]["Encrypted"]);
            }),
        ),
        // Scanner-level rejections (tests/stream_equivalence.rs).
        case("empty", Vec::new()),
        case("short", b"PCZT".to_vec()),
        case("largest zeros", vec![0; MAX_PCZT_BYTES]),
        case("v1 magic", {
            let mut bytes = fixture();
            bytes[4] = 1;
            bytes
        }),
        case("overlong version varint", {
            let mut bytes = fixture();
            bytes.splice(8..9, [0x86, 0]);
            bytes
        }),
        case("trailing byte", {
            let mut bytes = fixture();
            bytes.push(0);
            bytes
        }),
        case(
            "tx modifiable",
            mutate(|v| v["global"]["tx_modifiable"] = json!(128)),
        ),
        case(
            "global proprietary",
            mutate(|v| v["global"]["proprietary"] = json!({"x": [1]})),
        ),
        case(
            "transparent present",
            mutate(|v| v["transparent"] = json!({"inputs": [], "outputs": []})),
        ),
        case(
            "sapling present",
            mutate(|v| {
                v["sapling"] = json!({
                    "spends": [], "outputs": [], "value_sum": 0, "anchor": null, "bsk": null
                })
            }),
        ),
        case(
            "orchard present",
            mutate(|v| v["orchard"] = v["ironwood"].clone()),
        ),
        case("no ironwood", mutate(|v| v["ironwood"] = Value::Null)),
        case(
            "zero actions",
            mutate(|v| v["ironwood"]["actions"] = json!([])),
        ),
        case(
            "nine actions",
            mutate(|v| {
                let action = with_action(v, 0).clone();
                v["ironwood"]["actions"] = vec![action; MAX_ACTIONS + 1].into();
            }),
        ),
        case(
            "spend value u64::MAX",
            mutate(|v| with_action(v, 0)["spend"]["value"] = json!(u64::MAX)),
        ),
        case(
            "spend value above max money",
            mutate(|v| with_action(v, 0)["spend"]["value"] = json!(MAX_MONEY + 1)),
        ),
        case(
            "output value above max money",
            mutate(|v| with_action(v, 1)["output"]["value"] = json!(MAX_MONEY + 1)),
        ),
        case(
            "witness",
            mutate(|v| with_action(v, 0)["spend"]["witness"] = json!([0, vec![vec![0u8; 32]; 32]])),
        ),
        case(
            "spend zip32 derivation",
            mutate(|v| {
                with_action(v, 0)["spend"]["zip32_derivation"] =
                    json!({"seed_fingerprint": vec![0u8; 32], "derivation_path": [1u32]})
            }),
        ),
        case(
            "dummy sk",
            mutate(|v| with_action(v, 0)["spend"]["dummy_sk"] = json!(vec![0; 32])),
        ),
        case(
            "spend proprietary",
            mutate(|v| with_action(v, 0)["spend"]["proprietary"] = json!({"x": [1]})),
        ),
        case(
            "memo plaintext ciphertext",
            mutate(|v| {
                with_action(v, 0)["output"]["enc_ciphertext"] = json!({"MemoPlaintext": [1, 2]})
            }),
        ),
        case(
            "short enc ciphertext",
            mutate(|v| {
                with_action(v, 0)["output"]["enc_ciphertext"] = json!({"Encrypted": vec![0u8; 579]})
            }),
        ),
        case(
            "long enc ciphertext",
            mutate(|v| {
                with_action(v, 0)["output"]["enc_ciphertext"] = json!({"Encrypted": vec![0u8; 581]})
            }),
        ),
        case(
            "short out ciphertext",
            mutate(|v| with_action(v, 0)["output"]["out_ciphertext"] = json!(vec![0u8; 79])),
        ),
        case(
            "output zip32 derivation",
            mutate(|v| {
                with_action(v, 0)["output"]["zip32_derivation"] =
                    json!({"seed_fingerprint": vec![0u8; 32], "derivation_path": [1u32]})
            }),
        ),
        case(
            "user address",
            mutate(|v| with_action(v, 0)["output"]["user_address"] = json!("attacker")),
        ),
        case(
            "output proprietary",
            mutate(|v| with_action(v, 0)["output"]["proprietary"] = json!({"change": [1]})),
        ),
        case(
            "negative value sum",
            mutate(|v| v["ironwood"]["value_sum"][1] = json!(true)),
        ),
        case(
            "value sum above max money",
            mutate(|v| v["ironwood"]["value_sum"][0] = json!(MAX_MONEY + 1)),
        ),
        case(
            "note version v2",
            mutate(|v| v["ironwood"]["note_version"] = json!("V2")),
        ),
        case(
            "zkproof present",
            mutate(|v| v["ironwood"]["zkproof"] = json!(vec![0u8; 4])),
        ),
        case(
            "bsk present",
            mutate(|v| v["ironwood"]["bsk"] = json!(vec![0u8; 32])),
        ),
    ];
    // Deshields: the shielded half is balanced so that the Ironwood value sum
    // is `fee + Σ transparent outputs`, which is the identity the accounting
    // now has to get right.
    for count in [0usize, 1, 2, 4] {
        corpus.push(case(
            format!("{count} transparent outputs"),
            build_transparent(&transparent_values(count)),
        ));
    }
    // The same shielded side with no transparent bundle: the whole released
    // amount becomes fee, which the cap then refuses. This is the case that
    // fails if the subtraction is dropped on either side.
    corpus.push(case(
        "transparent release without a transparent bundle",
        build_deshield(&[], 100_000),
    ));
    // Unbalanced: a transparent output the value sum does not release, so the
    // fee subtraction underflows.
    corpus.push(case(
        "transparent output the value sum does not release",
        build_deshield(&[(100_000, p2pkh([0x33; 20]))], 0),
    ));
    corpus.push(case(
        "transparent output at max money",
        build_deshield(&[(MAX_MONEY, p2pkh([0x33; 20]))], 0),
    ));
    // A payment of nothing is not a payment: both twins refuse it as policy
    // rather than show "0 ZEC" to a public address and count it as reviewed.
    corpus.push(case(
        "transparent output of zero",
        build_deshield(&[(0, p2pkh([0x33; 20]))], 0),
    ));
    // Both admitted script shapes on their own, so the P2SH branch of the
    // projection is exercised without the alternation.
    for (name, script) in [
        ("p2pkh only", p2pkh([0x51; 20])),
        ("p2sh only", p2sh([0x51; 20])),
    ] {
        let value = transparent_values(1)[0];
        corpus.push(case(
            format!("transparent {name}"),
            build_deshield(&[(value, script)], value),
        ));
    }
    corpus.push(case(
        "transparent input",
        with_transparent_bundle_and_input(&build_transparent(&transparent_values(1))),
    ));
    corpus.push(case(
        "transparent outputs at the logical-action cap",
        build_deshield(
            &(0..MAX_ACTIONS - 2)
                .map(|i| (1, transparent_script_for(i)))
                .collect::<Vec<_>>(),
            (MAX_ACTIONS - 2) as u64,
        ),
    ));
    corpus.push(case(
        "transparent outputs past the logical-action cap",
        with_transparent_bundle(
            &build_transparent(&transparent_values(1)),
            (0..MAX_ACTIONS - 1)
                .map(|i| transparent_output_json(1, &transparent_script_for(i)))
                .collect(),
        ),
    ));
    for inputs in 1..=CORPUS_MAX_ACTIONS {
        let values: Vec<u64> = (0..inputs as u64).map(|i| 100_000 + i * 10_000).collect();
        corpus.push(case(format!("{inputs} inputs"), build_inputs(&values)));
    }
    for outputs in [1, 8] {
        corpus.push(case(
            format!("{outputs} actions reanchored"),
            mutate_bytes(&build_actions(outputs), |v| {
                v["ironwood"]["anchor"] = json!(vec![0; 32])
            }),
        ));
    }
    // tests/conformance.rs tampering cases.
    for field in ["recipient", "value", "rseed", "cmx"] {
        corpus.push(case(
            format!("payment output {field} tampered"),
            mutate(|v| {
                let i = payment(v);
                let f = &mut with_action(v, i)["output"][field];
                if field == "value" {
                    *f = 600_001.into();
                } else {
                    flip(f);
                }
            }),
        ));
    }
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
        corpus.push(case(
            format!("real spend {field} tampered"),
            mutate(|v| {
                let i = real(v);
                let f = &mut with_action(v, i)["spend"][field];
                if field == "value" {
                    *f = 1_000_001.into();
                } else {
                    flip(f);
                }
            }),
        ));
        corpus.push(case(
            format!("dummy spend {field} tampered"),
            mutate(|v| {
                let i = 1 - real(v);
                let f = &mut with_action(v, i)["spend"][field];
                if field == "value" {
                    *f = 1.into();
                } else {
                    flip(f);
                }
            }),
        ));
    }
    for field in ["ephemeral_key", "enc_ciphertext", "out_ciphertext", "ock"] {
        corpus.push(case(
            format!("payment output {field} tampered"),
            mutate(|v| {
                let i = payment(v);
                let f = &mut with_action(v, i)["output"][field];
                if field == "enc_ciphertext" {
                    flip(&mut f["Encrypted"]);
                } else if field == "ock" {
                    *f = json!(vec![0; 32]);
                } else {
                    flip(f);
                }
            }),
        ));
    }
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
        corpus.push(case(
            format!("global {field}={value}"),
            mutate(|v| v["global"][field] = value.into()),
        ));
    }
    for (part, fields) in [
        (
            "spend",
            vec![
                "nullifier",
                "rk",
                "recipient",
                "value",
                "rho",
                "rseed",
                "fvk",
                "alpha",
            ],
        ),
        ("output", vec!["cmx", "recipient", "value", "rseed"]),
    ] {
        for field in fields {
            corpus.push(case(
                format!("missing {part}.{field}"),
                mutate(|v| with_action(v, 0)[part][field] = Value::Null),
            ));
        }
    }
    for field in ["cv_net", "rcv"] {
        corpus.push(case(
            format!("missing {field}"),
            mutate(|v| with_action(v, 0)[field] = Value::Null),
        ));
    }
    corpus
}

/// The fixture with the payment output's genuine OCK, optionally corrupted.
fn with_valid_ock(flipped: bool) -> Vec<u8> {
    use orchard::keys::Scope;
    use orchard::note_encryption::IronwoodDomain;
    use pczt::roles::verifier::{OrchardError, Verifier};
    use zcash_note_encryption::{Domain, EphemeralKeyBytes};
    let mut value = json(&fixture());
    let i = payment(&value);
    let mut ock = None;
    Verifier::new(Pczt::parse(&fixture()).unwrap())
        .with_ironwood(|bundle| -> std::result::Result<(), OrchardError<()>> {
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
    with_action(&mut value, i)["output"]["ock"] = json!(ock.unwrap());
    if flipped {
        flip(&mut with_action(&mut value, i)["output"]["ock"]);
    }
    encode(value)
}

/// The padding dummy spend of `build_with_dummy_spend_padding` together with
/// the spend-authorizing key the host holds for it, recovered from `dummy_sk`
/// before the IO Finalizer signs the dummy and clears that field.
struct DummySpend {
    /// The finished fixture, byte-identical to the shared builder's.
    bytes: Vec<u8>,
    index: usize,
    ask: SpendAuthorizingKey,
    fvk: FullViewingKey,
}

/// `common::build_with_dummy_spend_padding` up to the Creator. The shared
/// builder finalizes in one step, so the pre-finalization PCZT (the only place
/// `dummy_sk` exists) is rebuilt here with the same seed and inputs and the
/// finished bytes are asserted equal to the shared fixture.
fn dummy_spend() -> &'static DummySpend {
    use orchard::bundle::BundleVersion;
    use orchard::keys::Scope;
    use orchard::note_encryption::IronwoodDomain;
    use orchard::value::NoteValue;
    use pczt::roles::creator::Creator;
    use pczt::roles::io_finalizer::IoFinalizer;
    use pczt::roles::redactor::Redactor;
    use zcash_note_encryption::try_note_decryption;
    use zcash_primitives::transaction::builder::{BundlePadding, DeferredPcztBuilder};
    use zcash_primitives::transaction::fees::zip317;
    use zcash_protocol::local_consensus::LocalNetwork;
    use zcash_protocol::value::Zatoshis;
    static DUMMY: std::sync::OnceLock<DummySpend> = std::sync::OnceLock::new();
    DUMMY.get_or_init(|| {
        let network = LocalNetwork {
            overwinter: Some(1.into()),
            sapling: Some(2.into()),
            blossom: Some(3.into()),
            heartwood: Some(4.into()),
            canopy: Some(5.into()),
            nu5: Some(6.into()),
            nu6: Some(7.into()),
            nu6_1: Some(8.into()),
            nu6_2: Some(9.into()),
            nu6_3: Some(10.into()),
        };
        let mut rng = ChaCha20Rng::from_seed([46; 32]);
        let (fvk, _) = keys();
        let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
        let version = BundleVersion::ironwood_v3();
        let mut funding = orchard::builder::Builder::new(
            orchard::builder::BundleType::DEFAULT,
            version,
            version.default_flags(),
            orchard::Anchor::empty_tree(),
        )
        .unwrap();
        funding
            .add_output(
                None,
                fvk.address_at(0u32, Scope::External),
                NoteValue::from_raw(1_000_000),
                MemoBytes::empty().into_bytes(),
            )
            .unwrap();
        let (bundle, meta) = funding.build_for_pczt(&mut rng).unwrap();
        let action = &bundle.actions()[meta.output_action_index(0).unwrap()];
        let (note, _, _) = try_note_decryption(
            &IronwoodDomain::for_pczt_action(action),
            &fvk.to_ivk(Scope::External).prepare(),
            action,
        )
        .unwrap();
        let mut builder = DeferredPcztBuilder::new::<zip317::FeeError>(
            network,
            HEIGHT.into(),
            BundlePadding::DEFAULT,
            BundlePadding::DEFAULT,
        )
        .unwrap();
        builder
            .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
            .unwrap();
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(Scope::External)),
                other.address_at(0u32, Scope::External),
                Zatoshis::from_u64(990_000).unwrap(),
                MemoBytes::empty(),
            )
            .unwrap();
        let result = builder
            .build_for_pczt(&mut rng, &zip317::FeeRule::standard())
            .unwrap();
        let unfinalized = Creator::build_from_parts(result.pczt_parts).unwrap();

        let before = json(&unfinalized.clone().serialize().unwrap());
        let dummy = before["ironwood"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|action| action["spend"]["value"] == 0)
            .expect("padding dummy spend");
        let sk = SpendingKey::from_bytes(bytes_of(&dummy["spend"]["dummy_sk"])).unwrap();
        let ask = SpendAuthorizingKey::from(&sk);
        let fvk = FullViewingKey::from(&sk);

        let bytes = Redactor::new(IoFinalizer::new(unfinalized).finalize_io().unwrap())
            .redact_sapling_with(|mut sapling| {
                sapling.clear_bsk();
                sapling.clear_anchor();
            })
            .redact_ironwood_with(|mut ironwood| {
                ironwood.clear_bsk();
                ironwood.redact_actions(|mut action| action.clear_spend_witness());
            })
            .finish()
            .serialize()
            .unwrap();
        let after = json(&bytes);
        let index = after["ironwood"]["actions"]
            .as_array()
            .unwrap()
            .iter()
            .position(|action| {
                action["spend"]["value"] == 0
                    && bytes_of::<96>(&action["spend"]["fvk"]) == fvk.to_bytes()
            })
            .expect("the same dummy after finalization");
        assert!(with_action(&mut after.clone(), index)["spend"]["dummy_sk"].is_null());
        // The IO Finalizer signs the dummy with `OsRng`, so only that
        // signature may differ from the shared fixture.
        let unsigned = |bytes: &[u8]| {
            let mut value = json(bytes);
            with_action(&mut value, index)["spend"]["spend_auth_sig"] = Value::Null;
            value
        };
        assert_eq!(
            unsigned(&bytes),
            unsigned(&build_with_dummy_spend_padding()),
            "replica of the shared builder"
        );
        DummySpend {
            bytes,
            index,
            ask,
            fvk,
        }
    })
}

fn bytes_of<const N: usize>(value: &Value) -> [u8; N] {
    serde_json::from_value::<Vec<u8>>(value.clone())
        .unwrap()
        .try_into()
        .unwrap()
}

/// `ask` as a scalar: randomizing by zero is the identity map on the key and
/// the fork exposes the randomized key's bytes (redpallas.rs:29).
fn ask_scalar(ask: &SpendAuthorizingKey) -> pallas::Scalar {
    pallas::Scalar::from_repr(<[u8; 32]>::from(ask.randomize(&pallas::Scalar::ZERO))).unwrap()
}

/// The dummy spend re-randomized by `alpha` (so `rk = ak + alpha·G`) with the
/// given signature in place of the IO Finalizer's.
fn rerandomize_dummy(alpha: &pallas::Scalar, rk: [u8; 32], signature: Option<[u8; 64]>) -> Vec<u8> {
    let dummy = dummy_spend();
    mutate_bytes(&dummy.bytes, |v| {
        let spend = &mut with_action(v, dummy.index)["spend"];
        assert_eq!(spend["value"], json!(0));
        spend["alpha"] = json!(alpha.to_repr().to_vec());
        spend["rk"] = json!(rk.to_vec());
        spend["spend_auth_sig"] = signature.map_or(Value::Null, |s| json!(s.to_vec()));
    })
}

/// An identity `rk` must be refused before parsing. The host sets
/// `alpha = -ask_dummy`, so `rk = ak_dummy + alpha·G` is the identity, encoded
/// as `[0; 32]`, and signs with `rsk = ask_dummy + alpha = 0`. Under the
/// identity key RedPallas verification reduces to `s·B == R`, so that
/// signature verifies for every message, including whichever sighash the
/// session would have derived; the host needs no sighash to mount this.
fn identity_rk_dummy() -> Vec<u8> {
    let dummy = dummy_spend();
    let alpha = -ask_scalar(&dummy.ask);
    let rsk = dummy.ask.randomize(&alpha);
    let rk = VerificationKey::from(&rsk);
    assert!(rk.is_identity());
    // pasta_curves-0.5.1/src/curves.rs:693-696: the identity encodes as zeros.
    assert_eq!(<[u8; 32]>::from(&rk), [0; 32]);
    let signature = <[u8; 64]>::from(&rsk.sign(ChaCha20Rng::from_seed(seed(99)), &[0; 32]));
    // curves.rs:663-670 and reddsa-0.5.1/src/verification_key.rs:99-111: the
    // zero encoding decodes to the identity and is admitted as a key, under
    // which the signature verifies for any message.
    let identity = VerificationKey::<SpendAuth>::try_from([0; 32]).unwrap();
    for message in [[0; 32], [0xff; 32], seed(1)] {
        identity
            .verify(&message, &Signature::from(signature))
            .unwrap();
    }
    rerandomize_dummy(&alpha, [0; 32], Some(signature))
}

/// The control for `identity_rk_dummy`: the same host-side re-randomization
/// with `alpha = 1 - ask_dummy`, so `rk` is the SpendAuth basepoint, a valid
/// non-identity key, signed over the true sighash by the `pczt` signer. Both
/// engine and session accept it, so the identity is the only difference.
fn basepoint_rk_dummy() -> Vec<u8> {
    use pczt::roles::signer::Signer;
    let dummy = dummy_spend();
    let alpha = pallas::Scalar::ONE - ask_scalar(&dummy.ask);
    let rk = VerificationKey::from(&dummy.ask.randomize(&alpha));
    assert!(!rk.is_identity());
    let unsigned = rerandomize_dummy(&alpha, <[u8; 32]>::from(&rk), None);
    let mut signer = Signer::new(Pczt::parse(&unsigned).unwrap()).unwrap();
    signer.sign_ironwood(dummy.index, &dummy.ask).unwrap();
    signer.finish().serialize().unwrap()
}

fn seed(n: u64) -> [u8; 32] {
    let mut seed = [0x5e; 32];
    seed[..8].copy_from_slice(&n.to_le_bytes());
    seed
}

#[test]
fn an_over_cap_deshield_is_refused_before_any_confirmation() {
    // 31 transparent outputs and two Ironwood actions: 33 logical actions, one
    // over the shared cap. The action count that decides it is behind the
    // shielded prefix, which the encoding puts after every transparent output,
    // so a session that confirmed each output as it arrived would walk the user
    // through 31 addresses for a transaction it was always going to refuse.
    let bytes = with_transparent_bundle(
        &build_transparent(&transparent_values(1)),
        (0..MAX_ACTIONS - 1)
            .map(|i| transparent_output_json(1, &transparent_script_for(i)))
            .collect(),
    );
    for chunk in CHUNKINGS {
        let mut session = fresh_session();
        session
            .begin(bytes.len(), &keys().0, &SEED_FINGERPRINT)
            .unwrap();
        let mut rest = &bytes[..];
        let error = loop {
            assert!(!rest.is_empty(), "the stream ran out before the verdict");
            let piece = &rest[..rest.len().min(chunk.max(1))];
            match session.feed(piece, &keys().0) {
                Ok((consumed, event)) => {
                    assert!(
                        matches!(event, Event::None),
                        "a screen was offered before the cap was decided, chunking {chunk}"
                    );
                    rest = &rest[consumed..];
                }
                Err(error) => break error,
            }
        };
        assert_eq!(error.code(), ErrorCode::Capacity, "chunking {chunk}");
    }
}

fn engine_for(policy: Policy, seed: [u8; 32]) -> Engine<ChaCha20Rng> {
    Engine::with_rng(policy, ChaCha20Rng::from_seed(seed)).unwrap()
}

fn session_for(policy: Policy, seed: [u8; 32]) -> Session<ChaCha20Rng> {
    Session::with_rng(policy, ChaCha20Rng::from_seed(seed)).unwrap()
}

fn fresh_session() -> Session<ChaCha20Rng> {
    session_for(policy(Network::Testnet, 100_000), seed(0))
}

struct Run {
    /// Every `ConfirmOutput`, in order.
    confirmed: Vec<ReviewedOutput>,
    /// Every `ConfirmTransparentOutput`, in order.
    transparent: Vec<ironwood::TransparentOutput>,
    review: Option<Review>,
}

/// Feeds `bytes` in pieces of `chunk` after `begin(declared)`, re-feeding
/// whatever a call leaves unconsumed, and collects the events.
fn stream_into(
    session: &mut Session<ChaCha20Rng>,
    bytes: &[u8],
    declared: usize,
    chunk: usize,
) -> Result<Run> {
    session.begin(declared, &keys().0, &SEED_FINGERPRINT)?;
    let mut run = Run {
        confirmed: Vec::new(),
        transparent: Vec::new(),
        review: None,
    };
    for piece in bytes.chunks(chunk.max(1)) {
        let mut rest = piece;
        while !rest.is_empty() {
            let (consumed, event) = session.feed(rest, &keys().0)?;
            // A held-back transparent confirmation is the one call that
            // consumes nothing; everything else must make progress or the loop
            // would not terminate.
            assert!(
                consumed > 0 || matches!(event, Event::ConfirmTransparentOutput(_)),
                "no progress on {} bytes",
                rest.len()
            );
            rest = &rest[consumed..];
            match event {
                Event::None => {}
                Event::ConfirmOutput(output) => {
                    assert!(run.review.is_none(), "confirmation after review");
                    assert_eq!(output.kind, OutputKind::Payment);
                    run.confirmed.push(output);
                }
                Event::ConfirmTransparentOutput(output) => {
                    assert!(run.review.is_none(), "confirmation after review");
                    run.transparent.push(output);
                }
                Event::Review(review) => {
                    assert!(run.review.is_none(), "second review");
                    assert!(rest.is_empty(), "bytes after the trailer");
                    run.review = Some(review);
                }
            }
        }
    }
    Ok(run)
}

fn run(session: &mut Session<ChaCha20Rng>, bytes: &[u8], chunk: usize) -> Result<Run> {
    stream_into(session, bytes, bytes.len(), chunk)
}

fn assert_no_consent(session: &mut Session<ChaCha20Rng>, token: Option<&ironwood::Token>) {
    assert!(!session.test_has_pending_request());
    assert!(session.test_request_binding_is_zero());
    if let Some(token) = token {
        assert_eq!(session.sign(token, &keys().1).err(), Some(ErrorCode::State));
    }
}

fn engine_signatures(engine: &mut Engine<ChaCha20Rng>, review: &Review) -> Vec<(u8, [u8; 64])> {
    engine.approve(review.token()).unwrap();
    let signed = engine.sign(review.token(), &keys().1).unwrap();
    signatures(&signed)
        .into_iter()
        .map(|(index, signature)| (u8::try_from(index).unwrap(), signature))
        .collect()
}

fn records(records: &[SignatureRecord]) -> Vec<(u8, [u8; 64])> {
    records
        .iter()
        .map(|record| (record.action_index, record.signature))
        .collect()
}

/// Applies the session's records host-side as design §1 requires.
fn apply_host_side(bytes: &[u8], sighash: &[u8; 32], records: &[SignatureRecord]) {
    let mut signer = Signer::new(Pczt::parse(bytes).unwrap()).unwrap();
    for record in records {
        signer
            .apply_orchard_spend_auth_signature(&SpendAuthSignature::from_parts(
                orchard::ValuePool::Ironwood,
                usize::from(record.action_index),
                record.signature,
            ))
            .unwrap();
    }
    assert_eq!(&signer.shielded_sighash(), sighash);
}

struct Counts {
    accepted: usize,
    rejected: Vec<(String, ErrorCode)>,
}

/// The whole corpus, every chunking, against the engine.
fn check(case: &Case, n: u64, counts: &mut Counts) {
    let name = &case.name;
    let mut engine = engine_for(case.policy, seed(n));
    let expected = engine.begin(&case.bytes, &keys().0, &SEED_FINGERPRINT);
    let mut contexts = BTreeSet::new();
    for chunk in CHUNKINGS {
        let mut session = session_for(case.policy, seed(n));
        let actual = run(&mut session, &case.bytes, chunk);
        match (&expected, actual) {
            (Ok(expected), Ok(run)) => {
                let review = run
                    .review
                    .as_ref()
                    .unwrap_or_else(|| panic!("{name}: no review"));
                assert_eq!(review.projection(), expected.projection(), "{name}");
                assert_eq!(review.sighash(), expected.sighash(), "{name}");
                let payments: Vec<_> = expected
                    .projection()
                    .outputs
                    .iter()
                    .filter(|output| output.kind == OutputKind::Payment)
                    .cloned()
                    .collect();
                assert_eq!(run.confirmed, payments, "{name}: chunk {chunk}");
                // The transparent outputs are confirmed in bundle order, and
                // before any shielded one: the encoding puts them first.
                assert_eq!(
                    run.transparent,
                    expected.projection().transparent_outputs,
                    "{name}: chunk {chunk}"
                );
                assert!(session.test_has_pending_request());
                contexts.insert(*review.token().context());
                assert_ne!(review.token().context(), expected.token().context());
                assert_ne!(review.token(), expected.token());
            }
            (Err(expected), Err(actual)) => {
                assert_eq!(actual, *expected, "{name}: chunk {chunk}");
                assert!(!session.test_is_streaming());
                assert_no_consent(&mut session, None);
            }
            (Ok(_), Err(actual)) => panic!("{name}: session rejected with {actual:?}"),
            (Err(expected), Ok(_)) => panic!("{name}: engine rejected with {expected:?}"),
        }
    }
    match expected {
        Ok(expected) => {
            // The token binds the bytes, not their chunking.
            assert_eq!(contexts.len(), 1, "{name}");
            counts.accepted += 1;
            let mut session = session_for(case.policy, seed(n));
            let review = run(&mut session, &case.bytes, 1024)
                .unwrap()
                .review
                .unwrap();
            let expected_signatures = engine_signatures(&mut engine, &expected);
            // No signing before approval; the attempt consumes the request.
            assert_eq!(
                session.sign(review.token(), &keys().1).err(),
                Some(ErrorCode::State)
            );
            assert_no_consent(&mut session, Some(review.token()));
            // Same RNG, same bytes.
            let mut session = session_for(case.policy, seed(n));
            let review = run(&mut session, &case.bytes, WHOLE)
                .unwrap()
                .review
                .unwrap();
            session.approve(review.token()).unwrap();
            assert_eq!(
                session.approve(review.token()).err(),
                Some(ErrorCode::State),
                "{name}: consent is single-use"
            );
            assert_no_consent(&mut session, Some(review.token()));
            let mut session = session_for(case.policy, seed(n));
            let review = run(&mut session, &case.bytes, 7).unwrap().review.unwrap();
            session.approve(review.token()).unwrap();
            let signatures = session.sign(review.token(), &keys().1).unwrap();
            assert_eq!(records(signatures.records()), expected_signatures, "{name}");
            apply_host_side(&case.bytes, review.sighash(), signatures.records());
            assert_no_consent(&mut session, Some(review.token()));
            // A wrong key releases nothing and consumes consent.
            let mut session = session_for(case.policy, seed(n));
            let review = run(&mut session, &case.bytes, 64).unwrap().review.unwrap();
            session.approve(review.token()).unwrap();
            let wrong = SpendAuthorizingKey::from(&SpendingKey::from_bytes([2; 32]).unwrap());
            assert_eq!(
                session.sign(review.token(), &wrong).err(),
                Some(ErrorCode::Signing)
            );
            assert_no_consent(&mut session, Some(review.token()));
        }
        Err(code) => {
            counts.rejected.push((name.clone(), code));
        }
    }
}

#[test]
fn corpus_matches_engine_under_every_chunking() {
    let corpus = corpus();
    let mut counts = Counts {
        accepted: 0,
        rejected: Vec::new(),
    };
    for (n, case) in corpus.iter().enumerate() {
        check(case, n as u64, &mut counts);
    }
    let by_class = |code| counts.rejected.iter().filter(|(_, c)| *c == code).count();
    eprintln!(
        "corpus: {} cases, {} accepted, {} rejected (malformed {}, policy {}, capacity {}), chunkings {:?}",
        corpus.len(),
        counts.accepted,
        counts.rejected.len(),
        by_class(ErrorCode::Malformed),
        by_class(ErrorCode::Policy),
        by_class(ErrorCode::Capacity),
        CHUNKINGS
    );
    assert_eq!(corpus.len(), counts.accepted + counts.rejected.len());
    assert!(counts.accepted >= 30, "{}", counts.accepted);
    assert!(counts.rejected.len() >= 90, "{}", counts.rejected.len());
    for code in [ErrorCode::Malformed, ErrorCode::Policy, ErrorCode::Capacity] {
        assert!(by_class(code) > 0, "{code:?}");
    }
}

/// A sample of single-byte mutations, whole-chunk only, so the differential
/// covers arbitrary corruption and not only field edits. The base carries an
/// explicit anchor and a padding output whose `out_ciphertext` no key
/// recovers (lib.rs:783-790): bytes the engine accepts changed.
#[test]
fn sampled_single_byte_mutations_match_engine() {
    let bytes = mutate_bytes(&build_inputs(&[100_000, 100_000]), |v| {
        v["ironwood"]["anchor"] = json!(vec![0; 32])
    });
    let (mut accepted, mut rejected) = (0, 0);
    for i in (0..bytes.len()).step_by(13) {
        let mut mutated = bytes.clone();
        mutated[i] ^= 0x01;
        let mut engine = engine_for(policy(Network::Testnet, 100_000), seed(1_000 + i as u64));
        let expected = engine.begin(&mutated, &keys().0, &SEED_FINGERPRINT);
        let mut session = session_for(policy(Network::Testnet, 100_000), seed(1_000 + i as u64));
        match (expected, run(&mut session, &mutated, WHOLE)) {
            (Ok(expected), Ok(run)) => {
                let review = run.review.unwrap();
                assert_eq!(review.projection(), expected.projection(), "byte {i}");
                assert_eq!(review.sighash(), expected.sighash(), "byte {i}");
                accepted += 1;
            }
            (Err(expected), Err(actual)) => {
                assert_eq!(actual, expected, "byte {i}");
                rejected += 1;
            }
            (expected, actual) => panic!(
                "byte {i}: engine {:?}, session {:?}",
                expected.map(|_| ()),
                actual.map(|_| ())
            ),
        }
    }
    eprintln!("single-byte sample: {accepted} accepted, {rejected} rejected");
    assert!(accepted >= 3, "{accepted}");
    assert!(rejected >= 50, "{rejected}");
}

/// Cumulative end offsets of every section: the header, each transparent
/// output, the shielded prefix, each action and the trailer.
fn section_ends(bytes: &[u8]) -> Vec<usize> {
    let mut scanner = stream::Scanner::new(bytes.len()).unwrap();
    let mut ends = Vec::new();
    let mut rest = bytes;
    while !rest.is_empty() {
        let (consumed, item) = scanner.feed(rest).unwrap();
        rest = &rest[consumed..];
        if item.is_some() {
            ends.push(bytes.len() - rest.len());
        }
    }
    assert!(scanner.is_finished());
    ends
}

#[test]
fn truncation_at_every_section_boundary_yields_no_token() {
    for (name, bytes) in [
        ("fixture", fixture()),
        ("8 actions", build_actions(8)),
        (
            "2 transparent outputs",
            build_transparent(&transparent_values(2)),
        ),
    ] {
        let view = json(&bytes);
        let ends = section_ends(&bytes);
        // Header, each transparent output, the shielded prefix, each action,
        // the trailer.
        assert_eq!(
            ends.len(),
            view["ironwood"]["actions"].as_array().unwrap().len()
                + view["transparent"]["outputs"]
                    .as_array()
                    .map_or(0, |outputs| outputs.len())
                + 3
        );
        assert_eq!(*ends.last().unwrap(), bytes.len());
        let mut cuts: Vec<usize> = ends[..ends.len() - 1].to_vec();
        cuts.extend(ends.windows(2).map(|w| (w[0] + w[1]) / 2));
        cuts.push(0);
        cuts.push(bytes.len() - 1);
        for cut in cuts {
            // Declared in full, delivered short: the stream simply waits.
            let mut session = fresh_session();
            let run = stream_into(&mut session, &bytes[..cut], bytes.len(), 1024)
                .unwrap_or_else(|e| panic!("{name}: cut {cut}: {e:?}"));
            assert!(run.review.is_none(), "{name}: cut {cut}");
            assert!(session.test_is_streaming());
            assert_no_consent(&mut session, None);
            let foreign = foreign_review();
            assert_eq!(
                session.sign(foreign.token(), &keys().1).err(),
                Some(ErrorCode::State)
            );
            assert!(!session.test_is_streaming(), "{name}: cut {cut}");
            // Declared short: the grammar cannot end there.
            let mut session = fresh_session();
            let result = stream_into(&mut session, &bytes[..cut], cut, 1024);
            assert!(result.is_err(), "{name}: declared {cut}");
            assert!(!session.test_is_streaming());
            assert_no_consent(&mut session, None);
        }
    }
}

/// A review from an unrelated session, for state checks that need a token.
fn foreign_review() -> Review {
    let mut other = session_for(policy(Network::Testnet, 100_000), seed(77));
    run(&mut other, &fixture(), WHOLE).unwrap().review.unwrap()
}

#[test]
fn trailer_rejection_after_confirmations_leaves_nothing() {
    let bytes = build_actions(8);
    let baseline = run(&mut fresh_session(), &bytes, 64).unwrap();
    assert_eq!(baseline.confirmed.len(), 4);
    for (name, mutation, code) in [
        (
            "balance",
            Box::new(|v: &mut Value| v["ironwood"]["value_sum"][0] = json!(1))
                as Box<dyn Fn(&mut Value)>,
            ErrorCode::Malformed,
        ),
        (
            "flags",
            Box::new(|v: &mut Value| v["ironwood"]["flags"] = json!(0)),
            ErrorCode::Policy,
        ),
        (
            "reserved flag bit",
            Box::new(|v: &mut Value| v["ironwood"]["flags"] = json!(0x80)),
            ErrorCode::Malformed,
        ),
        (
            "anchor",
            Box::new(|v: &mut Value| v["ironwood"]["anchor"] = json!(vec![0xff; 32])),
            ErrorCode::Malformed,
        ),
    ] {
        let mutated = mutate_bytes(&bytes, |v| mutation(v));
        assert_eq!(
            engine_for(policy(Network::Testnet, 100_000), seed(2))
                .begin(&mutated, &keys().0, &SEED_FINGERPRINT)
                .err(),
            Some(code),
            "{name}: oracle"
        );
        for chunk in CHUNKINGS {
            let mut session = fresh_session();
            let mut confirmed = 0;
            session
                .begin(mutated.len(), &keys().0, &SEED_FINGERPRINT)
                .unwrap();
            let mut rest = &mutated[..];
            let verdict = loop {
                match session.feed(&rest[..rest.len().min(chunk)], &keys().0) {
                    Ok((consumed, event)) => {
                        rest = &rest[consumed..];
                        match event {
                            Event::ConfirmOutput(_) => confirmed += 1,
                            Event::ConfirmTransparentOutput(_) => {}
                            Event::Review(_) => panic!("{name}: reviewed"),
                            Event::None => {}
                        }
                    }
                    Err(error) => break error,
                }
            };
            assert_eq!(verdict, code, "{name}: chunk {chunk}");
            assert_eq!(confirmed, 4, "{name}: chunk {chunk}");
            assert!(!session.test_is_streaming());
            assert_no_consent(&mut session, None);
        }
    }
}

#[test]
fn dummy_signature_is_verified_at_the_trailer() {
    let bytes = build_with_dummy_spend_padding();
    let bad = mutate_bytes(&bytes, |v| {
        let i = 1 - real(v);
        assert_eq!(with_action(v, i)["spend"]["value"], json!(0));
        flip(&mut with_action(v, i)["spend"]["spend_auth_sig"]);
    });
    assert_error(
        engine_for(policy(Network::Testnet, 100_000), seed(3))
            .begin(&bad, &keys().0, &SEED_FINGERPRINT)
            .unwrap_err(),
        ErrorCode::Malformed,
    );
    let mut session = fresh_session();
    session
        .begin(bad.len(), &keys().0, &SEED_FINGERPRINT)
        .unwrap();
    let mut confirmed = 0;
    let mut failed_at = None;
    for (offset, byte) in bad.iter().enumerate() {
        match session.feed(&[*byte], &keys().0) {
            Ok((1, Event::ConfirmOutput(_))) => confirmed += 1,
            Ok((1, Event::None)) => {}
            Ok(other) => panic!("{other:?}"),
            Err(error) => {
                assert_eq!(error, ErrorCode::Malformed);
                failed_at = Some(offset);
                break;
            }
        }
    }
    // The payment was confirmed before the last byte exposed the signature.
    assert_eq!(confirmed, 1);
    assert_eq!(failed_at, Some(bad.len() - 1));
    assert_no_consent(&mut session, None);
    assert!(run(&mut session, &bytes, WHOLE).unwrap().review.is_some());
}

/// Beyond the corpus's class equality: the
/// engine rejects the identity `rk` in `effects::sighash` (`extract_effects`
/// fails) and the session rejects it at the dummy action itself, before the
/// trailer, with nothing retained. The check is load-bearing: every other
/// per-action verification the session runs passes on this action, the
/// signature verifies under any message, and the same re-randomization to a
/// non-identity `rk` is accepted by both with the same sighash.
#[test]
fn identity_rk_dummy_spend_is_rejected_by_engine_and_session_alike() {
    use pczt::roles::verifier::{OrchardError, Verifier};
    use zcash_protocol::value::ZatBalance;
    let dummy = dummy_spend();
    let bytes = identity_rk_dummy();
    assert_eq!(
        engine_for(policy(Network::Testnet, 100_000), seed(6))
            .begin(&bytes, &keys().0, &SEED_FINGERPRINT)
            .err(),
        Some(ErrorCode::Malformed)
    );
    Verifier::new(Pczt::parse(&bytes).unwrap())
        .with_ironwood(|bundle| -> std::result::Result<(), OrchardError<()>> {
            let action = &bundle.actions()[dummy.index];
            let spend = action.spend();
            assert!(spend.rk().is_identity());
            // For a dummy the verifier uses the wire FVK whichever key is
            // expected (verify.rs:83), so both the host's and the device's
            // view pass.
            for fvk in [&dummy.fvk, &keys().0] {
                spend.verify_nullifier(Some(fvk)).unwrap();
                spend.verify_rk(Some(fvk)).unwrap();
            }
            action.verify_cv_net().unwrap();
            action.output().verify_note_commitment(spend).unwrap();
            assert!(
                bundle.extract_effects::<ZatBalance>().is_err(),
                "the engine's effects::sighash site"
            );
            Ok(())
        })
        .unwrap();

    for chunk in CHUNKINGS {
        let mut session = fresh_session();
        assert_eq!(
            run(&mut session, &bytes, chunk).err(),
            Some(ErrorCode::Malformed),
            "chunk {chunk}"
        );
        assert!(!session.test_is_streaming());
        assert_no_consent(&mut session, None);
    }
    // Byte by byte, the rejection lands on the last byte of the dummy action.
    let ends = section_ends(&bytes);
    let mut session = fresh_session();
    session
        .begin(bytes.len(), &keys().0, &SEED_FINGERPRINT)
        .unwrap();
    let failed_at = bytes
        .iter()
        .enumerate()
        .find_map(|(offset, byte)| session.feed(&[*byte], &keys().0).err().map(|e| (offset, e)));
    // The header and the shielded prefix precede the actions; this fixture
    // has no transparent bundle.
    assert_eq!(
        failed_at,
        Some((ends[2 + dummy.index] - 1, ErrorCode::Malformed))
    );

    let control = basepoint_rk_dummy();
    let expected = engine_for(policy(Network::Testnet, 100_000), seed(7))
        .begin(&control, &keys().0, &SEED_FINGERPRINT)
        .unwrap();
    let review = run(&mut fresh_session(), &control, 7)
        .unwrap()
        .review
        .unwrap();
    assert_eq!(review.sighash(), expected.sighash());
    assert_ne!(
        review.sighash(),
        run(&mut fresh_session(), &dummy.bytes, 7)
            .unwrap()
            .review
            .unwrap()
            .sighash()
    );
}

#[test]
fn second_begin_mid_stream_and_after_review_clears_everything() {
    let bytes = fixture();
    let mut session = fresh_session();
    let half = bytes.len() / 2;
    stream_into(&mut session, &bytes[..half], bytes.len(), 7).unwrap();
    assert!(session.test_is_streaming());
    let review = run(&mut session, &bytes, 7).unwrap().review.unwrap();
    session.approve(review.token()).unwrap();
    // Replacement drops the approved request; the new one carries a new token.
    let replacement = run(&mut session, &bytes, 7).unwrap().review.unwrap();
    assert_ne!(review.token(), replacement.token());
    assert_eq!(review.sighash(), replacement.sighash());
    assert_eq!(
        session.approve(review.token()).err(),
        Some(ErrorCode::State)
    );
    assert_no_consent(&mut session, Some(replacement.token()));
    // Begin then cancel mid-stream is idempotent and leaves nothing.
    stream_into(&mut session, &bytes[..half], bytes.len(), 7).unwrap();
    session.cancel();
    session.cancel();
    assert!(!session.test_is_streaming());
    assert_no_consent(&mut session, None);
}

#[test]
fn feeding_after_review_and_signing_before_review_are_state_errors() {
    let bytes = fixture();
    let mut session = fresh_session();
    let review = run(&mut session, &bytes, WHOLE).unwrap().review.unwrap();
    assert_eq!(session.feed(&[0], &keys().0).err(), Some(ErrorCode::State));
    assert_no_consent(&mut session, Some(review.token()));

    let mut session = fresh_session();
    assert_eq!(
        session.feed(&bytes, &keys().0).err(),
        Some(ErrorCode::State),
        "before begin"
    );
    stream_into(&mut session, &bytes[..bytes.len() - 1], bytes.len(), WHOLE).unwrap();
    let foreign = foreign_review();
    assert_eq!(
        session.sign(foreign.token(), &keys().1).err(),
        Some(ErrorCode::State)
    );
    assert!(!session.test_is_streaming());
    stream_into(&mut session, &bytes[..bytes.len() - 1], bytes.len(), WHOLE).unwrap();
    assert_eq!(
        session.approve(foreign.token()).err(),
        Some(ErrorCode::State)
    );
    assert!(!session.test_is_streaming());
    // Too many bytes for the declared length.
    let mut session = fresh_session();
    session
        .begin(bytes.len(), &keys().0, &SEED_FINGERPRINT)
        .unwrap();
    let mut extra = bytes.clone();
    extra.push(0);
    assert_eq!(
        session.feed(&extra, &keys().0).err(),
        Some(ErrorCode::State)
    );
    assert!(!session.test_is_streaming());
}

/// The session keeps only the encoding of the key bound at `begin`; every
/// `feed` must lend that key again, and any other key is a protocol
/// violation that resets the stream. A session bound to a foreign key
/// rejects real spends carrying the device key as `Malformed`, exactly as the
/// engine does through `verify_nullifier`'s FVK mismatch.
#[test]
fn feeding_with_a_different_key_is_a_state_error_and_resets() {
    let bytes = fixture();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([3; 32]).unwrap());
    let mut session = fresh_session();
    let half = bytes.len() / 2;
    stream_into(&mut session, &bytes[..half], bytes.len(), 64).unwrap();
    assert!(session.test_is_streaming());
    assert_eq!(
        session.feed(&bytes[half..], &other).err(),
        Some(ErrorCode::State)
    );
    assert!(!session.test_is_streaming());
    assert_no_consent(&mut session, None);
    assert!(run(&mut session, &bytes, 64).unwrap().review.is_some());

    assert_eq!(
        engine_for(policy(Network::Testnet, 100_000), seed(5))
            .begin(&bytes, &other, &SEED_FINGERPRINT)
            .err(),
        Some(ErrorCode::Malformed)
    );
    for chunk in CHUNKINGS {
        let mut session = fresh_session();
        session
            .begin(bytes.len(), &other, &SEED_FINGERPRINT)
            .unwrap();
        let mut rest = &bytes[..];
        let verdict = loop {
            match session.feed(&rest[..rest.len().min(chunk)], &other) {
                Ok((consumed, Event::Review(_))) => panic!("reviewed after {consumed}"),
                Ok((consumed, _)) => rest = &rest[consumed..],
                Err(error) => break error,
            }
        };
        assert_eq!(verdict, ErrorCode::Malformed, "chunk {chunk}");
        assert!(!session.test_is_streaming());
        assert_no_consent(&mut session, None);
    }
}

#[test]
fn wrong_account_key_releases_no_records() {
    let bytes = build_inputs(&[100_000, 100_000]);
    let mut session = fresh_session();
    let review = run(&mut session, &bytes, WHOLE).unwrap().review.unwrap();
    session.approve(review.token()).unwrap();
    let wrong = SpendAuthorizingKey::from(&SpendingKey::from_bytes([2; 32]).unwrap());
    assert_eq!(
        session.sign(review.token(), &wrong).err(),
        Some(ErrorCode::Signing)
    );
    assert_no_consent(&mut session, Some(review.token()));
    let review = run(&mut session, &bytes, WHOLE).unwrap().review.unwrap();
    session.approve(review.token()).unwrap();
    assert_eq!(
        session
            .sign(review.token(), &keys().1)
            .unwrap()
            .records()
            .len(),
        2
    );
}

#[test]
fn tokens_are_session_bound_and_byte_bound() {
    let bytes = fixture();
    let mut first = session_for(policy(Network::Testnet, 100_000), seed(10));
    let mut second = session_for(policy(Network::Testnet, 100_000), seed(11));
    let a = run(&mut first, &bytes, WHOLE).unwrap().review.unwrap();
    let b = run(&mut second, &bytes, WHOLE).unwrap().review.unwrap();
    assert_ne!(a.token(), b.token());
    assert_eq!(a.sighash(), b.sighash());
    assert_eq!(second.approve(a.token()).err(), Some(ErrorCode::State));
    assert_no_consent(&mut second, Some(b.token()));

    // Same sighash, different consumed bytes: new consent (conformance.rs:114-133).
    for altered in [
        mutate(|v| v["ironwood"]["anchor"] = json!(vec![0; 32])),
        mutate(|v| v["global"]["fallback_lock_time"] = json!(0)),
    ] {
        let mut session = session_for(policy(Network::Testnet, 100_000), seed(12));
        let old = run(&mut session, &bytes, WHOLE).unwrap().review.unwrap();
        session.approve(old.token()).unwrap();
        let new = run(&mut session, &altered, WHOLE).unwrap().review.unwrap();
        assert_eq!(old.sighash(), new.sighash());
        assert_ne!(old.token().context(), new.token().context());
        assert_eq!(
            session.sign(old.token(), &keys().1).err(),
            Some(ErrorCode::State)
        );
        assert_no_consent(&mut session, Some(new.token()));
    }
}
