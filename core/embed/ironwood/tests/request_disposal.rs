#![cfg(feature = "test")]

mod common;

use common::{TestEngineExt, assert_error, engine, fixture, keys};
use ironwood::ErrorCode;
use orchard::keys::{SpendAuthorizingKey, SpendingKey};

fn assert_empty(engine: &ironwood::Engine<rand_chacha::ChaCha20Rng>) {
    assert!(!engine.test_has_pending_request());
    assert!(engine.test_request_binding_is_zero());
}

#[test]
fn cancel_is_idempotent_and_clears_typed_binding_storage() {
    let mut e = engine();
    let review = e.begin_test(&fixture()).unwrap();
    assert!(e.test_has_pending_request());
    assert!(!e.test_request_binding_is_zero());

    e.cancel();
    assert_empty(&e);
    e.cancel();
    assert_empty(&e);
    assert_error(
        e.sign(review.token(), &keys().1).err().unwrap(),
        ErrorCode::State,
    );
}

#[test]
fn rejected_replacement_clears_previous_request_binding() {
    let mut e = engine();
    let review = e.begin_test(&fixture()).unwrap();
    e.approve(review.token()).unwrap();
    assert!(e.begin_test(b"PCZT").is_err());
    assert_empty(&e);
    assert_error(
        e.sign(review.token(), &keys().1).err().unwrap(),
        ErrorCode::State,
    );
}

#[test]
fn every_sign_attempt_consumes_and_clears_typed_binding_storage() {
    let mut successful = engine();
    let review = successful.begin_test(&fixture()).unwrap();
    successful.approve(review.token()).unwrap();
    successful.sign(review.token(), &keys().1).unwrap();
    assert_empty(&successful);

    let mut wrong_key = engine();
    let review = wrong_key.begin_test(&fixture()).unwrap();
    wrong_key.approve(review.token()).unwrap();
    let ask = SpendAuthorizingKey::from(&SpendingKey::from_bytes([2; 32]).unwrap());
    assert_error(
        wrong_key.sign(review.token(), &ask).err().unwrap(),
        ErrorCode::Signing,
    );
    assert_empty(&wrong_key);

    let mut owner = engine();
    let owner_review = owner.begin_test(&fixture()).unwrap();
    let mut other = engine();
    let other_review = other.begin_test(&fixture()).unwrap();
    assert_error(
        owner.approve(other_review.token()).err().unwrap(),
        ErrorCode::State,
    );
    assert_empty(&owner);
    assert_error(
        owner.sign(owner_review.token(), &keys().1).err().unwrap(),
        ErrorCode::State,
    );
}

// These tests intentionally cover only crate-owned typed binding arrays. The
// upstream Pczt allocation, its embedded FVK material, generic RNG state, and
// ASK internals do not implement typed zeroization and remain a Boundary 2
// blocker.
