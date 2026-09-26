//! How outputs are classified and shown, and the outgoing ciphertexts that
//! make them recoverable.

use orchard::keys::Scope;
use serde_json::{Value, json};
use zcash_signer::Error::{Malformed, Policy};
use zcash_signer::OutputKind;
use zcash_signer_tests::*;

fn padding(v: &mut Value) -> &mut Value {
    action(v, |a| a["output"]["value"] == 0)
}

#[test]
fn test_payment_and_change() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let events = review(&wallet, &bytes, 1024).unwrap();
    let summary = events.review().summary();
    assert_eq!(
        (
            summary.input_total,
            summary.payment_total,
            summary.change_total,
            summary.fee
        ),
        (1_000_000, 600_000, 390_000, 10_000)
    );
    assert_eq!(summary.expiry_height, HEIGHT + 40);
    assert_eq!(summary.padding_outputs, 0);
    let kinds: Vec<_> = summary
        .outputs
        .iter()
        .map(|output| (output.kind, output.value, output.receiver))
        .collect();
    let receiver = |to| wallet.address(to).to_raw_address_bytes();
    assert!(kinds.contains(&(OutputKind::Payment, 600_000, receiver(To::Other(0)))));
    assert!(kinds.contains(&(
        OutputKind::InternalChange,
        390_000,
        receiver(To::Internal(1))
    )));
    // Only the payment is confirmed.
    assert_eq!(events.payments.len(), 1);
    let (payment, user_address) = &events.payments[0];
    assert_eq!(
        (payment.kind, payment.value, user_address),
        (OutputKind::Payment, 600_000, &None)
    );
}

/// A payment to one of the account's own external addresses is a payment.
#[test]
fn test_self_payment() {
    let wallet = Wallet::testnet();
    let tx = Tx::pay(
        vec![
            Output {
                to: To::External(0),
                ..Output::payment(600_000)
            },
            Output::change(390_000),
        ],
        Vec::new(),
        10_000,
    );
    let events = review(&wallet, &build(&wallet, &tx).bytes, 1024).unwrap();
    let summary = events.review().summary();
    assert_eq!(
        (summary.payment_total, summary.change_total),
        (600_000, 390_000)
    );
}

/// A payment must be recoverable under the external OVK; change under the
/// internal one or none, never the external one.
#[test]
fn test_outgoing_viewing_keys() {
    let wallet = Wallet::testnet();
    let cases = [
        (
            "payment, internal OVK",
            Some(Scope::Internal),
            Some(Scope::Internal),
            Err(Malformed),
        ),
        (
            "payment, no OVK",
            None,
            Some(Scope::Internal),
            Err(Malformed),
        ),
        (
            "change, external OVK",
            Some(Scope::External),
            Some(Scope::External),
            Err(Malformed),
        ),
        ("change, no OVK", Some(Scope::External), None, Ok(())),
    ];
    for (name, payment_ovk, change_ovk, expected) in cases {
        let tx = Tx::pay(
            vec![
                Output {
                    ovk: payment_ovk,
                    ..Output::payment(600_000)
                },
                Output {
                    ovk: change_ovk,
                    ..Output::change(390_000)
                },
            ],
            Vec::new(),
            10_000,
        );
        assert_outcome(&wallet, name, &build(&wallet, &tx).bytes, expected);
    }
}

#[test]
fn test_padding() {
    let wallet = Wallet::testnet();
    let tx = Tx::pay(vec![Output::payment(990_000)], Vec::new(), 10_000);
    let bytes = build(&wallet, &tx).bytes;
    let events = review(&wallet, &bytes, 1024).unwrap();
    let summary = events.review().summary();
    assert_eq!(
        (
            summary.payment_total,
            summary.change_total,
            summary.padding_outputs
        ),
        (990_000, 0, 1)
    );
    // The padding has a dummy spend, whose signature covers the ciphertext.
    let mutated = mutate(&bytes, |v| {
        flip(&mut padding(v)["output"]["out_ciphertext"])
    });
    assert_outcome(&wallet, "padding out_ciphertext", &mutated, Err(Malformed));
}

/// Padding without an OVK has an `out_ciphertext` no key recovers. Behind a
/// real spend it is bound only by the sighash, so a changed one is a
/// different transaction to approve.
#[test]
fn test_padding_behind_a_real_spend() {
    let wallet = Wallet::testnet();
    let tx = Tx {
        notes: vec![100_000, 100_000],
        ..Tx::pay(vec![Output::payment(190_000)], Vec::new(), 10_000)
    };
    let bytes = build(&wallet, &tx).bytes;
    let mutated = mutate(&bytes, |v| {
        flip(&mut padding(v)["output"]["out_ciphertext"])
    });
    let reviews: Vec<_> = [&bytes, &mutated]
        .into_iter()
        .map(|bytes| {
            let signatures = sign(&wallet, bytes).unwrap();
            assert_signatures_apply(bytes, &signatures);
            review(&wallet, bytes, 1024).unwrap().review.unwrap()
        })
        .collect();
    assert_ne!(reviews[0].token(), reviews[1].token());
}

/// An OCK is optional, but must be the one that recovers the output.
#[test]
fn test_output_recovery_key() {
    let wallet = Wallet::testnet();
    let bytes = with_ocks(&build(&wallet, &Tx::simple()).bytes, &wallet.fvk);
    let cases: [(&str, Edit, Outcome); 3] = [
        ("valid", |_| {}, Ok(())),
        (
            "flipped",
            |v| flip(&mut payment_action(v)["output"]["ock"]),
            Err(Malformed),
        ),
        (
            "zero",
            |v| payment_action(v)["output"]["ock"] = json!(vec![0u8; 32]),
            Err(Malformed),
        ),
    ];
    for (name, edit, expected) in cases {
        assert_outcome(&wallet, name, &mutate(&bytes, edit), expected);
    }
}

/// The wallet's recipient string is shown with the payment it names, is not
/// signed, and must be UTF-8 of at most 512 bytes.
#[test]
fn test_user_address() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let with = |address: &str| {
        mutate(&bytes, |v| {
            payment_action(v)["output"]["user_address"] = json!(address)
        })
    };
    let plain = review(&wallet, &bytes, 1024).unwrap();
    for address in ["u1recipient", &"u".repeat(512)] {
        let named = with(address);
        let events = review(&wallet, &named, 1024).unwrap();
        assert_eq!(events.payments[0].1.as_deref(), Some(address));
        assert_eq!(events.review().summary(), plain.review().summary());
        assert_signatures_apply(&bytes, &sign(&wallet, &named).unwrap());
    }
    assert_outcome(&wallet, "513 bytes", &with(&"u".repeat(513)), Err(Policy));
    let named = with("ab");
    let at = offset_of_difference(&bytes, &named);
    assert_eq!(named[at..at + 4], [1, 2, b'a', b'b']);
    let mut invalid = named;
    invalid[at + 2] = 0xff;
    assert_outcome(&wallet, "not UTF-8", &invalid, Err(Malformed));
}
