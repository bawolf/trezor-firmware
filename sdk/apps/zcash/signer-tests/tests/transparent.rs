//! Transparent outputs: shown by the address their script pays, counted
//! against the action limit, and paid out of the Ironwood value sum.

use serde_json::{Value, json};
use zcash_protocol::value::MAX_MONEY;
use zcash_signer::Error::{Capacity, Malformed, Policy};
use zcash_signer::{Error, Event, MAX_ACTIONS, TransparentKind};
use zcash_signer_tests::*;

fn deshield(outputs: Vec<(u64, Vec<u8>)>) -> Tx {
    Tx::pay(
        vec![Output::payment(600_000), Output::change(390_000)],
        outputs,
        10_000,
    )
}

#[test]
fn test_transparent_outputs() {
    for (wallet, count) in [
        (Wallet::testnet(), 0),
        (Wallet::testnet(), 1),
        (Wallet::testnet(), 2),
        (Wallet::testnet(), 4),
        (Wallet::mainnet(), 2),
    ] {
        let outputs = transparent_outputs(count);
        let total: u64 = outputs.iter().map(|(value, _)| value).sum();
        let bytes = build(&wallet, &deshield(outputs.clone())).bytes;
        for chunk in CHUNKINGS {
            let events = review(&wallet, &bytes, chunk).unwrap();
            let summary = events.review().summary();
            assert_eq!(events.transparent, summary.transparent_outputs);
            assert_eq!(summary.transparent_total, total);
            // The fee is what the transparent outputs leave of the value sum.
            assert_eq!(
                (
                    summary.input_total,
                    summary.payment_total,
                    summary.change_total,
                    summary.fee
                ),
                (1_000_000 + total, 600_000, 390_000, 10_000)
            );
            for (index, (output, (value, script))) in
                events.transparent.iter().zip(&outputs).enumerate()
            {
                let (kind, hash) = if index % 2 == 0 {
                    (TransparentKind::P2pkh, &script[3..23])
                } else {
                    (TransparentKind::P2sh, &script[2..22])
                };
                assert_eq!(
                    (output.index, output.kind, output.value),
                    (index, kind, *value)
                );
                assert_eq!(output.hash, hash);
            }
        }
        assert_signatures_apply(&bytes, &sign(&wallet, &bytes).unwrap());
    }
}

/// Transparent outputs are paid from the value sum, so a PCZT whose value
/// sum does not cover them has no valid fee.
#[test]
fn test_value_sum_pays_transparent_outputs() {
    let wallet = Wallet::testnet();
    let with_output = deshield(transparent_outputs(1));
    let without_output = Tx {
        transparent: Vec::new(),
        ..with_output.clone()
    };
    let unpaid = Tx {
        notes: vec![1_000_000],
        ..with_output
    };
    // The same value sum without a transparent bundle is all fee.
    let bytes = build(&wallet, &without_output).bytes;
    let events = review(&wallet, &bytes, 1024).unwrap();
    assert_eq!(events.review().summary().fee, 110_000);
    assert_outcome(
        &wallet,
        "unpaid",
        &build(&wallet, &unpaid).bytes,
        Err(Malformed),
    );
}

/// Every field of every output is signed: each variant signs with the sighash
/// librustzcash computes for it.
#[test]
fn test_outputs_signed() {
    let wallet = Wallet::testnet();
    let outputs = transparent_outputs(2);
    let (a, b) = (outputs[0].clone(), outputs[1].clone());
    let variants = [
        ("as built", vec![a.clone(), b.clone()]),
        ("reordered", vec![b.clone(), a.clone()]),
        ("value", vec![(a.0 + 1, a.1.clone()), b.clone()]),
        ("hash", vec![(a.0, p2pkh([0xc7; 20])), b.clone()]),
        ("P2SH for P2PKH", vec![(a.0, p2sh([0xa0; 20])), b.clone()]),
    ];
    for (name, outputs) in variants {
        let bytes = build(&wallet, &deshield(outputs)).bytes;
        let signatures = sign(&wallet, &bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        assert_signatures_apply(&bytes, &signatures);
    }
}

/// A transparent output's `user_address` is neither shown nor signed.
#[test]
fn test_user_address_ignored() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &deshield(transparent_outputs(1))).bytes;
    let named = mutate(&bytes, |v| {
        v["transparent"]["outputs"][0]["user_address"] = json!("t".repeat(512));
    });
    let plain = review(&wallet, &bytes, 1024).unwrap();
    let events = review(&wallet, &named, 1024).unwrap();
    assert_eq!(events.review().summary(), plain.review().summary());
    assert_signatures_apply(&bytes, &sign(&wallet, &named).unwrap());
}

fn input() -> Value {
    json!({
        "prevout_txid": vec![0x11u8; 32],
        "prevout_index": 0,
        "sequence": null,
        "required_time_lock_time": null,
        "required_height_lock_time": null,
        "script_sig": null,
        "value": 1_000_000,
        "script_pubkey": p2pkh([0x22; 20]),
        "redeem_script": null,
        "partial_signatures": {},
        "sighash_type": 1,
        "bip32_derivation": {},
        "ripemd160_preimages": {},
        "sha256_preimages": {},
        "hash160_preimages": {},
        "hash256_preimages": {},
        "proprietary": {},
    })
}

fn with_script(script: &[u8]) -> Value {
    json!([transparent_output_json(100_000, script)])
}

/// Asserts the outcome of each edit of a PCZT with one transparent output.
fn assert_edits(edits: &[(&str, Edit)], expected: Error) {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &deshield(transparent_outputs(1))).bytes;
    for (name, edit) in edits {
        assert_outcome(&wallet, name, &mutate(&bytes, edit), Err(expected));
    }
}

#[test]
fn test_bundle_shape() {
    assert_edits(
        &[
            ("input", |v| v["transparent"]["inputs"] = json!([input()])),
            ("no outputs", |v| v["transparent"]["outputs"] = json!([])),
        ],
        Policy,
    );
}

/// Only P2PKH and P2SH scripts pay an address the device can show.
#[test]
fn test_scripts() {
    assert_edits(
        &[
            ("OP_RETURN", |v| {
                v["transparent"]["outputs"] = with_script(&[0x6a, 0x14])
            }),
            ("P2PK", |v| {
                v["transparent"]["outputs"] =
                    with_script(&[&[0x21][..], &[2; 33], &[0xac]].concat())
            }),
            ("P2PKH, wrong opcode", |v| {
                v["transparent"]["outputs"] =
                    with_script(&[&[0x77][..], &p2pkh([0x31; 20])[1..]].concat())
            }),
            ("P2SH, wrong opcode", |v| {
                v["transparent"]["outputs"] =
                    with_script(&[&p2sh([0x31; 20])[..22], &[0x88]].concat())
            }),
            ("P2PKH, short", |v| {
                v["transparent"]["outputs"] = with_script(&p2pkh([0x31; 20])[..24])
            }),
            ("P2PKH around 18 bytes", |v| {
                v["transparent"]["outputs"] =
                    with_script(&[&[0x76, 0xa9, 0x14][..], &[0xaa; 18], &[0x88, 0xac]].concat())
            }),
            ("P2SH around 22 bytes", |v| {
                v["transparent"]["outputs"] =
                    with_script(&[&[0xa9, 0x14][..], &[0xaa; 22], &[0x87]].concat())
            }),
            ("empty script", |v| {
                v["transparent"]["outputs"] = with_script(&[])
            }),
        ],
        Policy,
    );
}

#[test]
fn test_zero_value() {
    assert_edits(
        &[("zero", |v| {
            v["transparent"]["outputs"][0]["value"] = json!(0)
        })],
        Policy,
    );
}

#[test]
fn test_value_past_max_money() {
    assert_edits(
        &[("past MAX_MONEY", |v| {
            v["transparent"]["outputs"][0]["value"] = json!(MAX_MONEY + 1)
        })],
        Malformed,
    );
}

#[test]
fn test_output_fields() {
    assert_edits(
        &[
            ("redeem script", |v| {
                v["transparent"]["outputs"][0]["redeem_script"] = json!(p2sh([0x44; 20]))
            }),
            ("proprietary", |v| {
                v["transparent"]["outputs"][0]["proprietary"] = json!({ "x": [1] })
            }),
            ("user_address past 512 bytes", |v| {
                v["transparent"]["outputs"][0]["user_address"] = json!("t".repeat(513))
            }),
        ],
        Policy,
    );
}

/// A transparent output's proprietary count read to its tenth byte.
#[test]
fn test_ten_byte_varint() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &deshield(transparent_outputs(1))).bytes;
    let other = mutate(&bytes, |v| {
        v["transparent"]["outputs"][0]["proprietary"] = json!({ "x": [1] });
    });
    let at = offset_of_difference(&bytes, &other);
    let wide = [&bytes[..at], &varint(1 << 63), &bytes[at + 1..]].concat();
    assert_outcome(&wallet, "proprietary count", &wide, Err(Policy));
}

/// Transparent outputs count against `MAX_ACTIONS` with the Ironwood actions.
#[test]
fn test_action_limit() {
    let wallet = Wallet::testnet();
    let outputs = |count| {
        transparent_outputs(count)
            .into_iter()
            .map(|(_, script)| (1, script))
            .collect()
    };
    let one_action = |count| Tx {
        padded: false,
        ..Tx::pay(vec![Output::payment(600_000)], outputs(count), 10_000)
    };
    let cases = [
        ("one action", one_action(MAX_ACTIONS - 1), Ok(())),
        ("two actions", deshield(outputs(MAX_ACTIONS - 2)), Ok(())),
        (
            "two actions, one output too many",
            deshield(outputs(MAX_ACTIONS - 1)),
            Err(Capacity),
        ),
        (
            "more outputs than actions",
            deshield(outputs(MAX_ACTIONS)),
            Err(Capacity),
        ),
    ];
    for (name, tx, expected) in cases {
        let bytes = build(&wallet, &tx).bytes;
        assert_outcome(&wallet, name, &bytes, expected);
        if expected.is_err() {
            assert_no_event_before_error(&wallet, &bytes);
        }
    }
}

/// The action count follows the transparent outputs, which are held back
/// until it is known to fit.
fn assert_no_event_before_error(wallet: &Wallet, bytes: &[u8]) {
    for chunk in CHUNKINGS {
        let mut session = wallet.session();
        wallet.begin(&mut session, bytes.len()).unwrap();
        let mut rest = bytes;
        loop {
            match session.feed(&rest[..rest.len().min(chunk)], &wallet.fvk, &mut || {}) {
                Ok((consumed, Event::NeedMore)) => rest = &rest[consumed..],
                Ok((_, event)) => panic!("chunk {chunk}: {event:?}"),
                Err(_) => break,
            }
        }
    }
}
