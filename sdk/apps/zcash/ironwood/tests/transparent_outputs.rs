#![cfg(feature = "test")]
//! Transparent OUTPUTS: the ZIP-244 digest, and the grammar that admits them.
//!
//! For a shielded-funded V6 transaction with transparent outputs and NO
//! transparent inputs, ZIP-244 §S.2 makes `transparent_sig_digest` identical
//! to the txid node §T.2 -- `BLAKE2b-256("ZTxIdTranspaHash", prevouts_digest ‖
//! sequence_digest ‖ outputs_digest)` with the first two the fixed empty
//! personalized digests -- so the device needs exactly ONE extra running
//! BLAKE2b state (`ZTxIdOutputsHash`, fed `LE64(value) ‖ CompactSize-prefixed
//! scriptPubKey` per output) and no per-input recomputation. That claim is
//! checked here against three independent oracles, and the admission rules
//! that keep it valid (no transparent inputs, standard scripts only) are
//! checked with it.
//!
//! `digest.rs` is crate-private and `lib.rs` exposes neither it nor `wire`,
//! so both are compiled into this binary from source with the crate-root
//! names they expect.

mod common;
#[allow(dead_code)]
#[path = "../src/digest.rs"]
mod digest;
#[allow(dead_code)]
#[path = "../src/error.rs"]
mod error;
#[allow(dead_code)]
#[path = "../src/wire.rs"]
mod wire;

use blake2b_simd::Params;
use common::*;
use digest::{ActionEffects, Digest};
// `digest.rs` and `wire.rs` name these through `crate::`. They are the same
// enum the library exports, compiled a second time, so the verdicts this file
// compares against `preflight` are the library's `ErrorCode`.
pub use error::{Error, Result};
use ironwood::testing::preflight;
use ironwood::{
    ErrorCode, MAX_ACTIONS, MAX_TRANSPARENT_OUTPUTS, USER_ADDRESS_BUDGET, ZIP32_HARDENED,
};
use pczt::Pczt;
use pczt::orchard::EncCiphertext;
use pczt::roles::signer::Signer;
use serde_json::{Value, json};
use zcash_primitives::transaction::sighash::SignableInput;
use zcash_primitives::transaction::sighash_v6::v6_signature_hash;
use zcash_primitives::transaction::txid::TxIdDigester;

/// The counts the spike names, plus the control.
const CASES: [usize; 4] = [0, 1, 2, 4];

fn fixture_with(count: usize) -> Vec<u8> {
    build_transparent(&transparent_values(count))
}

/// The exact bytes the device feeds its one running hash, per output.
fn fed_bytes(bytes: &[u8]) -> Vec<Vec<u8>> {
    Pczt::parse(bytes)
        .unwrap()
        .transparent()
        .outputs()
        .iter()
        .map(|output| {
            let script = output.script_pubkey();
            let mut fed = output.value().to_le_bytes().to_vec();
            assert!(script.len() < 253, "one-byte CompactSize");
            fed.push(script.len() as u8);
            fed.extend_from_slice(script);
            fed
        })
        .collect()
}

struct Nodes {
    transparent: [u8; 32],
    sighash: [u8; 32],
}

/// The streaming device path: one pass over the wire bytes, one extra running
/// BLAKE2b state for the transparent outputs.
fn streamed(bytes: &[u8]) -> Nodes {
    let header = wire::scan(bytes).expect("admitted");
    let pczt = Pczt::parse(bytes).unwrap();
    let mut digest = Digest::new(&header).unwrap();
    for output in pczt.transparent().outputs() {
        digest.transparent_output(*output.value(), output.script_pubkey());
    }
    let bundle = pczt.ironwood();
    for action in bundle.actions() {
        let output = action.output();
        let EncCiphertext::Encrypted(enc_ciphertext) = output.enc_ciphertext() else {
            panic!("fixtures carry encrypted notes");
        };
        digest.action(&ActionEffects {
            cv_net: action.cv_net().as_ref().unwrap(),
            nullifier: action.spend().nullifier(),
            rk: action.spend().rk(),
            cmx: output.cmx().as_ref().unwrap(),
            ephemeral_key: output.ephemeral_key(),
            enc_ciphertext: enc_ciphertext.as_slice().try_into().unwrap(),
            out_ciphertext: output.out_ciphertext().as_slice().try_into().unwrap(),
        });
    }
    let (magnitude, negative) = *bundle.value_sum();
    let magnitude = i64::try_from(magnitude).unwrap();
    let value_balance = if negative { -magnitude } else { magnitude };
    let flags = *bundle.flags();
    Nodes {
        transparent: digest.transparent_digest(),
        sighash: digest.finish(flags, value_balance),
    }
}

/// Upstream, whole-transaction: `zcash_primitives` over the same PCZT, with
/// the transparent T.2 node reassembled from upstream's own sub-digests.
fn reference(bytes: &[u8], transparent_outputs: usize) -> Nodes {
    let tx = Pczt::parse(bytes).unwrap().into_effects().unwrap();
    let parts = tx.digest(TxIdDigester);
    assert_eq!(parts.sapling_digest, None);
    assert_eq!(parts.orchard_digest, None);
    assert_eq!(
        parts.transparent_digests.is_some(),
        transparent_outputs > 0,
        "upstream carries transparent digests iff the bundle is nonempty"
    );
    let empty = |personal: &[u8; 16]| {
        Params::new()
            .hash_length(32)
            .personal(personal)
            .to_state()
            .finalize()
    };
    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdTranspaHash")
        .to_state();
    if let Some(digests) = parts.transparent_digests.as_ref() {
        // With no transparent inputs these two are the fixed empty digests;
        // that is the whole reason one running hash suffices.
        assert_eq!(digests.prevouts_digest, empty(b"ZTxIdPrevoutHash"));
        assert_eq!(digests.sequence_digest, empty(b"ZTxIdSequencHash"));
        h.update(digests.prevouts_digest.as_bytes())
            .update(digests.sequence_digest.as_bytes())
            .update(digests.outputs_digest.as_bytes());
    }
    Nodes {
        transparent: h.finalize().as_bytes().try_into().unwrap(),
        sighash: v6_signature_hash(&tx, &SignableInput::Shielded, &parts)
            .as_bytes()
            .try_into()
            .unwrap(),
    }
}

/// The T.2c outputs sub-hash, recomputed from the fed bytes alone.
fn outputs_digest(bytes: &[u8]) -> [u8; 32] {
    let mut h = Params::new()
        .hash_length(32)
        .personal(b"ZTxIdOutputsHash")
        .to_state();
    for fed in fed_bytes(bytes) {
        h.update(&fed);
    }
    h.finalize().as_bytes().try_into().unwrap()
}

#[test]
fn transparent_outputs_need_exactly_one_running_hash() {
    let mut sighashes = Vec::new();
    for count in CASES {
        let bytes = fixture_with(count);
        assert_eq!(
            Pczt::parse(&bytes).unwrap().transparent().outputs().len(),
            count
        );
        preflight(&bytes).unwrap_or_else(|error| panic!("{count} outputs: {error:?}"));

        let streamed = streamed(&bytes);
        let reference = reference(&bytes, count);
        assert_eq!(
            streamed.transparent, reference.transparent,
            "{count} outputs: transparent node"
        );
        assert_eq!(
            streamed.sighash, reference.sighash,
            "{count} outputs: v6_signature_hash"
        );
        // Third oracle: the pczt Signer, which derives everything itself.
        assert_eq!(
            Signer::new(Pczt::parse(&bytes).unwrap())
                .unwrap()
                .shielded_sighash(),
            streamed.sighash,
            "{count} outputs: pczt signer sighash"
        );
        // Fourth: upstream's own T.2c sub-digest equals a hash of nothing but
        // the bytes the device fed.
        if count > 0 {
            let parts = Pczt::parse(&bytes)
                .unwrap()
                .into_effects()
                .unwrap()
                .digest(TxIdDigester);
            assert_eq!(
                parts.transparent_digests.unwrap().outputs_digest.as_bytes(),
                outputs_digest(&bytes),
                "{count} outputs: T.2c over the fed bytes"
            );
        }
        sighashes.push(streamed.sighash);
    }
    // Every count binds: the outputs are committed to, not merely tolerated.
    for (i, a) in sighashes.iter().enumerate() {
        for b in &sighashes[i + 1..] {
            assert_ne!(a, b, "two counts share a sighash");
        }
    }
}

#[test]
fn no_transparent_output_leaves_the_fixed_empty_node() {
    let node = streamed(&fixture_with(0)).transparent;
    assert_eq!(
        node,
        Params::new()
            .hash_length(32)
            .personal(b"ZTxIdTranspaHash")
            .to_state()
            .finalize()
            .as_bytes(),
    );
    // The untouched shielded corpus agrees, so admitting the bundle changed
    // nothing for a transaction that has none.
    assert_eq!(streamed(&fixture()).transparent, node);
}

#[test]
fn every_output_field_and_its_order_binds() {
    let base = fixture_with(2);
    let mut digests = vec![streamed(&base).sighash];
    let mut check = |outputs: Vec<Value>, name: &str| {
        let bytes = with_transparent_bundle(&base, outputs);
        let streamed = streamed(&bytes);
        assert_eq!(streamed.sighash, reference(&bytes, 2).sighash, "{name}");
        assert!(!digests.contains(&streamed.sighash), "{name} does not bind");
        digests.push(streamed.sighash);
    };
    let values = transparent_values(2);
    let a = transparent_output_json(values[0], &transparent_script_for(0));
    let b = transparent_output_json(values[1], &transparent_script_for(1));
    check(vec![b.clone(), a.clone()], "order");
    check(
        vec![
            transparent_output_json(values[0] + 1, &transparent_script_for(0)),
            b.clone(),
        ],
        "value",
    );
    check(
        vec![
            transparent_output_json(values[0], &p2pkh([0xc7; 20])),
            b.clone(),
        ],
        "script hash160",
    );
    // A 23-byte P2SH where a 25-byte P2PKH was: a different length, so the
    // CompactSize prefix is load-bearing, not just the payload.
    check(
        vec![transparent_output_json(values[0], &p2sh([0xa0; 20])), b],
        "script kind",
    );
    // `user_address` is outside the sighash, on both sides.
    let mut ignored = json(&base);
    ignored["transparent"]["outputs"][0]["user_address"] =
        Value::String("tmEXAMPLEaddressstringshowntotheuser".into());
    let ignored = encode(ignored);
    preflight(&ignored).unwrap();
    assert_eq!(streamed(&ignored).sighash, digests[0]);
    assert_eq!(reference(&ignored, 2).sighash, digests[0]);
}

#[test]
fn the_fed_bytes_are_value_then_prefixed_script() {
    let bytes = fixture_with(1);
    let fed = fed_bytes(&bytes);
    assert_eq!(fed.len(), 1);
    let mut expected = transparent_values(1)[0].to_le_bytes().to_vec();
    expected.push(25);
    expected.extend_from_slice(&transparent_script_for(0));
    assert_eq!(fed[0], expected);
    assert_eq!(fed[0].len(), 8 + 1 + 25);
}

/// A transparent bundle mutated through the v2 JSON view, and the class the
/// grammar must give it.
fn rejected(name: &str, transparent: Value, code: ErrorCode) {
    let mut value = json(&fixture_with(2));
    value["transparent"] = transparent;
    let bytes = encode(value);
    assert_eq!(preflight(&bytes).unwrap_err(), code, "{name}");
}

#[test]
fn a_transparent_input_is_refused() {
    // Zero inputs is what makes ZIP-244 §S.2 collapse to §T.2, and the device
    // has no transparent keychain: an input is policy, not a parse failure.
    let input = json!({
        "prevout_txid": vec![0x11; 32],
        "prevout_index": 0,
        "sequence": Value::Null,
        "required_time_lock_time": Value::Null,
        "required_height_lock_time": Value::Null,
        "script_sig": Value::Null,
        "value": 1_000_000,
        "script_pubkey": p2pkh([0x22; 20]),
        "redeem_script": Value::Null,
        "partial_signatures": {},
        "sighash_type": 1,
        "bip32_derivation": {},
        "ripemd160_preimages": {},
        "sha256_preimages": {},
        "hash160_preimages": {},
        "hash256_preimages": {},
        "proprietary": {},
    });
    let outputs = vec![transparent_output_json(
        transparent_values(1)[0],
        &transparent_script_for(0),
    )];
    rejected(
        "one input",
        json!({ "inputs": [input], "outputs": outputs }),
        ErrorCode::Policy,
    );
}

#[test]
fn a_present_but_empty_transparent_bundle_is_refused() {
    // The v2 encoder writes an empty bundle as an absent tag, so this would be
    // a second encoding of the same transaction.
    rejected(
        "no outputs",
        json!({ "inputs": [], "outputs": [] }),
        ErrorCode::Policy,
    );
}

#[test]
fn only_the_two_standard_script_shapes_are_admitted() {
    let value = transparent_values(1)[0];
    let one = |script: &[u8]| json!([transparent_output_json(value, script)]);
    // Both admitted shapes, on their own.
    for script in [p2pkh([0x31; 20]), p2sh([0x31; 20])] {
        let mut v = json(&fixture_with(2));
        v["transparent"] = json!({ "inputs": [], "outputs": one(&script) });
        // Only the shape is under test here; the value sum no longer balances,
        // which admission does not check.
        preflight(&encode(v)).unwrap();
    }
    for (name, script) in [
        // OP_RETURN, the shape Ledger refuses too: unspendable, no address.
        ("op_return", vec![0x6a, 0x14]),
        // P2PK: a bare 33-byte pubkey, which has no Base58Check encoding.
        ("p2pk", {
            let mut s = vec![0x21];
            s.extend_from_slice(&[0x02; 33]);
            s.push(0xac);
            s
        }),
        // P2PKH with one opcode changed, at the admitted length.
        ("p2pkh_with_op_equal", {
            let mut s = p2pkh([0x31; 20]);
            s[0] = 0x77;
            s
        }),
        ("p2sh_with_op_equalverify", {
            let mut s = p2sh([0x31; 20]);
            s[22] = 0x88;
            s
        }),
        // A 24-byte script: neither admitted length.
        ("truncated_p2pkh", p2pkh([0x31; 20])[..24].to_vec()),
        ("empty", Vec::new()),
    ] {
        rejected(
            name,
            json!({ "inputs": [], "outputs": one(&script) }),
            ErrorCode::Policy,
        );
    }
}

#[test]
fn the_ignored_output_fields_must_be_absent_or_empty() {
    let value = transparent_values(1)[0];
    let mut base = transparent_output_json(value, &transparent_script_for(0));
    // `user_address` is the one admitted-and-ignored field, within its budget.
    base["user_address"] = Value::String("t".repeat(USER_ADDRESS_BUDGET));
    let mut v = json(&fixture_with(2));
    v["transparent"] = json!({ "inputs": [], "outputs": [base.clone()] });
    preflight(&encode(v)).unwrap();

    let mut over = base.clone();
    over["user_address"] = Value::String("t".repeat(USER_ADDRESS_BUDGET + 1));
    rejected(
        "user_address over budget",
        json!({ "inputs": [], "outputs": [over] }),
        ErrorCode::Policy,
    );

    let mut redeem = base.clone();
    redeem["redeem_script"] = json!(p2pkh([0x31; 20]));
    rejected(
        "redeem_script present",
        json!({ "inputs": [], "outputs": [redeem] }),
        ErrorCode::Policy,
    );

    let mut proprietary = base;
    proprietary["proprietary"] = json!({ "vendor": [1, 2, 3] });
    rejected(
        "proprietary nonempty",
        json!({ "inputs": [], "outputs": [proprietary] }),
        ErrorCode::Policy,
    );
}

#[test]
fn transparent_outputs_and_actions_share_one_logical_action_cap() {
    // ZIP-317 counts a standard transparent output as one logical action, so
    // the device charges it against the same 32 the Ironwood actions use.
    assert_eq!(MAX_TRANSPARENT_OUTPUTS, MAX_ACTIONS - 1);
    let value = transparent_values(1)[0];
    let outputs = |count: usize| -> Value {
        (0..count)
            .map(|i| transparent_output_json(value, &transparent_script_for(i)))
            .collect::<Vec<_>>()
            .into()
    };
    // The corpus fixture has two Ironwood actions, so 30 outputs is the cap
    // and 31 is one too many even though it is within MAX_TRANSPARENT_OUTPUTS.
    let bytes = fixture_with(0);
    let actions = Pczt::parse(&bytes).unwrap().ironwood().actions().len();
    assert_eq!(actions, 2);
    for (count, expected) in [
        (MAX_ACTIONS - actions, None),
        (MAX_ACTIONS - actions + 1, Some(ErrorCode::Capacity)),
        (MAX_TRANSPARENT_OUTPUTS + 1, Some(ErrorCode::Capacity)),
    ] {
        let mut v = json(&bytes);
        v["transparent"] = json!({ "inputs": [], "outputs": outputs(count) });
        assert_eq!(preflight(&encode(v)).err(), expected, "{count} outputs");
    }
}

/// Keeps the `ZIP32_HARDENED` import honest: `wire.rs` needs it at the crate
/// root, and an unused import is a warning the style check refuses.
#[test]
fn the_wire_module_shares_this_crate_roots_names() {
    assert_eq!(ZIP32_HARDENED, 1 << 31);
}
