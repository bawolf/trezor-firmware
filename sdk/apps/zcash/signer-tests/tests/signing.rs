//! Spend authorization signatures. Each is applied with the `pczt` Signer,
//! which verifies it against the sighash librustzcash computes, so a
//! signature that applies was made over the right digest.

use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use serde_json::{Value, json};
use zcash_signer::{Event, Hedged, MAX_ACTIONS, Session, Signatures};
use zcash_signer_tests::*;

/// `count` outputs of 100_000, alternately payments and change, in as many
/// actions.
fn actions(count: usize, transparent: usize) -> Tx {
    let outputs = (0..count)
        .map(|i| {
            if i % 2 == 0 {
                Output::payment(100_000)
            } else {
                Output::change(100_000)
            }
        })
        .collect();
    Tx {
        padded: false,
        ..Tx::pay(
            outputs,
            transparent_outputs(transparent),
            zip317_fee(count + transparent),
        )
    }
}

#[test]
fn test_payment_and_change() {
    let wallet = Wallet::testnet();
    let built = build(&wallet, &Tx::simple());
    let mut bytes = built.bytes.clone();
    let mut session = wallet.session();
    let events = stream(&mut session, &wallet, &bytes, bytes.len(), 1024).unwrap();
    // The session keeps nothing of the caller's buffer.
    bytes.fill(0);
    let token = events.review().token();
    session.approve(token).unwrap();
    let signatures = session.sign(token, &wallet.ask, &mut || {}).unwrap();
    assert_eq!(signatures.records().len(), 1);
    assert_eq!(
        usize::from(signatures.records()[0].action_index),
        built.spends[0]
    );
    assert_signatures_apply(&built.bytes, &signatures);
}

#[test]
fn test_action_counts() {
    let wallet = Wallet::testnet();
    for count in 1..=MAX_ACTIONS {
        let tx = Tx {
            view: [View::Bare, View::Full][count % 2],
            ..actions(count, 0)
        };
        let mut bytes = build(&wallet, &tx).bytes;
        if count % 3 == 0 {
            bytes = with_ocks(&bytes, &wallet.fvk);
        }
        let events = review(&wallet, &bytes, 1024).unwrap();
        assert_eq!(events.review().summary().outputs.len(), count);
        assert_eq!(events.payments.len(), count.div_ceil(2));
        assert_signatures_apply(&bytes, &sign(&wallet, &bytes).unwrap());
    }
}

#[test]
fn test_action_counts_with_transparent_outputs() {
    let wallet = Wallet::testnet();
    for (count, transparent) in [
        (1, 1),
        (2, 4),
        (16, 16),
        (1, MAX_ACTIONS - 1),
        (MAX_ACTIONS - 1, 1),
    ] {
        for view in [View::Bare, View::Full] {
            let tx = Tx {
                view,
                ..actions(count, transparent)
            };
            let bytes = build(&wallet, &tx).bytes;
            assert_signatures_apply(&bytes, &sign(&wallet, &bytes).unwrap());
        }
    }
}

/// What `zcash_client_backend` hands over: change without an OVK, and the
/// recipient's address on the payment.
#[test]
fn test_zcash_client_backend_view() {
    let wallet = Wallet::testnet();
    for view in [View::Bare, View::Full] {
        let tx = Tx::pay(
            vec![
                Output {
                    user_address: Some("u1recipient".into()),
                    ..Output::payment(600_000)
                },
                Output {
                    ovk: None,
                    ..Output::change(390_000)
                },
            ],
            Vec::new(),
            10_000,
        );
        let bytes = build(&wallet, &Tx { view, ..tx }).bytes;
        assert_signatures_apply(&bytes, &sign(&wallet, &bytes).unwrap());
    }
}

#[test]
fn test_notes() {
    let wallet = Wallet::testnet();
    for count in 1..=8u64 {
        let notes: Vec<u64> = (0..count).map(|i| 100_000 + i * 10_000).collect();
        let fee = zip317_fee(count as usize);
        let tx = Tx {
            notes: notes.clone(),
            ..Tx::pay(
                vec![Output::payment(notes.iter().sum::<u64>() - fee)],
                Vec::new(),
                fee,
            )
        };
        let built = build(&wallet, &tx);
        let signatures = sign(&wallet, &built.bytes).unwrap();
        let mut spends = built.spends.clone();
        spends.sort();
        let indices: Vec<usize> = signatures
            .records()
            .iter()
            .map(|r| usize::from(r.action_index))
            .collect();
        assert_eq!(indices, spends);
        assert_signatures_apply(&built.bytes, &signatures);
    }
}

/// Fields outside the sighash: the signatures over one encoding apply to the
/// other.
#[test]
fn test_equivalent_encodings() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let edits: [Edit; 3] = [
        |v| v["ironwood"]["anchor"] = json!(vec![0u8; 32]),
        |v| v["global"]["fallback_lock_time"] = Value::Null,
        |v| v["ironwood"]["bsk"] = json!(vec![9u8; 32]),
    ];
    for edit in edits {
        let signatures = sign(&wallet, &mutate(&bytes, edit)).unwrap();
        assert_signatures_apply(&bytes, &signatures);
    }
}

#[test]
fn test_progress() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::two_notes()).bytes;
    let mut session = wallet.session();
    wallet.begin(&mut session, bytes.len()).unwrap();
    let mut calls = 0;
    let mut rest = &bytes[..];
    let mut review = None;
    while !rest.is_empty() {
        let (consumed, event) = session.feed(rest, &wallet.fvk, &mut || calls += 1).unwrap();
        rest = &rest[consumed..];
        if let Event::Review(reviewed) = event {
            review = Some(reviewed);
        }
    }
    assert!(calls > 0);
    let review = review.unwrap();
    let token = review.token();
    session.approve(token).unwrap();
    let mut calls = 0;
    let signatures = session
        .sign(token, &wallet.ask, &mut || calls += 1)
        .unwrap();
    assert_eq!(calls, signatures.records().len());
}

fn nonces(signatures: &Signatures) -> Vec<[u8; 32]> {
    // `R`, the nonce commitment: the half of a signature that depends on the
    // nonce alone.
    signatures
        .records()
        .iter()
        .map(|record| record.signature[..32].try_into().unwrap())
        .collect()
}

fn sign_using<R: RngCore + CryptoRng>(wallet: &Wallet, rng: R, bytes: &[u8]) -> Signatures {
    let session = Session::new(wallet.policy(), rng).unwrap();
    let signatures = sign_with(wallet, session, bytes).unwrap();
    assert_signatures_apply(bytes, &signatures);
    signatures
}

#[test]
fn test_rng_determines_signatures() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let signatures = |seed| sign_using(&wallet, ChaCha20Rng::from_seed([seed; 32]), &bytes);
    assert_eq!(signatures(1).records(), signatures(1).records());
    assert_ne!(nonces(&signatures(1)), nonces(&signatures(2)));
}

/// A failed entropy source: every draw is zeros.
struct DeadRng;

impl RngCore for DeadRng {
    fn next_u32(&mut self) -> u32 {
        0
    }
    fn next_u64(&mut self) -> u64 {
        0
    }
    fn fill_bytes(&mut self, destination: &mut [u8]) {
        destination.fill(0);
    }
    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), rand_core::Error> {
        destination.fill(0);
        Ok(())
    }
}

impl CryptoRng for DeadRng {}

/// Without entropy, a hedged nonce is still unique to the device secret and
/// the transaction, so no signature reveals the key.
#[test]
fn test_hedged_nonces_without_entropy() {
    let wallet = Wallet::testnet();
    let first = build(&wallet, &Tx::simple()).bytes;
    let second = build(
        &wallet,
        &Tx {
            seed: 1,
            ..Tx::simple()
        },
    )
    .bytes;
    let sign = |secret: &[u8], bytes| sign_using(&wallet, Hedged::new(DeadRng, secret), bytes);

    // The same secret and transaction sign alike.
    assert_eq!(
        sign(b"secret A", &first).records(),
        sign(b"secret A", &first).records()
    );
    // Another transaction, or another secret, draws another nonce.
    assert_ne!(
        nonces(&sign(b"secret A", &first)),
        nonces(&sign(b"secret A", &second))
    );
    assert_ne!(
        nonces(&sign(b"secret A", &first)),
        nonces(&sign(b"secret B", &first))
    );
}
