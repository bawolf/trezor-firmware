#![cfg(feature = "test")]
//! The spend-authorization nonce must not rest on the TRNG alone.
//!
//! Every test here runs the session with an RNG that returns nothing but
//! zeros -- the stuck, biased or fault-injected TRNG the hedge exists for --
//! and asserts that signing still behaves like a deterministic signature
//! scheme keyed by the wallet secret: signatures verify, and no two of them
//! share a nonce unless the transaction and the key are also the same.
//!
//! Without [`Hedged`] each of these produces the same `random_bytes`, so
//! reddsa's `T = HStar(random_bytes ‖ pk ‖ msg)` is attacker-known and one
//! signature reveals `rsk`, hence `ask`. See `src/hedge.rs`.

mod common;

use common::*;
use ironwood::{Hedged, Network, Review, Session, SignatureRecord};
use pczt::Pczt;
use pczt::roles::signer::{Signer, SpendAuthSignature};
use rand_core::{CryptoRng, RngCore};
use zcash_protocol::memo::MemoBytes;

/// The worst case the hedge is designed for: an entropy source that has
/// failed completely and says so to nobody.
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
        self.fill_bytes(destination);
        Ok(())
    }
}

impl CryptoRng for DeadRng {}

fn hedged(seed: &[u8]) -> Session<Hedged<DeadRng>> {
    Session::with_rng(
        policy(Network::Testnet, 100_000),
        Hedged::from_seed(DeadRng, seed),
    )
    .unwrap()
}

/// Feeds the whole PCZT, approves, signs. Returns the records and the sighash
/// the session derived, so the caller can apply them the way a wallet does.
fn sign_with<R: RngCore + CryptoRng>(
    session: &mut Session<R>,
    bytes: &[u8],
) -> (Vec<SignatureRecord>, [u8; 32]) {
    let (fvk, ask) = keys();
    session.begin(bytes.len(), &fvk, &SEED_FINGERPRINT).unwrap();
    let mut rest = bytes;
    let mut review: Option<Review> = None;
    while !rest.is_empty() {
        let (consumed, event) = session.feed(rest, &fvk).unwrap();
        rest = &rest[consumed..];
        if let ironwood::Event::Review(found) = event {
            review = Some(found);
        }
    }
    let review = review.expect("the session reviewed the bundle");
    let sighash = *review.sighash();
    session.approve(review.token()).unwrap();
    let signatures = session.sign(review.token(), &ask).unwrap();
    (signatures.records().to_vec(), sighash)
}

/// A wallet applying the records re-verifies each one against its action's
/// `rk` and its own sighash before storing it, so this is the verification
/// the device's signatures actually have to pass.
fn verify_as_a_wallet(bytes: &[u8], records: &[SignatureRecord], sighash: &[u8; 32]) {
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

fn only_signature(records: &[SignatureRecord]) -> [u8; 64] {
    assert_eq!(records.len(), 1, "the fixture has one real spend");
    records[0].signature
}

/// A dead TRNG must not stop the device signing, and what it produces must
/// still be a valid RedPallas signature over this transaction's sighash.
#[test]
fn a_dead_entropy_source_still_yields_a_verifying_signature() {
    let bytes = fixture();
    let (records, sighash) = sign_with(&mut hedged(b"wallet seed A"), &bytes);
    verify_as_a_wallet(&bytes, &records, &sighash);
}

/// Two transactions, one wallet, no entropy: the signatures must differ.
/// reddsa's `HStar` binds `msg`, so this would hold without the hedge too; it
/// is here because it is the first thing a reader checks and its absence
/// would mean something far worse than a weak nonce.
#[test]
fn two_transactions_do_not_share_a_nonce_without_entropy() {
    let first = fixture();
    let second = build(40_000, 20_000, MemoBytes::empty(), false);
    assert_ne!(first, second);

    let (first_records, first_sighash) = sign_with(&mut hedged(b"wallet seed A"), &first);
    let (second_records, second_sighash) = sign_with(&mut hedged(b"wallet seed A"), &second);
    assert_ne!(first_sighash, second_sighash);

    verify_as_a_wallet(&first, &first_records, &first_sighash);
    verify_as_a_wallet(&second, &second_records, &second_sighash);
    assert_ne!(
        only_signature(&first_records),
        only_signature(&second_records)
    );
}

/// The test with teeth. Two wallets sign the *same* transaction with the
/// *same* spend-authorizing key and no entropy; only the hedge secret differs.
/// Unhedged, both would draw `random_bytes = 0` and emit the same 64 bytes.
#[test]
fn two_wallets_do_not_share_a_nonce_without_entropy() {
    let bytes = fixture();
    let (mine, sighash) = sign_with(&mut hedged(b"wallet seed A"), &bytes);
    let (theirs, other_sighash) = sign_with(&mut hedged(b"wallet seed B"), &bytes);
    assert_eq!(sighash, other_sighash, "same transaction, same message");

    verify_as_a_wallet(&bytes, &mine, &sighash);
    verify_as_a_wallet(&bytes, &theirs, &other_sighash);
    assert_ne!(only_signature(&mine), only_signature(&theirs));
}

/// The hedge is a wrapper, not a replacement: with the same secret, the same
/// transaction and a dead TRNG the device is deterministic. Stated as a test
/// so that a later change to `Hedged` that quietly reintroduces a
/// per-signature dependence on entropy alone shows up here rather than in a
/// review.
#[test]
fn the_same_wallet_and_transaction_without_entropy_is_deterministic() {
    let bytes = fixture();
    let (once, _) = sign_with(&mut hedged(b"wallet seed A"), &bytes);
    let (twice, _) = sign_with(&mut hedged(b"wallet seed A"), &bytes);
    assert_eq!(only_signature(&once), only_signature(&twice));
}

/// `alpha`, and so `rk`, is host-supplied inside the PCZT: the device draws
/// no randomness for the re-randomization at all, and its only check is
/// `rk == ak.randomize(alpha)` (`session.rs`, `verify_rk`). So the question
/// "does the re-randomization share the nonce's entropy source" has the
/// answer "it has no entropy source", and a signature under a host-chosen
/// `alpha` still hands over `ask` if the nonce leaks -- which is the whole
/// reason the nonce is hedged. Pinned as the shape of the argument: the same
/// host bytes give the same `rk` and the same sighash on two independent
/// sessions, so nothing about `alpha` varies with the device.
#[test]
fn the_rerandomizer_comes_from_the_host_not_the_device() {
    let bytes = fixture();
    let (first, first_sighash) = sign_with(&mut hedged(b"wallet seed A"), &bytes);
    let (second, second_sighash) = sign_with(&mut hedged(b"wallet seed B"), &bytes);
    assert_eq!(first_sighash, second_sighash);
    assert_eq!(first[0].action_index, second[0].action_index);
    // Two different wallets, and the wallet-side verification -- which checks
    // each signature against the action's own `rk` -- accepts both, so `rk`
    // did not move between the two runs.
    verify_as_a_wallet(&bytes, &first, &first_sighash);
    verify_as_a_wallet(&bytes, &second, &second_sighash);
}
