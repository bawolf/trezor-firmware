use super::common::{HEIGHT, build_inputs, fixture, json, keys};
use ironwood_approval::{Engine, Error, Policy};
use pczt::{
    Pczt,
    roles::signer::{Signer, SpendAuthSignature},
};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use std::{cell::Cell, num::NonZeroU32, panic::AssertUnwindSafe, rc::Rc};

// These fault injectors are test-only. They deliberately violate availability,
// never supply production randomness, and must not be used by a device adapter.
struct SessionUnavailable;
impl CryptoRng for SessionUnavailable {}
impl RngCore for SessionUnavailable {
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

#[test]
fn unavailable_session_entropy_prevents_engine_creation() {
    assert!(matches!(
        Engine::with_rng(
            Policy::regtest(HEIGHT, 100_000).unwrap(),
            keys().0,
            SessionUnavailable
        ),
        Err(Error("session entropy unavailable"))
    ));
}

#[test]
fn seeded_synthetic_signing_is_reproducible_and_preserves_metadata() {
    let bytes = fixture();
    let mut previous = None;
    for _ in 0..2 {
        let mut engine = Engine::with_rng(
            Policy::regtest(HEIGHT, 100_000).unwrap(),
            keys().0,
            ChaCha20Rng::from_seed([42; 32]),
        )
        .unwrap();
        let review = engine.begin(&bytes).unwrap();
        engine.approve(review.token()).unwrap();
        let signed = engine.sign(review.token(), &keys().1).unwrap();
        assert_eq!(&signed.sighash, review.sighash());
        let mut oracle = Signer::new(Pczt::parse(&bytes).unwrap()).unwrap();
        for signature in &signed.signatures {
            oracle
                .apply_orchard_spend_auth_signature(&SpendAuthSignature::from_parts(
                    signature.value_pool(),
                    signature.action_index(),
                    *signature.signature(),
                ))
                .unwrap();
        }
        assert_eq!(oracle.shielded_sighash(), signed.sighash);
        let original = json(&bytes);
        let signed_bytes = signed.pczt.serialize().unwrap();
        let mut restored = json(&signed_bytes);
        for signature in &signed.signatures {
            let i = signature.action_index();
            restored["ironwood"]["actions"][i]["spend"]["spend_auth_sig"] =
                original["ironwood"]["actions"][i]["spend"]["spend_auth_sig"].clone();
        }
        assert_eq!(restored, original, "signing changed non-signature metadata");
        let result = (
            signed.token,
            signed.sighash,
            signed.signatures,
            signed_bytes,
        );
        if let Some(previous) = &previous {
            assert_eq!(&result, previous);
        }
        previous = Some(result);
        assert!(engine.sign(review.token(), &keys().1).is_err());
    }
}

#[test]
fn independently_seeded_sessions_reject_each_others_tokens() {
    let bytes = fixture();
    let mut first = Engine::with_rng(
        Policy::regtest(HEIGHT, 100_000).unwrap(),
        keys().0,
        ChaCha20Rng::from_seed([43; 32]),
    )
    .unwrap();
    let mut second = Engine::with_rng(
        Policy::regtest(HEIGHT, 100_000).unwrap(),
        keys().0,
        ChaCha20Rng::from_seed([44; 32]),
    )
    .unwrap();
    let a = first.begin(&bytes).unwrap();
    let b = second.begin(&bytes).unwrap();
    assert_ne!(a.token(), b.token());
    assert_eq!(a.sighash(), b.sighash());
    assert!(second.approve(a.token()).is_err());
    assert!(second.approve(b.token()).is_err());
    assert!(second.sign(b.token(), &keys().1).is_err());
}

struct PanicOnSecondSignature {
    rng: ChaCha20Rng,
    signing_calls: Rc<Cell<usize>>,
}
impl CryptoRng for PanicOnSecondSignature {}
impl RngCore for PanicOnSecondSignature {
    fn next_u32(&mut self) -> u32 {
        unreachable!()
    }
    fn next_u64(&mut self) -> u64 {
        unreachable!()
    }
    fn try_fill_bytes(&mut self, bytes: &mut [u8]) -> Result<(), rand_core::Error> {
        self.rng.try_fill_bytes(bytes) // Session creation succeeds.
    }
    fn fill_bytes(&mut self, bytes: &mut [u8]) {
        let count = self.signing_calls.get() + 1;
        self.signing_calls.set(count);
        assert!(count < 2, "synthetic signature entropy failure");
        self.rng.fill_bytes(bytes);
    }
}

#[test]
fn entropy_panic_after_first_signature_consumes_consent_without_response() {
    let signing_calls = Rc::new(Cell::new(0));
    let mut engine = Engine::with_rng(
        Policy::regtest(HEIGHT, 100_000).unwrap(),
        keys().0,
        PanicOnSecondSignature {
            rng: ChaCha20Rng::from_seed([45; 32]),
            signing_calls: signing_calls.clone(),
        },
    )
    .unwrap();
    let review = engine.begin(&build_inputs(&[100_000, 100_000])).unwrap();
    engine.approve(review.token()).unwrap();
    // Upstream RedDSA uses infallible fill_bytes. A trusted RNG must fail-stop
    // on entropy loss; this host-only unwind test exercises the interruption.
    let result =
        std::panic::catch_unwind(AssertUnwindSafe(|| engine.sign(review.token(), &keys().1)));
    assert!(result.is_err());
    assert_eq!(signing_calls.get(), 2);
    assert!(engine.sign(review.token(), &keys().1).is_err());
    assert!(engine.approve(review.token()).is_err());
}
