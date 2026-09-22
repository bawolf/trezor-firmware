#![cfg(feature = "test")]
//! Differential tests of the streaming digest against the crate's own
//! `effects::sighash`, `zcash_primitives` called directly, the `pczt` signer,
//! and the engine's reviewed sighash.
//!
//! `digest.rs` and `effects.rs` are crate-private and `lib.rs` exposes
//! neither, so both are compiled into this binary from source together with
//! the `wire` and `error` modules they name through `crate::`.

mod common;
#[allow(dead_code)]
#[path = "../src/digest.rs"]
mod digest;
#[allow(dead_code)]
#[path = "../src/effects.rs"]
mod effects;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
#[allow(dead_code)]
#[path = "../src/wire.rs"]
mod wire;

use std::collections::BTreeSet;

use common::*;
use digest::{ActionEffects, Digest};
pub use error::{Error, ErrorCode, Result};
use ironwood::{Engine, Network, ZIP32_HARDENED};
use orchard::ValuePool;
use orchard::bundle::TxVersion;
use orchard::bundle::commitments::hash_bundle_txid_empty;
use pczt::Pczt;
use pczt::orchard::EncCiphertext;
use pczt::roles::signer::Signer;
use pczt::roles::verifier::{OrchardError, Verifier};
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde_json::{Value, json};
use zcash_primitives::transaction::sighash::SignableInput;
use zcash_primitives::transaction::sighash_v6::v6_signature_hash;
use zcash_primitives::transaction::txid::TxIdDigester;
use zcash_primitives::transaction::{Authorization, TransactionData};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::{MAX_MONEY, ZatBalance};

struct EffectsOnly;
impl Authorization for EffectsOnly {
    type TransparentAuth = transparent::bundle::EffectsOnly;
    type SaplingAuth = sapling::bundle::EffectsOnly;
    type OrchardAuth = orchard::bundle::EffectsOnly;
}

struct Nodes {
    header: [u8; 32],
    ironwood: [u8; 32],
    sighash: [u8; 32],
}

/// Feeds the parsed bundle's wire bytes through the streaming digest.
fn streamed(bytes: &[u8], header: &wire::Header) -> Result<Nodes> {
    let pczt = Pczt::parse(bytes).unwrap();
    let bundle = pczt.ironwood();
    let mut digest = Digest::new(header)?;
    for action in bundle.actions() {
        let output = action.output();
        let EncCiphertext::Encrypted(enc_ciphertext) = output.enc_ciphertext() else {
            panic!("fixtures carry encrypted notes");
        };
        digest.action(&ActionEffects {
            cv_net: action.cv_net().as_ref().unwrap(),
            nullifier: action.spend().nullifier(),
            rk: action.spend().rk(),
            cmx: output.cmx().as_ref().unwrap(),
            ephemeral_key: output.ephemeral_key(),
            enc_ciphertext: enc_ciphertext.as_slice().try_into().unwrap(),
            out_ciphertext: output.out_ciphertext().as_slice().try_into().unwrap(),
        });
    }
    let (magnitude, negative) = *bundle.value_sum();
    let magnitude = i64::try_from(magnitude).unwrap();
    let value_balance = if negative { -magnitude } else { magnitude };
    let flags = *bundle.flags();
    Ok(Nodes {
        header: digest.header_digest(),
        ironwood: digest.ironwood_digest(flags, value_balance),
        sighash: digest.finish(flags, value_balance),
    })
}

/// The crate's `effects::sighash` over the upstream-parsed bundle, checked
/// against `zcash_primitives` called directly, with upstream's nodes.
fn reference(bytes: &[u8], header: &wire::Header) -> Result<Nodes> {
    let mut nodes = None;
    Verifier::new(Pczt::parse(bytes).unwrap())
        .with_ironwood(|bundle| -> std::result::Result<(), OrchardError<Error>> {
            let sighash = effects::sighash(bundle, header).map_err(OrchardError::Custom)?;
            let tx: TransactionData<EffectsOnly> = TransactionData::from_parts_v6(
                BranchId::try_from(header.branch).unwrap(),
                header.lock_time,
                header.expiry.into(),
                None,
                None,
                None,
                bundle.extract_effects::<ZatBalance>().unwrap(),
            );
            let parts = tx.digest(TxIdDigester);
            assert!(parts.transparent_digests.is_none());
            assert!(parts.sapling_digest.is_none());
            assert!(parts.orchard_digest.is_none());
            assert_eq!(
                v6_signature_hash(&tx, &SignableInput::Shielded, &parts).as_bytes(),
                sighash
            );
            assert_eq!(parts.ironwood_digest.is_none(), bundle.actions().is_empty());
            let empty = hash_bundle_txid_empty(ValuePool::Ironwood, TxVersion::V6).unwrap();
            nodes = Some(Nodes {
                header: parts.header_digest.as_bytes().try_into().unwrap(),
                ironwood: parts
                    .ironwood_digest
                    .unwrap_or(empty)
                    .as_bytes()
                    .try_into()
                    .unwrap(),
                sighash,
            });
            Ok(())
        })
        .map_err(|error| match error {
            OrchardError::Custom(error) => error,
            _ => Error::malformed(),
        })?;
    Ok(nodes.unwrap())
}

#[track_caller]
fn signer(bytes: &[u8]) -> [u8; 32] {
    Signer::new(Pczt::parse(bytes).unwrap())
        .unwrap()
        .shielded_sighash()
}

/// Streaming and reference must agree node by node.
#[track_caller]
fn agree_with(bytes: &[u8], header: &wire::Header) -> [u8; 32] {
    let streamed = streamed(bytes, header).unwrap();
    let reference = reference(bytes, header).unwrap();
    assert_eq!(streamed.header, reference.header, "header digest");
    assert_eq!(streamed.ironwood, reference.ironwood, "ironwood digest");
    assert_eq!(streamed.sighash, reference.sighash, "shielded sighash");
    streamed.sighash
}

/// Adds the `pczt` signer, which derives the header from the PCZT itself.
#[track_caller]
fn agree_signed(bytes: &[u8], header: &wire::Header) -> [u8; 32] {
    let sighash = agree_with(bytes, header);
    assert_eq!(signer(bytes), sighash, "pczt signer sighash");
    sighash
}

/// Adds admission, which is where the device takes its header from.
#[track_caller]
fn agree(bytes: &[u8]) -> [u8; 32] {
    agree_signed(bytes, &wire::scan(bytes).unwrap())
}

#[track_caller]
fn rejected(bytes: &[u8], header: &wire::Header) {
    assert_eq!(streamed(bytes, header).err(), Some(Error::policy()));
    assert_eq!(reference(bytes, header).err(), Some(Error::policy()));
}

fn actions(bytes: &[u8]) -> usize {
    Pczt::parse(bytes).unwrap().ironwood().actions().len()
}

fn mutate_bytes(bytes: &[u8], f: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = json(bytes);
    f(&mut value);
    encode(value)
}

fn flip_at(value: &mut Value, index: usize) {
    let n = value[index].as_u64().unwrap();
    value[index] = (n ^ 1).into();
}

fn swap_actions(value: &mut Value, path: &[&str]) {
    let actions = &mut value["ironwood"]["actions"];
    let mut a = actions[0].clone();
    let mut b = actions[1].clone();
    let (mut pa, mut pb) = (&mut a, &mut b);
    for key in path {
        pa = &mut pa[*key];
        pb = &mut pb[*key];
    }
    std::mem::swap(pa, pb);
    actions[0] = a;
    actions[1] = b;
}

fn corpus() -> Vec<(String, Vec<u8>)> {
    let mut corpus = vec![
        ("fixture".into(), fixture()),
        (
            "self_payment".into(),
            build(600_000, 390_000, MemoBytes::empty(), true),
        ),
        (
            "memo".into(),
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes(b"streaming digest").unwrap(),
                false,
            ),
        ),
        ("wrong_change_ovk".into(), build_with_wrong_change_ovk()),
        (
            "discarded_payment_ovk".into(),
            build_with_discarded_payment_ovk(),
        ),
        (
            "dummy_spend_padding".into(),
            build_with_dummy_spend_padding(),
        ),
        ("mainnet".into(), network_fixture(Network::Mainnet)),
        ("testnet".into(), network_fixture(Network::Testnet)),
    ];
    for outputs in 1..=8 {
        corpus.push((format!("outputs_{outputs}"), build_actions(outputs)));
    }
    for inputs in 1..=8 {
        let values: Vec<u64> = (0..inputs).map(|i| 100_000 + i * 10_000).collect();
        corpus.push((format!("inputs_{inputs}"), build_inputs(&values)));
    }
    corpus
}

#[test]
fn corpus_agrees_on_every_path_and_covers_one_to_eight_actions() {
    let mut counts = BTreeSet::new();
    let mut digests = BTreeSet::new();
    for (name, bytes) in corpus() {
        let sighash = agree(&bytes);
        assert!(digests.insert(sighash), "{name} repeats a digest");
        counts.insert(actions(&bytes));
    }
    assert_eq!(counts, (1..=8).collect());
}

#[test]
fn engine_review_reports_the_streaming_sighash() {
    let mut engines = [
        (Network::Testnet, engine()),
        (
            Network::Mainnet,
            Engine::with_rng(
                policy(Network::Mainnet, 100_000),
                ChaCha20Rng::from_seed([7; 32]),
            )
            .unwrap(),
        ),
    ];
    let mut reviewed = 0;
    for (name, bytes) in corpus() {
        for (network, engine) in &mut engines {
            if let Ok(review) = engine.begin_test(&bytes) {
                assert_eq!(*review.sighash(), agree(&bytes), "{name} on {network:?}");
                reviewed += 1;
            }
        }
    }
    // Everything but the two outgoing-key variants (a payment without an OVK,
    // change under the external OVK) passes device policy; a nonempty memo is
    // shown, not refused (design §11).
    assert_eq!(reviewed, corpus().len() - 2);
}

#[test]
fn header_fields_are_bound_exactly_as_upstream() {
    let base = agree(&fixture());
    let mut digests = BTreeSet::from([base]);
    // Only Nu6_3 admits the V6 format, so the pczt signer refuses every other
    // branch (`UnsupportedConsensusBranchId`); the effects path hashes any.
    for branch in [
        BranchId::Sprout,
        BranchId::Sapling,
        BranchId::Blossom,
        BranchId::Heartwood,
        BranchId::Canopy,
        BranchId::Nu5,
        BranchId::Nu6,
        BranchId::Nu6_1,
        BranchId::Nu6_2,
    ] {
        let bytes = mutate(|v| v["global"]["consensus_branch_id"] = u32::from(branch).into());
        let header = wire::scan(&bytes).unwrap();
        assert!(digests.insert(agree_with(&bytes, &header)), "{branch:?}");
    }
    for branch in [1u32, 2, 0xdead_beef, u32::MAX] {
        let bytes = mutate(|v| v["global"]["consensus_branch_id"] = branch.into());
        let mut header = wire::scan(&fixture()).unwrap();
        header.branch = branch;
        rejected(&bytes, &header);
    }
    for lock_time in [1u32, 2, u32::MAX] {
        let bytes = mutate(|v| v["global"]["fallback_lock_time"] = lock_time.into());
        assert!(digests.insert(agree(&bytes)), "lock time {lock_time}");
    }
    assert_eq!(
        agree(&mutate(|v| v["global"]["fallback_lock_time"] = Value::Null)),
        agree(&mutate(|v| v["global"]["fallback_lock_time"] = 0.into())),
    );
    for expiry in [0u32, 1, HEIGHT, u32::MAX] {
        let bytes = mutate(|v| v["global"]["expiry_height"] = expiry.into());
        assert!(digests.insert(agree(&bytes)), "expiry {expiry}");
    }
    // Version, group and coin type are fixed or ignored by both paths; the
    // pczt signer would build another format, so it is not consulted. Outside
    // V6 the upstream verifier insists on an anchor, which the sighash ignores.
    for (field, value) in [
        ("tx_version", 5u32),
        ("version_group_id", 0),
        ("coin_type", 133),
    ] {
        let bytes = mutate(|v| {
            v["global"][field] = value.into();
            v["ironwood"]["anchor"] = json!(vec![0; 32]);
        });
        assert_eq!(
            agree_with(&bytes, &wire::scan(&bytes).unwrap()),
            base,
            "{field}"
        );
    }
}

#[test]
fn bundle_fields_are_bound_exactly_as_upstream() {
    let base = agree(&fixture());
    let mut digests = BTreeSet::new();
    for flags in 0u8..=7 {
        let bytes = mutate(|v| v["ironwood"]["flags"] = flags.into());
        assert!(digests.insert(agree(&bytes)), "flags {flags}");
    }
    assert!(digests.contains(&base));
    // Reserved bits do not parse upstream, so there is nothing to compare.
    for flags in [8u8, 0x80, 0xff] {
        let bytes = mutate(|v| v["ironwood"]["flags"] = flags.into());
        let header = wire::scan(&bytes).unwrap();
        assert_eq!(reference(&bytes, &header).err(), Some(Error::malformed()));
    }
    // Admission refuses negative sums by policy, so those take the base header.
    let header = wire::scan(&fixture()).unwrap();
    let mut balances = BTreeSet::new();
    for (magnitude, negative) in [
        (0u64, false),
        (1, false),
        (1, true),
        (10_000, true),
        (MAX_MONEY, false),
        (MAX_MONEY, true),
    ] {
        let bytes = mutate(|v| v["ironwood"]["value_sum"] = json!([magnitude, negative]));
        if negative {
            assert_eq!(wire::scan(&bytes).err(), Some(Error::policy()));
        } else {
            assert!(wire::scan(&bytes).is_ok());
        }
        assert!(
            balances.insert(agree_signed(&bytes, &header)),
            "{magnitude} {negative}"
        );
    }
    assert_eq!(
        agree_signed(
            &mutate(|v| v["ironwood"]["value_sum"] = json!([0, true])),
            &header
        ),
        agree(&mutate(|v| v["ironwood"]["value_sum"] = json!([0, false]))),
    );
    // Beyond MAX_MONEY the upstream effects are unrepresentable; admission
    // rejects the same bytes before any digest is attempted.
    let bytes = mutate(|v| v["ironwood"]["value_sum"] = json!([MAX_MONEY + 1, false]));
    assert_eq!(wire::scan(&bytes).err(), Some(Error::malformed()));
    assert_eq!(reference(&bytes, &header).err(), Some(Error::malformed()));
    // The anchor belongs to the authorizing digest only.
    assert_eq!(
        agree(&mutate(|v| v["ironwood"]["anchor"] = json!(vec![0; 32]))),
        base
    );
    assert_eq!(
        agree(&mutate(|v| v["ironwood"]["anchor"] = json!(vec![7; 32]))),
        base
    );
}

#[test]
fn action_effects_are_bound_exactly_as_upstream() {
    for (name, bytes) in [("fixture", fixture()), ("outputs_8", build_actions(8))] {
        let base = agree(&bytes);
        let mut digests = BTreeSet::from([base]);
        for path in [
            &["cv_net"][..],
            &["spend", "nullifier"],
            &["spend", "rk"],
            &["output", "cmx"],
            &["output", "ephemeral_key"],
            &["output", "enc_ciphertext", "Encrypted"],
            &["output", "out_ciphertext"],
        ] {
            let swapped = mutate_bytes(&bytes, |v| swap_actions(v, path));
            assert!(digests.insert(agree(&swapped)), "{name} swap {path:?}");
        }
        // Every byte of the compact, memo and noncompact ciphertext segments.
        for index in [0, 51, 52, 563, 564, 579] {
            let flipped = mutate_bytes(&bytes, |v| {
                flip_at(
                    &mut v["ironwood"]["actions"][1]["output"]["enc_ciphertext"]["Encrypted"],
                    index,
                )
            });
            assert!(digests.insert(agree(&flipped)), "{name} enc {index}");
        }
        for index in [0, 79] {
            let flipped = mutate_bytes(&bytes, |v| {
                flip_at(
                    &mut v["ironwood"]["actions"][1]["output"]["out_ciphertext"],
                    index,
                )
            });
            assert!(digests.insert(agree(&flipped)), "{name} out {index}");
        }
        // An ephemeral key must stay a valid point for upstream extraction, so
        // it is replaced by another fixture's rather than flipped. The
        // streaming digest hashes any bytes; point validity is an admission
        // duty, not a digest one.
        let foreign =
            json(&build_actions(3))["ironwood"]["actions"][0]["output"]["ephemeral_key"].clone();
        let replaced = mutate_bytes(&bytes, |v| {
            v["ironwood"]["actions"][1]["output"]["ephemeral_key"] = foreign;
        });
        assert!(digests.insert(agree(&replaced)), "{name} epk");
        let invalid = mutate_bytes(&bytes, |v| {
            flip_at(
                &mut v["ironwood"]["actions"][1]["output"]["ephemeral_key"],
                0,
            )
        });
        let header = wire::scan(&invalid).unwrap();
        assert!(streamed(&invalid, &header).is_ok());
        assert_eq!(reference(&invalid, &header).err(), Some(Error::malformed()));
        let reversed = mutate_bytes(&bytes, |v| {
            let actions = v["ironwood"]["actions"].as_array_mut().unwrap();
            actions.reverse();
        });
        assert!(digests.insert(agree(&reversed)), "{name} reversed");
        let single = mutate_bytes(&bytes, |v| {
            let first = v["ironwood"]["actions"][0].clone();
            v["ironwood"]["actions"] = json!([first]);
        });
        assert_eq!(actions(&single), 1);
        assert!(digests.insert(agree(&single)), "{name} single");
        // Non-effecting fields leave every path unchanged.
        for (path, value) in [
            (&["spend", "value"][..], json!(123)),
            (&["output", "value"], json!(456)),
            (&["spend", "spend_auth_sig"], Value::Null),
            (&["output", "ock"], Value::Null),
            (&["output", "rseed"], json!(vec![9; 32])),
        ] {
            let unchanged = mutate_bytes(&bytes, |v| {
                let mut field = &mut v["ironwood"]["actions"][0];
                for key in path {
                    field = &mut field[*key];
                }
                *field = value;
            });
            assert_eq!(agree(&unchanged), base, "{name} {path:?}");
        }
    }
}

#[test]
fn no_actions_substitutes_the_empty_ironwood_digest() {
    let bytes = mutate(|v| v["ironwood"]["actions"] = json!([]));
    assert_eq!(actions(&bytes), 0);
    assert_eq!(wire::scan(&bytes).err(), Some(Error::policy()));
    let header = wire::scan(&fixture()).unwrap();
    let sighash = agree_with(&bytes, &header);
    assert_eq!(signer(&bytes), sighash);
    assert_ne!(sighash, agree(&fixture()));
    // Flags and balance are not hashed once the bundle is absent.
    let flagged = mutate_bytes(&bytes, |v| {
        v["ironwood"]["flags"] = 0.into();
        v["ironwood"]["value_sum"] = json!([1, true]);
    });
    assert_eq!(agree_with(&flagged, &header), sighash);
}
