//! The request state machine: `begin`, `feed`, `approve`, `sign`, and the
//! token that binds an approval to one reviewed request of one session.

use std::num::NonZeroU32;
use std::panic::{AssertUnwindSafe, catch_unwind};

use orchard::keys::{FullViewingKey, SpendAuthorizingKey, SpendingKey};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use serde_json::{Value, json};
use zcash_signer::Error::{Entropy, Malformed, Signing, State};
use zcash_signer::{Policy, Review, Session};
use zcash_signer_tests::*;

fn reviewed(wallet: &Wallet, session: &mut Session<ChaCha20Rng>, bytes: &[u8]) -> Review {
    stream(session, wallet, bytes, bytes.len(), 1024)
        .unwrap()
        .review
        .unwrap()
}

fn other_ask() -> SpendAuthorizingKey {
    SpendAuthorizingKey::from(&SpendingKey::from_bytes([2; 32]).unwrap())
}

#[test]
fn test_sign_requires_approval() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let mut session = wallet.session();
    let review = reviewed(&wallet, &mut session, &bytes);
    let token = review.token();
    assert_eq!(
        session.sign(token, &wallet.ask, &mut || {}).err(),
        Some(State)
    );
    // The attempt consumed the request.
    assert_eq!(session.approve(token), Err(State));
}

#[test]
fn test_approval_is_single_use() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let mut session = wallet.session();
    let review = reviewed(&wallet, &mut session, &bytes);
    let token = review.token();
    session.approve(token).unwrap();
    assert_eq!(session.approve(token), Err(State));
    assert_eq!(
        session.sign(token, &wallet.ask, &mut || {}).err(),
        Some(State)
    );

    let review = reviewed(&wallet, &mut session, &bytes);
    let token = review.token();
    session.approve(token).unwrap();
    session.sign(token, &wallet.ask, &mut || {}).unwrap();
    assert_eq!(
        session.sign(token, &wallet.ask, &mut || {}).err(),
        Some(State)
    );
    assert_eq!(session.approve(token), Err(State));
}

/// Another account's key signs nothing, and consumes the approval.
#[test]
fn test_wrong_key() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::two_notes()).bytes;
    let mut session = wallet.session();
    let review = reviewed(&wallet, &mut session, &bytes);
    session.approve(review.token()).unwrap();
    assert_eq!(
        session.sign(review.token(), &other_ask(), &mut || {}).err(),
        Some(Signing)
    );
    assert_eq!(
        session.sign(review.token(), &wallet.ask, &mut || {}).err(),
        Some(State)
    );
}

/// A new request drops the one being streamed or approved, even if it fails
/// to start.
#[test]
fn test_begin_discards_request() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let mut session = wallet.session();
    stream(
        &mut session,
        &wallet,
        &bytes[..bytes.len() / 2],
        bytes.len(),
        64,
    )
    .unwrap();
    let first = reviewed(&wallet, &mut session, &bytes);
    session.approve(first.token()).unwrap();
    let second = reviewed(&wallet, &mut session, &bytes);
    assert_ne!(first.token(), second.token());
    assert_eq!(session.approve(first.token()), Err(State));

    let review = reviewed(&wallet, &mut session, &bytes);
    session.approve(review.token()).unwrap();
    assert_eq!(wallet.begin(&mut session, 4), Err(Malformed));
    assert_eq!(
        session.sign(review.token(), &wallet.ask, &mut || {}).err(),
        Some(State)
    );
}

/// A token from another session is refused, and drops the pending request.
#[test]
fn test_foreign_token() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let mut first = wallet.session_with(wallet.policy(), 1);
    let mut second = wallet.session_with(wallet.policy(), 2);
    let a = reviewed(&wallet, &mut first, &bytes);
    let b = reviewed(&wallet, &mut second, &bytes);
    assert_ne!(a.token(), b.token());
    assert_eq!(second.approve(a.token()), Err(State));
    assert_eq!(second.approve(b.token()), Err(State));
    assert_eq!(
        second.sign(b.token(), &wallet.ask, &mut || {}).err(),
        Some(State)
    );
}

/// Sessions with the same RNG draw the same session id, so their tokens
/// differ only by what they bind: the policy and the bytes, not the chunking.
#[test]
fn test_token_binding() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let token = |policy: Policy, bytes: &[u8], chunk| {
        let mut session = wallet.session_with(policy, 0);
        stream(&mut session, &wallet, bytes, bytes.len(), chunk)
            .unwrap()
            .review
            .unwrap()
    };
    let base = token(wallet.policy(), &bytes, 1024);
    for chunk in CHUNKINGS {
        assert_eq!(token(wallet.policy(), &bytes, chunk).token(), base.token());
    }
    let policy = |account, height, max_fee| {
        Policy::new(wallet.network, account, height, max_fee, EXPIRY_WINDOW).unwrap()
    };
    for other in [
        policy(wallet.account + 1, HEIGHT, MAX_FEE),
        policy(wallet.account, HEIGHT - 1, MAX_FEE),
        policy(wallet.account, HEIGHT, MAX_FEE - 1),
    ] {
        assert_ne!(token(other, &bytes, 1024).token(), base.token());
    }
    // Other bytes with the same sighash: an anchor, and the zero lock time
    // left implicit.
    for altered in [
        mutate(&bytes, |v| v["ironwood"]["anchor"] = json!(vec![0u8; 32])),
        mutate(&bytes, |v| v["global"]["fallback_lock_time"] = Value::Null),
    ] {
        assert_eq!(sighash(&altered), sighash(&bytes));
        assert_ne!(token(wallet.policy(), &altered, 1024).token(), base.token());
    }
}

#[test]
fn test_calls_out_of_order() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let foreign = reviewed(
        &wallet,
        &mut wallet.session_with(wallet.policy(), 9),
        &bytes,
    );
    let half = &bytes[..bytes.len() / 2];
    let feed = |session: &mut Session<ChaCha20Rng>, bytes: &[u8]| {
        session.feed(bytes, &wallet.fvk, &mut || {}).map(|_| ())
    };

    let mut session = wallet.session();
    assert_eq!(feed(&mut session, &bytes), Err(State), "feed before begin");
    reviewed(&wallet, &mut session, &bytes);
    assert_eq!(
        feed(&mut session, &[0]),
        Err(State),
        "feed after the review"
    );

    // Approving or signing mid-stream ends the stream.
    type Call = fn(&mut Session<ChaCha20Rng>, &Review, &Wallet) -> zcash_signer::Result<()>;
    let calls: [(&str, Call); 2] = [
        ("approve", |session, review, _| {
            session.approve(review.token())
        }),
        ("sign", |session, review, wallet| {
            session
                .sign(review.token(), &wallet.ask, &mut || {})
                .map(|_| ())
        }),
    ];
    for (name, call) in calls {
        stream(&mut session, &wallet, half, bytes.len(), 1024).unwrap();
        assert_eq!(call(&mut session, &foreign, &wallet), Err(State), "{name}");
        assert_eq!(
            feed(&mut session, &bytes[half.len()..]),
            Err(State),
            "{name}"
        );
    }

    // More bytes than declared.
    wallet.begin(&mut session, bytes.len()).unwrap();
    let extra = [&bytes[..], &[0]].concat();
    assert_eq!(feed(&mut session, &extra), Err(State));
    assert_eq!(feed(&mut session, &[]), Err(State), "an error resets");
}

/// Every feed must lend the key given to `begin`.
#[test]
fn test_feed_with_other_key() {
    let wallet = Wallet::testnet();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([3; 32]).unwrap());
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let half = bytes.len() / 2;
    let mut session = wallet.session();
    stream(&mut session, &wallet, &bytes[..half], bytes.len(), 64).unwrap();
    assert_eq!(
        session.feed(&bytes[half..], &other, &mut || {}).err(),
        Some(State)
    );
    assert_eq!(
        session.feed(&bytes[half..], &wallet.fvk, &mut || {}).err(),
        Some(State)
    );
    reviewed(&wallet, &mut session, &bytes);

    // A session begun for another key refuses the account's real spend.
    let stranger = Wallet {
        fvk: other,
        ..Wallet::testnet()
    };
    assert_outcome(&stranger, "other key", &bytes, Err(Malformed));
}

struct FailingRng;

impl RngCore for FailingRng {
    fn next_u32(&mut self) -> u32 {
        unreachable!()
    }
    fn next_u64(&mut self) -> u64 {
        unreachable!()
    }
    fn fill_bytes(&mut self, _: &mut [u8]) {
        unreachable!()
    }
    fn try_fill_bytes(&mut self, _: &mut [u8]) -> Result<(), rand_core::Error> {
        Err(NonZeroU32::new(rand_core::Error::CUSTOM_START)
            .unwrap()
            .into())
    }
}

impl CryptoRng for FailingRng {}

#[test]
fn test_no_entropy() {
    let policy = Wallet::testnet().policy();
    assert_eq!(Session::new(policy, FailingRng).err(), Some(Entropy));
}

/// Panics when the second signature draws its nonce, and only then.
struct PanicsOnSecondNonce {
    inner: ChaCha20Rng,
    nonces: usize,
}

impl RngCore for PanicsOnSecondNonce {
    fn next_u32(&mut self) -> u32 {
        unreachable!()
    }
    fn next_u64(&mut self) -> u64 {
        unreachable!()
    }
    fn fill_bytes(&mut self, destination: &mut [u8]) {
        self.nonces += 1;
        assert_ne!(self.nonces, 2, "entropy failed");
        self.inner.fill_bytes(destination);
    }
    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), rand_core::Error> {
        self.inner.try_fill_bytes(destination)
    }
}

impl CryptoRng for PanicsOnSecondNonce {}

/// `sign` consumes the approval whether or not it succeeds, even if it
/// unwinds.
#[test]
fn test_panic_while_signing() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::two_notes()).bytes;
    let rng = PanicsOnSecondNonce {
        inner: ChaCha20Rng::from_seed([0; 32]),
        nonces: 0,
    };
    let mut session = Session::new(wallet.policy(), rng).unwrap();
    let review = stream(&mut session, &wallet, &bytes, bytes.len(), 1024)
        .unwrap()
        .review
        .unwrap();
    session.approve(review.token()).unwrap();
    let signed = catch_unwind(AssertUnwindSafe(|| {
        session.sign(review.token(), &wallet.ask, &mut || {})
    }));
    assert!(signed.is_err());
    let again = session.sign(review.token(), &wallet.ask, &mut || {});
    assert_eq!(again.err(), Some(State));
}
