use ironwood_approval::{Engine, OutputKind, Policy};
use ironwood_emulator_bridge::{
    OK, OUTPUT_CAPACITY, REJECTED, RESPONSE_CAPACITY, Snapshot, ironwood_test_begin,
    ironwood_test_cancel, ironwood_test_sign,
};
use orchard::keys::{FullViewingKey, SpendingKey};
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use serde_json::json;
use std::sync::atomic::Ordering::SeqCst;

#[path = "support/platform_stubs.rs"]
mod platform_stubs;

#[test]
fn c_boundary_preserves_review_and_consumes_consent() {
    assert_eq!(core::mem::size_of::<ironwood_emulator_bridge::Output>(), 56);
    assert_eq!(core::mem::size_of::<Snapshot>(), 528);
    assert_eq!(core::mem::offset_of!(Snapshot, total_input), 0);
    assert_eq!(core::mem::offset_of!(Snapshot, payments), 8);
    assert_eq!(core::mem::offset_of!(Snapshot, change), 16);
    assert_eq!(core::mem::offset_of!(Snapshot, fee), 24);
    assert_eq!(core::mem::offset_of!(Snapshot, branch), 32);
    assert_eq!(core::mem::offset_of!(Snapshot, expiry), 36);
    assert_eq!(core::mem::offset_of!(Snapshot, padding_outputs), 40);
    assert_eq!(core::mem::offset_of!(Snapshot, output_count), 44);
    assert_eq!(core::mem::offset_of!(Snapshot, token), 48);
    assert_eq!(core::mem::offset_of!(Snapshot, outputs), 80);
    assert_eq!(
        core::mem::offset_of!(ironwood_emulator_bridge::Output, value),
        0
    );
    assert_eq!(
        core::mem::offset_of!(ironwood_emulator_bridge::Output, action_index),
        8
    );
    assert_eq!(
        core::mem::offset_of!(ironwood_emulator_bridge::Output, kind),
        12
    );
    assert_eq!(
        core::mem::offset_of!(ironwood_emulator_bridge::Output, receiver),
        13
    );
    platform_stubs::SEED.store(42, SeqCst);
    let seed: [u8; 32] = core::array::from_fn(|index| index as u8);
    let fixture = include_bytes!("hd-signer-view.pczt");
    let begin = |bytes: &[u8], snapshot: &mut Snapshot| unsafe {
        ironwood_test_begin(
            seed.as_ptr(),
            seed.len(),
            bytes.as_ptr(),
            bytes.len(),
            snapshot,
        )
    };
    let mut snapshot = Snapshot::default();
    let mut output = vec![0; RESPONSE_CAPACITY];
    let mut written = usize::MAX;
    let sign = |token: &[u8; 32], output: &mut [u8], written: &mut usize| unsafe {
        ironwood_test_sign(token.as_ptr(), output.as_mut_ptr(), output.len(), written)
    };
    assert_eq!(sign(&[0; 32], &mut output, &mut written), REJECTED);
    assert_eq!(written, 0);
    assert_eq!(begin(fixture, &mut snapshot), OK);

    // Compare snapshot fields against the actual core's independent Review.
    let spending =
        SpendingKey::from_zip32_seed(&seed, 1, 9u32.try_into().expect("test account")).unwrap();
    let mut reference = Engine::with_rng(
        Policy::regtest(10_000_000, 100_000).unwrap(),
        FullViewingKey::from(&spending),
        ChaCha20Rng::from_seed([42; 32]),
    )
    .unwrap();
    let review = reference.begin(fixture).unwrap();
    let p = review.projection();
    // The bridge uses the platform RNG while this independent projection oracle
    // uses ChaCha; token unpredictability is covered by the bridge unit tests.
    assert_eq!(
        (
            snapshot.total_input,
            snapshot.payments,
            snapshot.change,
            snapshot.fee
        ),
        (p.total_input, p.payments, p.change, p.fee)
    );
    assert_eq!((snapshot.branch, snapshot.expiry), (p.branch, p.expiry));
    assert_eq!(snapshot.padding_outputs as usize, p.padding_outputs);
    assert_eq!(snapshot.output_count as usize, p.outputs.len());
    for (actual, expected) in snapshot.outputs.iter().zip(&p.outputs) {
        assert_eq!(actual.receiver, expected.receiver);
        assert_eq!(actual.value, expected.value);
        assert_eq!(actual.action_index as usize, expected.action_index);
        assert_eq!(
            actual.kind,
            if expected.kind == OutputKind::Payment {
                0
            } else {
                1
            }
        );
    }
    let original = snapshot.token;
    assert_eq!(begin(fixture, &mut snapshot), OK);
    assert_ne!(
        snapshot.token, original,
        "replacement must not restart the Engine"
    );
    assert_eq!(sign(&original, &mut output, &mut written), REJECTED);
    assert_eq!(
        sign(&snapshot.token, &mut output, &mut written),
        REJECTED,
        "wrong-token attempt must revoke the replacement too"
    );

    assert_eq!(begin(fixture, &mut snapshot), OK);
    let mut oversized_response = vec![0; RESPONSE_CAPACITY + 1];
    assert_eq!(
        sign(&snapshot.token, &mut oversized_response, &mut written),
        REJECTED
    );
    assert_eq!(written, 0);
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);

    assert_eq!(begin(fixture, &mut snapshot), OK);
    ironwood_test_cancel();
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);
    assert_eq!(begin(fixture, &mut snapshot), OK);
    assert_eq!(
        sign(&snapshot.token, &mut output[..1], &mut written),
        OUTPUT_CAPACITY
    );
    assert_eq!(written, 0);
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);

    assert_eq!(begin(fixture, &mut snapshot), OK);
    assert_eq!(
        unsafe {
            ironwood_test_begin(
                seed.as_ptr(),
                seed.len(),
                fixture.as_ptr(),
                fixture.len(),
                core::ptr::null_mut(),
            )
        },
        REJECTED
    );
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);
    assert_eq!(begin(fixture, &mut snapshot), OK);
    assert_eq!(
        unsafe {
            ironwood_test_sign(
                core::ptr::null(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        },
        REJECTED
    );
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);

    // Every failed replacement revokes the previous consent, including size and
    // pointer failures before Rust constructs an input slice.
    let malformed_limit = vec![0; ironwood_approval::wire::MAX_PCZT_BYTES];
    let oversized_input = vec![0; ironwood_approval::wire::MAX_PCZT_BYTES + 1];
    for invalid in [
        &[][..],
        &fixture[..fixture.len() - 1],
        &malformed_limit[..],
        &oversized_input[..],
    ] {
        assert_eq!(begin(fixture, &mut snapshot), OK);
        assert_eq!(begin(invalid, &mut snapshot), REJECTED);
        assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);
        assert_eq!(written, 0);
    }
    assert_eq!(begin(fixture, &mut snapshot), OK);
    assert_eq!(
        unsafe {
            ironwood_test_begin(
                seed.as_ptr(),
                seed.len(),
                core::ptr::null(),
                fixture.len(),
                &mut snapshot,
            )
        },
        REJECTED
    );
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), REJECTED);

    // The native Engine owns its parsed input after begin. Overwriting and
    // freeing caller storage cannot replace the transaction awaiting consent.
    let mut caller_input = fixture.to_vec();
    assert_eq!(begin(&caller_input, &mut snapshot), OK);
    caller_input.fill(0);
    drop(caller_input);
    assert_eq!(sign(&snapshot.token, &mut output, &mut written), OK);
    let evidence = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../evidence");
    std::fs::create_dir_all(&evidence).unwrap();
    std::fs::write(evidence.join("signed-inputs-2.pczt"), &output[..written]).unwrap();
    let outputs: Vec<_> = snapshot.outputs[..snapshot.output_count as usize]
        .iter()
        .map(|out| {
            json!({
                "action_index":out.action_index,
                "receiver":out.receiver.iter().map(|b| format!("{b:02x}")).collect::<String>(),
                "value":out.value,
                "kind":if out.kind==0 {"Payment"} else {"InternalChange"},
            })
        })
        .collect();
    let projection = json!({"network":"regtest (synthetic)","pool":"Ironwood",
        "branch":snapshot.branch,"expiry":snapshot.expiry,"total_input":snapshot.total_input,
        "payments":snapshot.payments,"change":snapshot.change,"fee":snapshot.fee,
        "padding_outputs":snapshot.padding_outputs,"outputs":outputs});
    std::fs::write(
        evidence.join("projection.json"),
        serde_json::to_vec_pretty(&projection).unwrap(),
    )
    .unwrap();
    assert_eq!(
        sign(&snapshot.token, &mut output, &mut written),
        REJECTED,
        "success must consume consent"
    );
    assert_eq!(written, 0);

    // The preserved outputs-8 fixture belongs to the pre-HD key boundary and
    // cannot validate the borrowed-seed ABI. The recovered unit test above now
    // owns terminal disposal/counter coverage; this integration test retains the
    // complete C-boundary replacement, cancellation, ownership and signing flow.
    ironwood_test_cancel();
}
