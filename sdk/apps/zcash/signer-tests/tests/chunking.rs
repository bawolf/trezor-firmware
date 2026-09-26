//! The PCZT arrives in chunks of any size, and must end exactly where it was
//! declared to.

use serde_json::json;
use zcash_signer::Error::{Capacity, Malformed, Policy, State};
use zcash_signer::{Event, MAX_PCZT_BYTES, Review, TransparentKind};
use zcash_signer_tests::*;

fn corpus() -> Vec<(&'static str, Wallet, Tx)> {
    let named = Output {
        user_address: Some("u1recipient".into()),
        ..Output::payment(600_000)
    };
    let eight_notes = Tx {
        notes: vec![110_000; 8],
        ..Tx::pay(
            (0..8).map(|_| Output::payment(100_000)).collect(),
            Vec::new(),
            80_000,
        )
    };
    vec![
        ("simple", Wallet::testnet(), Tx::simple()),
        ("mainnet", Wallet::mainnet(), Tx::simple()),
        (
            "transparent",
            Wallet::testnet(),
            Tx::pay(
                vec![Output::payment(600_000)],
                transparent_outputs(4),
                10_000,
            ),
        ),
        (
            "full view",
            Wallet::testnet(),
            Tx {
                view: View::Full,
                ..Tx::pay(vec![named], Vec::new(), 10_000)
            },
        ),
        ("eight notes", Wallet::testnet(), eight_notes),
    ]
}

/// Every chunking shows the same outputs, reviews the same summary, and binds
/// the same token, which matches what librustzcash parses.
#[test]
fn test_chunkings_agree() {
    for (name, wallet, tx) in corpus() {
        let bytes = build(&wallet, &tx).bytes;
        let whole = review(&wallet, &bytes, usize::MAX).unwrap();
        for chunk in [1, 7, 64, 1024] {
            let events = review(&wallet, &bytes, chunk).unwrap();
            assert_eq!(events.payments, whole.payments, "{name}: chunk {chunk}");
            assert_eq!(
                events.transparent, whole.transparent,
                "{name}: chunk {chunk}"
            );
            assert_eq!(
                events.review().summary(),
                whole.review().summary(),
                "{name}"
            );
            assert_eq!(events.review().token(), whole.review().token(), "{name}");
        }
        assert_matches_librustzcash(&bytes, whole.review());
    }
}

/// The outputs reviewed are the ones librustzcash parses.
fn assert_matches_librustzcash(bytes: &[u8], review: &Review) {
    let parsed = json(bytes);
    let summary = review.summary();
    assert_eq!(summary.expiry_height, parsed["global"]["expiry_height"]);
    for output in &summary.outputs {
        let action = &parsed["ironwood"]["actions"][output.action_index]["output"];
        assert_eq!(output.receiver, byte_array::<43>(&action["recipient"]));
        assert_eq!(output.value, action["value"]);
    }
    let transparent = parsed["transparent"]["outputs"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    assert_eq!(summary.transparent_outputs.len(), transparent.len());
    for (output, expected) in summary.transparent_outputs.iter().zip(transparent) {
        let script = match output.kind {
            TransparentKind::P2pkh => p2pkh(output.hash),
            TransparentKind::P2sh => p2sh(output.hash),
        };
        assert_eq!(json!(script), expected["script_pubkey"]);
        assert_eq!(output.value, expected["value"]);
    }
}

/// A PCZT cut short of its declared length waits for more; one declared as
/// short as the cut cannot end there.
#[test]
fn test_truncation() {
    let wallet = Wallet::testnet();
    let tx = Tx::pay(
        vec![Output::payment(600_000)],
        transparent_outputs(2),
        10_000,
    );
    let bytes = build(&wallet, &tx).bytes;
    let foreign = review(&wallet, &bytes, usize::MAX).unwrap().review.unwrap();
    for cut in 0..bytes.len() {
        let mut session = wallet.session();
        let outcome = stream(&mut session, &wallet, &bytes[..cut], cut, 1024).map(|_| ());
        assert_eq!(outcome, Err(Malformed), "declared {cut}");
    }
    // Byte by byte, the review comes with the last byte.
    let mut session = wallet.session();
    wallet.begin(&mut session, bytes.len()).unwrap();
    let mut offset = 0;
    while offset < bytes.len() {
        let (consumed, event) = session
            .feed(&bytes[offset..offset + 1], &wallet.fvk, &mut || {})
            .unwrap();
        offset += consumed;
        assert_eq!(matches!(event, Event::Review(_)), offset == bytes.len());
    }
    // Waiting for more holds nothing to approve.
    let mut session = wallet.session();
    stream(
        &mut session,
        &wallet,
        &bytes[..bytes.len() - 1],
        bytes.len(),
        1024,
    )
    .unwrap();
    assert_eq!(session.approve(foreign.token()), Err(State));
}

#[test]
fn test_declared_length() {
    let wallet = Wallet::testnet();
    let mut session = wallet.session();
    assert_eq!(
        wallet.begin(&mut session, MAX_PCZT_BYTES + 1),
        Err(Capacity)
    );
    assert_eq!(wallet.begin(&mut session, MAX_PCZT_BYTES), Ok(()));
    for length in 0..8 {
        assert_eq!(
            wallet.begin(&mut session, length),
            Err(Malformed),
            "{length}"
        );
    }
    // A trailing byte, declared.
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let trailing = [&bytes[..], &[0]].concat();
    assert_outcome(&wallet, "trailing byte", &trailing, Err(Malformed));
}

/// A refusal at the trailer comes after the payments were shown, and leaves
/// nothing to approve.
#[test]
fn test_refusal_at_trailer() {
    let wallet = Wallet::testnet();
    let tx = Tx {
        notes: vec![110_000; 8],
        ..Tx::pay(
            (0..8).map(|_| Output::payment(100_000)).collect(),
            Vec::new(),
            80_000,
        )
    };
    let bytes = build(&wallet, &tx).bytes;
    let foreign = review(&wallet, &bytes, usize::MAX).unwrap().review.unwrap();
    let cases: [(&str, Edit, _); 4] = [
        (
            "value sum",
            |v| v["ironwood"]["value_sum"][0] = json!(1),
            Malformed,
        ),
        ("flags", |v| v["ironwood"]["flags"] = json!(0), Policy),
        (
            "reserved flag",
            |v| v["ironwood"]["flags"] = json!(0x80),
            Malformed,
        ),
        (
            "anchor",
            |v| v["ironwood"]["anchor"] = json!(vec![0xffu8; 32]),
            Malformed,
        ),
    ];
    for (name, edit, expected) in cases {
        let mutated = mutate(&bytes, edit);
        for chunk in CHUNKINGS {
            let mut session = wallet.session();
            wallet.begin(&mut session, mutated.len()).unwrap();
            let mut rest = &mutated[..];
            let mut shown = 0;
            let error = loop {
                match session.feed(&rest[..rest.len().min(chunk)], &wallet.fvk, &mut || {}) {
                    Ok((consumed, event)) => {
                        rest = &rest[consumed..];
                        match event {
                            Event::ConfirmOutput { .. } => shown += 1,
                            Event::NeedMore => {}
                            event => panic!("{name}: {event:?}"),
                        }
                    }
                    Err(error) => break error,
                }
            };
            assert_eq!((error, shown), (expected, 8), "{name}: chunk {chunk}");
            assert_eq!(session.approve(foreign.token()), Err(State));
        }
    }
}
