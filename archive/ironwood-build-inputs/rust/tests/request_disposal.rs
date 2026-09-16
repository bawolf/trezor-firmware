//! Inspect actual private ownership; no new device/debug API is exposed.
use super::*;
use std::sync::atomic::Ordering::SeqCst;
use std::vec;

#[path = "support/platform_stubs.rs"]
mod platform_stubs;

const PCZT: &[u8] = include_bytes!("hd-signer-view.pczt");

fn begin() -> Snapshot {
    let mut snapshot = Snapshot::default();
    assert_eq!(
        unsafe {
            ironwood_test_begin(
                (0u8..32).collect::<std::vec::Vec<_>>().as_ptr(),
                32,
                PCZT.as_ptr(),
                PCZT.len(),
                &mut snapshot,
            )
        },
        OK
    );
    assert!(BRIDGE.lock().pending.is_some());
    snapshot
}

fn assert_disposed() {
    assert!(BRIDGE.lock().pending.is_none());
    assert_eq!(ironwood_test_response_capacity(), 0);
}

#[test]
fn whole_request_is_disposed_on_terminal_paths() {
    platform_stubs::SEED.store(42, SeqCst);
    ironwood_test_cancel();
    ironwood_test_cancel();
    assert_disposed();
    let mut output = vec![0; RESPONSE_CAPACITY];
    let mut written = usize::MAX;

    let cancelled = begin();
    ironwood_test_cancel();
    assert_disposed();
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                cancelled.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        REJECTED
    );
    assert_eq!(written, 0);

    // Repeating entropy must not recreate the cancelled request's authority.
    let repeated_seed = begin();
    assert_ne!(cancelled.token, repeated_seed.token);
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                cancelled.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        REJECTED
    );
    assert_eq!(written, 0);
    assert_disposed();

    let replaced = begin();
    let replacement = begin();
    assert_ne!(replaced.token, replacement.token);
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                replaced.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        REJECTED
    );
    assert_disposed();

    // Invalid replacement arguments and invalid PCZT both discard old authority.
    begin();
    let mut snapshot = Snapshot::default();
    assert_eq!(
        unsafe {
            ironwood_test_begin(
                (0u8..32).collect::<std::vec::Vec<_>>().as_ptr(),
                32,
                ptr::null(),
                PCZT.len(),
                &mut snapshot,
            )
        },
        REJECTED
    );
    assert_disposed();
    begin();
    let malformed = [0_u8];
    assert_eq!(
        unsafe {
            ironwood_test_begin(
                (0u8..32).collect::<std::vec::Vec<_>>().as_ptr(),
                32,
                malformed.as_ptr(),
                malformed.len(),
                &mut snapshot,
            )
        },
        REJECTED
    );
    assert_disposed();

    let invalid_output = begin();
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                invalid_output.token.as_ptr(),
                ptr::null_mut(),
                0,
                &mut written,
            )
        },
        REJECTED
    );
    assert_disposed();
    let invalid_written = begin();
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                invalid_written.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                ptr::null_mut(),
            )
        },
        REJECTED
    );
    assert_disposed();
    let too_small = begin();
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                too_small.token.as_ptr(),
                output.as_mut_ptr(),
                0,
                &mut written,
            )
        },
        OUTPUT_CAPACITY
    );
    assert_eq!(written, 0);
    assert_disposed();

    // Private fault injection: approval failure must also drop the outer owner.
    // This is not a claim that a host can mutate the engine or that abort unwinds.
    let invalid_engine = begin();
    BRIDGE.lock().pending.as_mut().unwrap().engine.cancel();
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                invalid_engine.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        REJECTED
    );
    assert_disposed();

    platform_stubs::SEED.store(43, SeqCst);
    let positive = begin();
    let expected_sighash = *BRIDGE.lock().pending.as_ref().unwrap().review.sighash();
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                positive.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        OK
    );
    assert_eq!(written, PCZT.len() + 64);
    assert_disposed();
    let signed = pczt::Pczt::parse(&output[..written]).unwrap();
    // Keep full-signer features outside this constrained build. The independent
    // pair oracle separately checks this saved output's digest and all fields.
    let evidence = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../unit-evidence");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("positive.input"), PCZT).unwrap();
    std::fs::write(evidence.join("positive.signed"), &output[..written]).unwrap();
    let mut verified = 0;
    pczt::roles::verifier::Verifier::new(signed)
        .with_ironwood(
            |bundle| -> Result<(), pczt::roles::verifier::OrchardError<()>> {
                for action in bundle.actions() {
                    action
                        .spend()
                        .rk()
                        .verify(
                            &expected_sighash,
                            action.spend().spend_auth_sig().as_ref().unwrap(),
                        )
                        .unwrap();
                    verified += 1;
                }
                Ok(())
            },
        )
        .unwrap();
    assert_eq!(verified, 2);
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                positive.token.as_ptr(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        REJECTED
    );
    assert_eq!(written, 0);
    assert_disposed();
    assert_eq!(
        platform_stubs::ENTROPY_CALLS.load(SeqCst),
        platform_stubs::CLEAR_CALLS.load(SeqCst)
    );

    // Returning to the first entropy seed still must not revive an old token.
    platform_stubs::SEED.store(42, SeqCst);
    let returned_seed = begin();
    assert_ne!(returned_seed.token, cancelled.token);
    assert_ne!(returned_seed.token, repeated_seed.token);
    // Exhaustion is terminal and cannot wrap back to an old request counter.
    BRIDGE.lock().next_request = u64::MAX - 1;
    let last = begin();
    assert_ne!(last.token, returned_seed.token);
    assert_eq!(BRIDGE.lock().next_request, u64::MAX);
    let calls = platform_stubs::ENTROPY_CALLS.load(SeqCst);
    assert_eq!(
        unsafe {
            ironwood_test_begin(
                (0u8..32).collect::<std::vec::Vec<_>>().as_ptr(),
                32,
                PCZT.as_ptr(),
                PCZT.len(),
                &mut snapshot,
            )
        },
        REJECTED
    );
    assert_disposed();
    assert_eq!(BRIDGE.lock().next_request, u64::MAX);
    assert_eq!(platform_stubs::ENTROPY_CALLS.load(SeqCst), calls);
}

#[test]
fn stateless_rng_handles_unaligned_slices_and_clears_each_block() {
    assert_eq!(size_of::<StrongRng>(), 0);
    platform_stubs::SEED.store(42, SeqCst);
    #[repr(align(4))]
    struct Guarded([u8; 100]);
    let mut rng = StrongRng;
    for length in [0_usize, 1, 3, 4, 8, 31, 32, 33, 80] {
        for fallible in [false, true] {
            let calls = platform_stubs::ENTROPY_CALLS.load(SeqCst);
            let clears = platform_stubs::CLEAR_CALLS.load(SeqCst);
            let mut bytes = Guarded([0xA5; 100]);
            let destination = &mut bytes.0[1..1 + length];
            assert_eq!(destination.as_ptr() as usize % 4, 1);
            if fallible {
                rng.try_fill_bytes(destination).unwrap();
            } else {
                rng.fill_bytes(destination);
            }
            assert!(bytes.0[1..1 + length].iter().all(|&byte| byte == 42));
            assert_eq!(bytes.0[0], 0xA5);
            assert!(bytes.0[1 + length..].iter().all(|&byte| byte == 0xA5));
            assert_eq!(
                platform_stubs::ENTROPY_CALLS.load(SeqCst) - calls,
                length.div_ceil(32)
            );
            assert_eq!(
                platform_stubs::CLEAR_CALLS.load(SeqCst) - clears,
                length.div_ceil(32)
            );
        }
    }
    let calls = platform_stubs::ENTROPY_CALLS.load(SeqCst);
    assert_eq!(rng.next_u32(), u32::from_le_bytes([42; 4]));
    assert_eq!(rng.next_u64(), u64::from_le_bytes([42; 8]));
    assert_eq!(platform_stubs::ENTROPY_CALLS.load(SeqCst) - calls, 2);
    assert_eq!(
        platform_stubs::ENTROPY_CALLS.load(SeqCst),
        platform_stubs::CLEAR_CALLS.load(SeqCst)
    );
}

#[test]
fn explicit_counter_separates_recreated_engines_with_repeating_entropy() {
    platform_stubs::SEED.store(42, SeqCst);
    let seed: [u8; 32] = core::array::from_fn(|index| index as u8);
    let spending = SpendingKey::from_zip32_seed(&seed, 1, 9u32.try_into().unwrap()).unwrap();
    let policy = || Policy::regtest(10_000_000, 100_000).unwrap();
    let fvk = || FullViewingKey::from(&spending);
    let mut first = Engine::with_rng(policy(), fvk(), StrongRng).unwrap();
    let mut repeat = Engine::with_rng(policy(), fvk(), StrongRng).unwrap();
    let first_token = first.begin(PCZT).unwrap().token().clone();
    // Executed regression witness: a literal stateless swap without a counter repeats authority.
    assert_eq!(first_token, *repeat.begin(PCZT).unwrap().token());
    let mut next = Engine::with_rng_and_counter(policy(), fvk(), StrongRng, 1).unwrap();
    assert_ne!(first_token, *next.begin(PCZT).unwrap().token());
    let mut exhausted = Engine::with_rng_and_counter(policy(), fvk(), StrongRng, u64::MAX).unwrap();
    assert_eq!(exhausted.begin(PCZT).err().unwrap().0, "session exhausted");
}
