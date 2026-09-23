#![cfg(feature = "test")]
//! Differential tests for the incremental scanner: it must accept and reject
//! exactly what `wire::scan` does (reached here as `testing::preflight`), with
//! the same error class under every chunking, and the fields it yields must
//! equal the upstream deserializer's.

// `stream` is a private crate module and the `#![no_std]` library cannot host
// the fixture module in its unit tests, so the module is compiled here with
// the crate-root names it expects.
mod common;
#[path = "../src/stream.rs"]
mod stream;

use common::*;
use ironwood::testing::preflight;
use ironwood::{
    Error, ErrorCode, MAX_ACTIONS, MAX_PCZT_BYTES, MAX_SCRIPT_PUBKEY_BYTES,
    MAX_TRANSPARENT_OUTPUTS, Network, P2PKH_SCRIPT_BYTES, P2SH_SCRIPT_BYTES, Result,
    TransparentKind, USER_ADDRESS_BUDGET, ZIP32_HARDENED,
};
use pczt::Pczt;
use pczt::roles::verifier::{OrchardError, Verifier};
use serde_json::{Value, json};
use stream::{
    ACTION_BUDGET, HEADER_BUDGET, Header, Item, SECTION_BUDGET, SHIELDED_BUDGET, Scanner,
    TRAILER_BUDGET, TRANSPARENT_OUTPUT_BUDGET, Zip32Derivation,
};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::MAX_MONEY;

const WHOLE: usize = usize::MAX;
const CHUNKINGS: [usize; 5] = [1, 7, 64, 1024, WHOLE];

/// Longest accepted encodings: minimal canonical varints for the fixed values,
/// eight-byte varints for money, every optional field present (an empty
/// Sapling bundle with anchor and `bsk`, a `user_address` at its budget, the
/// Ironwood `bsk`).
const HEADER_ACCEPTED_MAX: usize = 8 + 3 * 5 + (1 + 5) + 5 + 5 + 1 + 1 + 1 + 1 + 1;
/// The value, the longest admitted script with its prefix, the absent
/// `redeem_script`, the empty `bip32_derivation`, a `user_address` at its
/// budget and the empty proprietary map.
const TRANSPARENT_OUTPUT_ACCEPTED_MAX: usize =
    8 + (1 + MAX_SCRIPT_PUBKEY_BYTES) + 1 + 1 + (1 + 2 + USER_ADDRESS_BUDGET) + 1;
/// An empty Sapling bundle with anchor and `bsk`, the absent Orchard tag, the
/// Ironwood tag and the action count.
const SHIELDED_ACCEPTED_MAX: usize = 1 + 3 + 33 + 33 + 1 + 1 + 1;
const ACTION_ACCEPTED_MAX: usize = 9 * 33
    + 65
    + 2 * 44
    + 2 * 9
    + 97
    + 2
    + 2 * (1 + 32 + 1 + 3 * 5)
    + 2
    + 32
    + 1
    + 582
    + 81
    + 33
    + (1 + 2 + USER_ADDRESS_BUDGET);
const TRAILER_ACCEPTED_MAX: usize = 1 + 8 + 1 + 33 + 1 + 1 + 33;
// The transparent outputs and the actions share one cap, and an output's
// longest encoding is far shorter than an action's, so the longest admitted
// PCZT is the all-actions one.
const _: () = assert!(TRANSPARENT_OUTPUT_ACCEPTED_MAX < ACTION_ACCEPTED_MAX);
const _: () = assert!(MAX_TRANSPARENT_OUTPUTS < MAX_ACTIONS);
const _: () = assert!(
    HEADER_ACCEPTED_MAX
        + SHIELDED_ACCEPTED_MAX
        + MAX_ACTIONS * ACTION_ACCEPTED_MAX
        + TRAILER_ACCEPTED_MAX
        <= MAX_PCZT_BYTES
);

#[derive(Clone, Debug, PartialEq, Eq)]
struct OwnedAction {
    cv_net: [u8; 32],
    nullifier: [u8; 32],
    rk: [u8; 32],
    spend_auth_sig: Option<[u8; 64]>,
    spend_recipient: [u8; 43],
    spend_value: u64,
    rho: [u8; 32],
    spend_rseed: [u8; 32],
    fvk: [u8; 96],
    alpha: [u8; 32],
    spend_zip32_derivation: Option<([u8; 32], [u32; 3])>,
    cmx: [u8; 32],
    ephemeral_key: [u8; 32],
    enc_ciphertext: [u8; 580],
    out_ciphertext: [u8; 80],
    output_recipient: [u8; 43],
    output_value: u64,
    output_rseed: [u8; 32],
    ock: Option<[u8; 32]>,
    output_zip32_derivation: Option<([u8; 32], [u32; 3])>,
    rcv: [u8; 32],
}

fn owned_derivation(claim: Option<Zip32Derivation<'_>>) -> Option<([u8; 32], [u32; 3])> {
    claim.map(|claim| (*claim.seed_fingerprint, claim.path))
}

impl OwnedAction {
    fn from(action: &stream::Action<'_>) -> Self {
        let spend = &action.spend;
        let output = &action.output;
        Self {
            cv_net: *action.cv_net,
            nullifier: *spend.nullifier,
            rk: *spend.rk,
            spend_auth_sig: spend.spend_auth_sig.copied(),
            spend_recipient: *spend.recipient,
            spend_value: spend.value,
            rho: *spend.rho,
            spend_rseed: *spend.rseed,
            fvk: *spend.fvk,
            alpha: *spend.alpha,
            spend_zip32_derivation: owned_derivation(spend.zip32_derivation),
            cmx: *output.cmx,
            ephemeral_key: *output.ephemeral_key,
            enc_ciphertext: *output.enc_ciphertext,
            out_ciphertext: *output.out_ciphertext,
            output_recipient: *output.recipient,
            output_value: output.value,
            output_rseed: *output.rseed,
            ock: output.ock.copied(),
            output_zip32_derivation: owned_derivation(output.zip32_derivation),
            rcv: *action.rcv,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OwnedTrailer {
    flags: u8,
    value_sum: u64,
    anchor: Option<[u8; 32]>,
}

/// One yielded transparent output, owned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OwnedTransparentOutput {
    value: u64,
    kind: TransparentKind,
    hash: [u8; 20],
    script_pubkey: [u8; MAX_SCRIPT_PUBKEY_BYTES],
    script_len: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Scanned {
    header: Option<Header>,
    transparent_outputs: Vec<OwnedTransparentOutput>,
    /// Declared Ironwood action count, yielded after the transparent bundle.
    actions_declared: Option<usize>,
    actions: Vec<OwnedAction>,
    trailer: Option<OwnedTrailer>,
    /// Byte length of every completed section, in order.
    sections: Vec<usize>,
}

struct Run {
    result: Result<()>,
    scanned: Scanned,
    /// Bytes handed to the scanner before it answered.
    fed: usize,
}

fn run(bytes: &[u8], chunk: usize) -> Run {
    let mut scanned = Scanned::default();
    let mut fed = 0;
    let mut section_start = 0;
    let result = (|| {
        let mut scanner = Scanner::new(bytes.len())?;
        for piece in bytes.chunks(chunk) {
            let mut rest = piece;
            while !rest.is_empty() {
                let (consumed, item) = scanner.feed(rest)?;
                assert!(consumed > 0, "no progress on {} bytes", rest.len());
                fed += consumed;
                rest = &rest[consumed..];
                let Some(item) = item else { continue };
                scanned.sections.push(fed - section_start);
                section_start = fed;
                match item {
                    Item::Header(header) => scanned.header = Some(header),
                    Item::TransparentOutput(output) => {
                        let mut script_pubkey = [0; MAX_SCRIPT_PUBKEY_BYTES];
                        script_pubkey[..output.script_pubkey.len()]
                            .copy_from_slice(output.script_pubkey);
                        scanned.transparent_outputs.push(OwnedTransparentOutput {
                            value: output.value,
                            kind: output.kind,
                            hash: output.hash,
                            script_pubkey,
                            script_len: output.script_pubkey.len(),
                        });
                    }
                    Item::Shielded(shielded) => scanned.actions_declared = Some(shielded.actions),
                    Item::Action(action) => scanned.actions.push(OwnedAction::from(&action)),
                    Item::Trailer(trailer) => {
                        scanned.trailer = Some(OwnedTrailer {
                            flags: trailer.flags,
                            value_sum: trailer.value_sum,
                            anchor: trailer.anchor.copied(),
                        })
                    }
                }
            }
        }
        assert!(scanner.is_finished(), "input ended without a verdict");
        Ok(())
    })();
    Run {
        result,
        scanned,
        fed,
    }
}

fn scan(bytes: &[u8], chunk: usize) -> Result<Scanned> {
    let run = run(bytes, chunk);
    run.result.map(|()| run.scanned)
}

fn assert_same_verdict(name: &str, bytes: &[u8]) -> Option<Scanned> {
    let expected = preflight(bytes);
    let whole = scan(bytes, WHOLE);
    for chunk in CHUNKINGS {
        let actual = scan(bytes, chunk);
        assert_eq!(
            actual.as_ref().err(),
            expected.as_ref().err(),
            "{name}: chunk {chunk}"
        );
        assert_eq!(actual, whole, "{name}: chunk {chunk} differs from whole");
    }
    whole.ok()
}

fn octets(value: &Value) -> Vec<u8> {
    value
        .as_array()
        .unwrap_or_else(|| panic!("not a byte array: {value}"))
        .iter()
        .map(|byte| u8::try_from(byte.as_u64().unwrap()).unwrap())
        .collect()
}

fn optional_octets(value: &Value) -> Option<Vec<u8>> {
    if value.is_null() {
        None
    } else {
        Some(octets(value))
    }
}

fn optional_derivation(value: &Value) -> Option<([u8; 32], [u32; 3])> {
    if value.is_null() {
        return None;
    }
    let fingerprint: [u8; 32] = octets(&value["seed_fingerprint"]).try_into().unwrap();
    let path: Vec<u32> = value["derivation_path"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i.as_u64().unwrap() as u32)
        .collect();
    Some((fingerprint, path.try_into().unwrap()))
}

fn upstream_json(bytes: &[u8]) -> Option<Value> {
    let v2 = pczt::v2::Pczt::try_from(Pczt::parse(bytes).ok()?).ok()?;
    Some(serde_json::to_value(v2).unwrap())
}

/// Every yielded field must equal the upstream deserializer's raw field.
fn assert_matches_upstream_json(name: &str, scanned: &Scanned, upstream: &Value) {
    let global = &upstream["global"];
    let header = scanned.header.unwrap();
    assert_eq!(
        header,
        Header {
            version: global["tx_version"].as_u64().unwrap() as u32,
            group: global["version_group_id"].as_u64().unwrap() as u32,
            branch: global["consensus_branch_id"].as_u64().unwrap() as u32,
            lock_time: global["fallback_lock_time"].as_u64().unwrap_or(0) as u32,
            expiry: global["expiry_height"].as_u64().unwrap() as u32,
            coin_type: global["coin_type"].as_u64().unwrap() as u32,
            transparent_outputs: upstream["transparent"]["outputs"]
                .as_array()
                .map_or(0, |outputs| outputs.len()),
        },
        "{name}: header"
    );
    assert_eq!(
        scanned.actions_declared,
        Some(upstream["ironwood"]["actions"].as_array().unwrap().len()),
        "{name}: action count"
    );
    for (i, output) in scanned.transparent_outputs.iter().enumerate() {
        let t = &upstream["transparent"]["outputs"][i];
        assert_eq!(
            output.value,
            t["value"].as_u64().unwrap(),
            "{name}: t{i} value"
        );
        assert_eq!(
            &output.script_pubkey[..output.script_len],
            octets(&t["script_pubkey"]).as_slice(),
            "{name}: t{i} script"
        );
    }
    for (i, action) in scanned.actions.iter().enumerate() {
        let a = &upstream["ironwood"]["actions"][i];
        let spend = &a["spend"];
        let output = &a["output"];
        assert_eq!(
            action.cv_net.to_vec(),
            octets(&a["cv_net"]),
            "{name}: {i} cv_net"
        );
        assert_eq!(action.nullifier.to_vec(), octets(&spend["nullifier"]));
        assert_eq!(action.rk.to_vec(), octets(&spend["rk"]));
        assert_eq!(
            action.spend_auth_sig.map(|s| s.to_vec()),
            optional_octets(&spend["spend_auth_sig"])
        );
        assert_eq!(action.spend_recipient.to_vec(), octets(&spend["recipient"]));
        assert_eq!(action.spend_value, spend["value"].as_u64().unwrap());
        assert_eq!(action.rho.to_vec(), octets(&spend["rho"]));
        assert_eq!(action.spend_rseed.to_vec(), octets(&spend["rseed"]));
        assert_eq!(action.fvk.to_vec(), octets(&spend["fvk"]));
        assert_eq!(action.alpha.to_vec(), octets(&spend["alpha"]));
        assert_eq!(
            action.spend_zip32_derivation,
            optional_derivation(&spend["zip32_derivation"]),
            "{name}: {i} spend zip32_derivation"
        );
        assert_eq!(action.cmx.to_vec(), octets(&output["cmx"]));
        assert_eq!(
            action.ephemeral_key.to_vec(),
            octets(&output["ephemeral_key"])
        );
        assert_eq!(
            action.enc_ciphertext.to_vec(),
            octets(&output["enc_ciphertext"]["Encrypted"])
        );
        assert_eq!(
            action.out_ciphertext.to_vec(),
            octets(&output["out_ciphertext"])
        );
        assert_eq!(
            action.output_recipient.to_vec(),
            octets(&output["recipient"])
        );
        assert_eq!(action.output_value, output["value"].as_u64().unwrap());
        assert_eq!(action.output_rseed.to_vec(), octets(&output["rseed"]));
        assert_eq!(
            action.ock.map(|o| o.to_vec()),
            optional_octets(&output["ock"])
        );
        assert_eq!(
            action.output_zip32_derivation,
            optional_derivation(&output["zip32_derivation"]),
            "{name}: {i} output zip32_derivation"
        );
        assert_eq!(action.rcv.to_vec(), octets(&a["rcv"]), "{name}: {i} rcv");
    }
    let bundle = &upstream["ironwood"];
    let trailer = scanned.trailer.unwrap();
    assert_eq!(u64::from(trailer.flags), bundle["flags"].as_u64().unwrap());
    assert_eq!(trailer.value_sum, bundle["value_sum"][0].as_u64().unwrap());
    assert_eq!(bundle["value_sum"][1], json!(false));
    assert_eq!(
        trailer.anchor.map(|a| a.to_vec()),
        optional_octets(&bundle["anchor"]),
        "{name}: anchor"
    );
}

/// The same fields through `orchard::pczt::Action` accessors. Returns whether
/// upstream's stricter bundle parser accepted the input at all.
fn assert_matches_orchard_accessors(name: &str, bytes: &[u8], scanned: &Scanned) -> bool {
    let verified = Verifier::new(Pczt::parse(bytes).unwrap()).with_ironwood(
        |bundle| -> std::result::Result<(), OrchardError<()>> {
            assert_eq!(bundle.actions().len(), scanned.actions.len(), "{name}");
            for (a, s) in bundle.actions().iter().zip(&scanned.actions) {
                let spend = a.spend();
                let output = a.output();
                assert_eq!(a.cv_net().to_bytes(), s.cv_net);
                assert_eq!(spend.nullifier().to_bytes(), s.nullifier);
                assert_eq!(<[u8; 32]>::from(spend.rk()), s.rk);
                assert_eq!(
                    spend.spend_auth_sig().as_ref().map(<[u8; 64]>::from),
                    s.spend_auth_sig
                );
                assert_eq!(
                    spend.recipient().unwrap().to_raw_address_bytes(),
                    s.spend_recipient
                );
                assert_eq!(spend.value().unwrap().inner(), s.spend_value);
                assert_eq!(spend.rho().unwrap().to_bytes(), s.rho);
                assert_eq!(*spend.rseed().unwrap().as_bytes(), s.spend_rseed);
                assert_eq!(spend.fvk().as_ref().unwrap().to_bytes(), s.fvk);
                assert_eq!(output.cmx().to_bytes(), s.cmx);
                let note = output.encrypted_note();
                assert_eq!(note.epk_bytes, s.ephemeral_key);
                assert_eq!(note.enc_ciphertext, s.enc_ciphertext);
                assert_eq!(note.out_ciphertext, s.out_ciphertext);
                assert_eq!(
                    output.recipient().unwrap().to_raw_address_bytes(),
                    s.output_recipient
                );
                assert_eq!(output.value().unwrap().inner(), s.output_value);
                assert_eq!(*output.rseed().unwrap().as_bytes(), s.output_rseed);
            }
            let trailer = scanned.trailer.unwrap();
            assert_eq!(bundle.flag_byte(), trailer.flags, "{name}");
            assert_eq!(
                i64::try_from(*bundle.value_sum()).unwrap(),
                i64::try_from(trailer.value_sum).unwrap()
            );
            if let Some(anchor) = trailer.anchor {
                assert_eq!(bundle.anchor().to_bytes(), anchor);
            }
            Ok(())
        },
    );
    match verified {
        Ok(_) => true,
        Err(OrchardError::Parse(_)) => false,
        Err(error) => panic!("{name}: {error:?}"),
    }
}

fn with_action(v: &mut Value, index: usize) -> &mut Value {
    &mut v["ironwood"]["actions"][index]
}

/// The fixture with an empty Sapling bundle, as the Full signer view keeps it:
/// with or without the default anchor and the host's `bsk`.
fn sapling_present(anchor: bool, bsk: bool) -> Vec<u8> {
    mutate(|v| {
        v["sapling"] = json!({
            "spends": [],
            "outputs": [],
            "value_sum": 0,
            "anchor": if anchor { json!(vec![0u8; 32]) } else { Value::Null },
            "bsk": if bsk { json!(vec![9u8; 32]) } else { Value::Null },
        })
    })
}

/// `sapling_present(false, false)` with the byte `offset` past the Sapling
/// tag overwritten: 1 is the spend count, 2 the output count.
fn patch_after_sapling_tag(offset: usize, byte: u8) -> Vec<u8> {
    let absent = fixture();
    let mut present = sapling_present(false, false);
    let tag = absent
        .iter()
        .zip(&present)
        .position(|(a, b)| a != b)
        .unwrap();
    assert_eq!(present[tag], 1);
    present[tag + offset] = byte;
    present
}

/// A two-byte `user_address` whose first byte is not UTF-8: what postcard's
/// `String` refuses.
fn user_address_with_invalid_utf8() -> Vec<u8> {
    let absent = fixture();
    let mut present = mutate(|v| {
        let i = payment(v);
        with_action(v, i)["output"]["user_address"] = json!("ab");
    });
    let tag = absent
        .iter()
        .zip(&present)
        .position(|(a, b)| a != b)
        .unwrap();
    assert_eq!(present[tag..tag + 4], [1, 2, b'a', b'b']);
    present[tag + 2] = 0xff;
    present
}

fn accepted_corpus() -> Vec<(String, Vec<u8>)> {
    let mut corpus = vec![
        ("fixture".into(), fixture()),
        ("mainnet".into(), network_fixture(Network::Mainnet)),
        ("testnet".into(), network_fixture(Network::Testnet)),
        ("wrong change ovk".into(), build_with_wrong_change_ovk()),
        (
            "discarded payment ovk".into(),
            build_with_discarded_payment_ovk(),
        ),
        (
            "dummy spend padding".into(),
            build_with_dummy_spend_padding(),
        ),
        (
            "zero change".into(),
            build(990_000, 0, MemoBytes::empty(), false),
        ),
        (
            "self payment".into(),
            build(600_000, 390_000, MemoBytes::empty(), true),
        ),
        (
            "nonempty memo".into(),
            build(
                600_000,
                390_000,
                MemoBytes::from_bytes(b"hello").unwrap(),
                false,
            ),
        ),
        ("two inputs".into(), build_inputs(&[100_000; 2])),
        ("eight inputs".into(), build_inputs(&[100_000; 8])),
        (
            "overflowing inputs".into(),
            build_inputs(&[MAX_MONEY / 2, MAX_MONEY / 2 + 1]),
        ),
        (
            "explicit zero lock time".into(),
            mutate(|v| v["global"]["fallback_lock_time"] = json!(0)),
        ),
        (
            "lock time one".into(),
            mutate(|v| v["global"]["fallback_lock_time"] = json!(1)),
        ),
        (
            "zero anchor".into(),
            mutate(|v| v["ironwood"]["anchor"] = json!(vec![0; 32])),
        ),
        (
            "absent anchor".into(),
            mutate(|v| v["ironwood"]["anchor"] = Value::Null),
        ),
        (
            "ock".into(),
            mutate(|v| {
                let i = payment(v);
                with_action(v, i)["output"]["ock"] = json!(vec![7; 32]);
            }),
        ),
        ("sapling present".into(), sapling_present(false, false)),
        ("sapling full view".into(), sapling_present(true, true)),
        (
            "user_address".into(),
            mutate(|v| {
                let i = payment(v);
                with_action(v, i)["output"]["user_address"] = json!("u1recipient");
            }),
        ),
        (
            "user_address at budget".into(),
            mutate(|v| {
                let i = payment(v);
                with_action(v, i)["output"]["user_address"] =
                    json!("u".repeat(USER_ADDRESS_BUDGET));
            }),
        ),
        (
            "bsk present".into(),
            mutate(|v| v["ironwood"]["bsk"] = json!(vec![0u8; 32])),
        ),
        ("change without ovk".into(), build_with_change_without_ovk()),
        ("stock sdk view".into(), build_stock_sdk_view()),
        ("own zip32 derivation".into(), with_own_derivation(true)),
        (
            "other zip32 derivation".into(),
            mutate(|v| {
                let path = [0x8000_0020, 0x8000_0085, 0xffff_ffff];
                with_action(v, 0)["spend"]["zip32_derivation"] = derivation_json(&[7; 32], &path);
                with_action(v, 1)["output"]["zip32_derivation"] = derivation_json(&[8; 32], &path);
            }),
        ),
        (
            "absent dummy signature".into(),
            mutate(|v| {
                let i = 1 - real(v);
                with_action(v, i)["spend"]["spend_auth_sig"] = Value::Null;
            }),
        ),
        (
            "flags zero".into(),
            mutate(|v| v["ironwood"]["flags"] = json!(0)),
        ),
        (
            "duplicated action".into(),
            mutate(|v| {
                let action = with_action(v, real(v)).clone();
                v["ironwood"]["actions"] = json!([action.clone(), action]);
            }),
        ),
        (
            "five-byte header varints".into(),
            mutate(|v| {
                for field in [
                    "tx_version",
                    "version_group_id",
                    "consensus_branch_id",
                    "fallback_lock_time",
                    "expiry_height",
                    "coin_type",
                ] {
                    v["global"][field] = json!(u32::MAX);
                }
            }),
        ),
        (
            "max money everywhere".into(),
            mutate(|v| {
                for i in 0..2 {
                    with_action(v, i)["spend"]["value"] = json!(MAX_MONEY);
                    with_action(v, i)["output"]["value"] = json!(MAX_MONEY);
                }
                v["ironwood"]["value_sum"][0] = json!(MAX_MONEY);
            }),
        ),
    ];
    for outputs in 1..=CORPUS_MAX_ACTIONS {
        corpus.push((format!("{outputs} actions"), build_actions(outputs)));
    }
    for count in [1usize, 2, 4] {
        corpus.push((
            format!("{count} transparent outputs"),
            build_transparent(&transparent_values(count)),
        ));
    }
    // A `user_address` at its budget on a transparent output, the one field
    // there the device admits and ignores.
    corpus.push((
        "transparent user_address at budget".into(),
        with_transparent_bundle(&build_transparent(&transparent_values(1)), {
            let mut output =
                transparent_output_json(transparent_values(1)[0], &transparent_script_for(0));
            output["user_address"] = json!("t".repeat(USER_ADDRESS_BUDGET));
            vec![output]
        }),
    ));
    // The cap, exactly: the corpus fixture has two Ironwood actions, and a
    // transparent output is a logical action just like one of those.
    corpus.push((
        "transparent outputs at the logical-action cap".into(),
        with_transparent_bundle(
            &build_transparent(&transparent_values(1)),
            (0..MAX_ACTIONS - 2)
                .map(|i| transparent_output_json(1, &transparent_script_for(i)))
                .collect(),
        ),
    ));
    corpus
}

fn rejected_corpus() -> Vec<(String, Vec<u8>, ErrorCode)> {
    use ErrorCode::{Capacity, Malformed, Policy};
    let mut corpus: Vec<(String, Vec<u8>, ErrorCode)> = vec![
        ("empty".into(), Vec::new(), Malformed),
        ("short".into(), b"PCZT".to_vec(), Malformed),
        ("oversize".into(), vec![0; MAX_PCZT_BYTES + 1], Capacity),
        ("largest zeros".into(), vec![0; MAX_PCZT_BYTES], Malformed),
        (
            "v1 magic".into(),
            {
                let mut bytes = fixture();
                bytes[4] = 1;
                bytes
            },
            Malformed,
        ),
        (
            "overlong version varint".into(),
            {
                let mut bytes = fixture();
                bytes.splice(8..9, [0x86, 0]);
                bytes
            },
            Malformed,
        ),
        (
            "trailing byte".into(),
            {
                let mut bytes = fixture();
                bytes.push(0);
                bytes
            },
            Malformed,
        ),
        (
            "tx modifiable".into(),
            mutate(|v| v["global"]["tx_modifiable"] = json!(128)),
            Policy,
        ),
        (
            "global proprietary".into(),
            mutate(|v| v["global"]["proprietary"] = json!({"x": [1]})),
            Policy,
        ),
        // The v2 encoder writes the empty bundle as an absent tag, so a
        // present-and-empty bundle is a second encoding of the same
        // transaction.
        (
            "transparent present and empty".into(),
            mutate(|v| v["transparent"] = json!({"inputs": [], "outputs": []})),
            Policy,
        ),
        (
            "transparent input".into(),
            mutate(|v| {
                v["transparent"] = json!({
                    "inputs": [{
                        "prevout_txid": vec![0x11u8; 32],
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
                    }],
                    "outputs": [transparent_output_json(100_000, &p2pkh([0x33; 20]))],
                })
            }),
            Policy,
        ),
        (
            "transparent script not standard".into(),
            with_transparent_bundle(
                &fixture(),
                vec![transparent_output_json(100_000, &[0x6a, 0x14])],
            ),
            Policy,
        ),
        (
            "transparent script at an admitted length but the wrong shape".into(),
            with_transparent_bundle(&fixture(), {
                let mut script = p2pkh([0x33; 20]);
                script[0] = 0x77;
                vec![transparent_output_json(100_000, &script)]
            }),
            Policy,
        ),
        (
            "transparent redeem_script".into(),
            with_transparent_bundle(&fixture(), {
                let mut output = transparent_output_json(100_000, &p2pkh([0x33; 20]));
                output["redeem_script"] = json!(p2sh([0x44; 20]));
                vec![output]
            }),
            Policy,
        ),
        (
            "transparent proprietary".into(),
            with_transparent_bundle(&fixture(), {
                let mut output = transparent_output_json(100_000, &p2pkh([0x33; 20]));
                output["proprietary"] = json!({"x": [1]});
                vec![output]
            }),
            Policy,
        ),
        (
            "transparent user_address over budget".into(),
            with_transparent_bundle(&fixture(), {
                let mut output = transparent_output_json(100_000, &p2pkh([0x33; 20]));
                output["user_address"] = json!("t".repeat(USER_ADDRESS_BUDGET + 1));
                vec![output]
            }),
            Policy,
        ),
        (
            "transparent value over max money".into(),
            with_transparent_bundle(
                &fixture(),
                vec![transparent_output_json(MAX_MONEY + 1, &p2pkh([0x33; 20]))],
            ),
            Malformed,
        ),
        (
            "transparent outputs past the logical-action cap".into(),
            with_transparent_bundle(
                &fixture(),
                (0..MAX_ACTIONS - 1)
                    .map(|i| transparent_output_json(1, &transparent_script_for(i)))
                    .collect(),
            ),
            Capacity,
        ),
        (
            "more transparent outputs than the transparent cap".into(),
            with_transparent_bundle(
                &fixture(),
                (0..MAX_TRANSPARENT_OUTPUTS + 1)
                    .map(|i| transparent_output_json(1, &transparent_script_for(i % 2)))
                    .collect(),
            ),
            Capacity,
        ),
        (
            "sapling spend".into(),
            patch_after_sapling_tag(1, 1),
            Policy,
        ),
        (
            "sapling output".into(),
            patch_after_sapling_tag(2, 1),
            Policy,
        ),
        (
            "sapling value sum".into(),
            mutate(|v| {
                v["sapling"] = json!({
                    "spends": [], "outputs": [], "value_sum": 1, "anchor": null, "bsk": null
                })
            }),
            Policy,
        ),
        (
            "orchard present".into(),
            mutate(|v| v["orchard"] = v["ironwood"].clone()),
            Policy,
        ),
        (
            "no ironwood".into(),
            mutate(|v| v["ironwood"] = Value::Null),
            Policy,
        ),
        (
            "zero actions".into(),
            mutate(|v| v["ironwood"]["actions"] = json!([])),
            Policy,
        ),
        (
            "nine actions".into(),
            mutate(|v| {
                let action = with_action(v, 0).clone();
                v["ironwood"]["actions"] = vec![action; MAX_ACTIONS + 1].into();
            }),
            Capacity,
        ),
        (
            "spend value u64::MAX".into(),
            mutate(|v| with_action(v, 0)["spend"]["value"] = json!(u64::MAX)),
            Malformed,
        ),
        (
            "spend value above max money".into(),
            mutate(|v| with_action(v, 0)["spend"]["value"] = json!(MAX_MONEY + 1)),
            Malformed,
        ),
        (
            "output value above max money".into(),
            mutate(|v| with_action(v, 1)["output"]["value"] = json!(MAX_MONEY + 1)),
            Malformed,
        ),
        (
            "witness".into(),
            mutate(|v| with_action(v, 0)["spend"]["witness"] = json!([0, vec![vec![0u8; 32]; 32]])),
            Policy,
        ),
        (
            "spend zip32 derivation, one index".into(),
            mutate(|v| {
                with_action(v, 0)["spend"]["zip32_derivation"] =
                    json!({"seed_fingerprint": vec![0u8; 32], "derivation_path": [1u32]})
            }),
            Policy,
        ),
        (
            "spend zip32 derivation, four indices".into(),
            mutate(|v| {
                with_action(v, 0)["spend"]["zip32_derivation"] =
                    derivation_json(&[0; 32], &[0x8000_0020; 4])
            }),
            Policy,
        ),
        (
            "spend zip32 derivation, non-hardened".into(),
            mutate(|v| {
                with_action(v, 0)["spend"]["zip32_derivation"] =
                    derivation_json(&[0; 32], &[0x8000_0020, 0x8000_0001, 9])
            }),
            Malformed,
        ),
        (
            "dummy sk".into(),
            mutate(|v| with_action(v, 0)["spend"]["dummy_sk"] = json!(vec![0; 32])),
            Policy,
        ),
        (
            "spend proprietary".into(),
            mutate(|v| with_action(v, 0)["spend"]["proprietary"] = json!({"x": [1]})),
            Policy,
        ),
        (
            "memo plaintext ciphertext".into(),
            mutate(|v| {
                with_action(v, 0)["output"]["enc_ciphertext"] = json!({"MemoPlaintext": [1, 2]})
            }),
            Policy,
        ),
        (
            "short enc ciphertext".into(),
            mutate(|v| {
                with_action(v, 0)["output"]["enc_ciphertext"] = json!({"Encrypted": vec![0u8; 579]})
            }),
            Malformed,
        ),
        (
            "long enc ciphertext".into(),
            mutate(|v| {
                with_action(v, 0)["output"]["enc_ciphertext"] = json!({"Encrypted": vec![0u8; 581]})
            }),
            Malformed,
        ),
        (
            "short out ciphertext".into(),
            mutate(|v| with_action(v, 0)["output"]["out_ciphertext"] = json!(vec![0u8; 79])),
            Malformed,
        ),
        (
            "output zip32 derivation, one index".into(),
            mutate(|v| {
                with_action(v, 0)["output"]["zip32_derivation"] =
                    json!({"seed_fingerprint": vec![0u8; 32], "derivation_path": [1u32]})
            }),
            Policy,
        ),
        (
            "output zip32 derivation, non-hardened".into(),
            mutate(|v| {
                with_action(v, 0)["output"]["zip32_derivation"] =
                    derivation_json(&[0; 32], &[0, 0x8000_0001, 0x8000_0009])
            }),
            Malformed,
        ),
        (
            "user_address over budget".into(),
            mutate(|v| {
                with_action(v, 0)["output"]["user_address"] =
                    json!("u".repeat(USER_ADDRESS_BUDGET + 1))
            }),
            Policy,
        ),
        (
            "user_address invalid utf-8".into(),
            user_address_with_invalid_utf8(),
            Malformed,
        ),
        (
            "output proprietary".into(),
            mutate(|v| with_action(v, 0)["output"]["proprietary"] = json!({"change": [1]})),
            Policy,
        ),
        (
            "negative value sum".into(),
            mutate(|v| v["ironwood"]["value_sum"][1] = json!(true)),
            Policy,
        ),
        (
            "value sum above max money".into(),
            mutate(|v| v["ironwood"]["value_sum"][0] = json!(MAX_MONEY + 1)),
            Malformed,
        ),
        (
            "note version v2".into(),
            mutate(|v| v["ironwood"]["note_version"] = json!("V2")),
            Policy,
        ),
        (
            "zkproof present".into(),
            mutate(|v| v["ironwood"]["zkproof"] = json!(vec![0u8; 4])),
            Policy,
        ),
    ];
    for (part, fields) in [
        (
            "spend",
            vec![
                "nullifier",
                "rk",
                "recipient",
                "value",
                "rho",
                "rseed",
                "fvk",
                "alpha",
            ],
        ),
        ("output", vec!["cmx", "recipient", "value", "rseed"]),
    ] {
        for field in fields {
            corpus.push((
                format!("missing {part}.{field}"),
                mutate(|v| with_action(v, 0)[part][field] = Value::Null),
                Malformed,
            ));
        }
    }
    for field in ["cv_net", "rcv"] {
        corpus.push((
            format!("missing {field}"),
            mutate(|v| with_action(v, 0)[field] = Value::Null),
            Malformed,
        ));
    }
    corpus
}

fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while value >= 128 {
        bytes.push((value & 127) as u8 | 128);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}

#[test]
fn budgets_bound_the_grammar() {
    assert_eq!(HEADER_BUDGET, 101);
    assert_eq!(TRANSPARENT_OUTPUT_BUDGET, 589);
    assert_eq!(SHIELDED_BUDGET, 109);
    assert_eq!(ACTION_BUDGET, 2015);
    assert_eq!(TRAILER_BUDGET, 89);
    assert_eq!(SECTION_BUDGET, ACTION_BUDGET);
    assert_eq!(HEADER_ACCEPTED_MAX, 44);
    assert_eq!(TRANSPARENT_OUTPUT_ACCEPTED_MAX, 552);
    assert_eq!(SHIELDED_ACCEPTED_MAX, 73);
    assert_eq!(ACTION_ACCEPTED_MAX, 1911);
    assert_eq!(TRAILER_ACCEPTED_MAX, 78);
    for (name, bytes) in accepted_corpus() {
        let scanned = scan(&bytes, WHOLE).unwrap();
        let sections = &scanned.sections;
        let transparent = scanned.transparent_outputs.len();
        assert_eq!(sections.iter().sum::<usize>(), bytes.len(), "{name}");
        // header, each transparent output, the shielded prefix, each action,
        // the trailer.
        assert_eq!(
            sections.len(),
            transparent + scanned.actions.len() + 3,
            "{name}"
        );
        assert!(
            sections[0] <= HEADER_ACCEPTED_MAX,
            "{name}: header {}",
            sections[0]
        );
        for length in &sections[1..1 + transparent] {
            assert!(
                *length <= TRANSPARENT_OUTPUT_ACCEPTED_MAX,
                "{name}: transparent output {length}"
            );
        }
        assert!(
            sections[1 + transparent] <= SHIELDED_ACCEPTED_MAX,
            "{name}: shielded {}",
            sections[1 + transparent]
        );
        for length in &sections[2 + transparent..sections.len() - 1] {
            assert!(*length <= ACTION_ACCEPTED_MAX, "{name}: action {length}");
        }
        assert!(
            sections[sections.len() - 1] <= TRAILER_ACCEPTED_MAX,
            "{name}: trailer"
        );
    }
}

#[test]
fn accepted_corpus_matches_upstream_under_every_chunking() {
    let mut unparsed_upstream = Vec::new();
    for (name, bytes) in accepted_corpus() {
        preflight(&bytes).unwrap_or_else(|e| panic!("{name}: {e:?}"));
        let scanned = assert_same_verdict(&name, &bytes).unwrap();
        assert_eq!(Some(scanned.actions.len()), scanned.actions_declared);
        assert_matches_upstream_json(&name, &scanned, &upstream_json(&bytes).unwrap());
        if !assert_matches_orchard_accessors(&name, &bytes, &scanned) {
            unparsed_upstream.push(name);
        }
    }
    // Shape-only admission leaves version policy to `validate`, so a deferred
    // anchor behind a non-v6 version passes the scanner and only upstream's
    // bundle parser refuses it.
    assert_eq!(unparsed_upstream, ["five-byte header varints"]);
}

#[test]
fn rejected_corpus_matches_wire_scan_under_every_chunking() {
    for (name, bytes, code) in rejected_corpus() {
        assert_eq!(preflight(&bytes).unwrap_err(), code, "{name}: oracle");
        assert!(assert_same_verdict(&name, &bytes).is_none(), "{name}");
    }
}

#[test]
fn every_truncation_and_trailing_byte_matches_wire_scan() {
    let bytes = fixture();
    for n in 0..bytes.len() {
        assert_eq!(preflight(&bytes[..n]).unwrap_err(), ErrorCode::Malformed);
        assert!(assert_same_verdict(&format!("length {n}"), &bytes[..n]).is_none());
    }
    let mut extra = bytes;
    extra.push(0);
    assert!(assert_same_verdict("trailing", &extra).is_none());
}

#[test]
fn every_single_byte_mutation_matches_wire_scan() {
    let bytes = fixture();
    let (mut accepted, mut rejected) = (0, 0);
    for i in 0..bytes.len() {
        for replacement in [0, 255, bytes[i] ^ 128] {
            let mut mutated = bytes.clone();
            mutated[i] = replacement;
            let expected = preflight(&mutated);
            let whole = scan(&mutated, WHOLE);
            assert_eq!(whole.as_ref().err(), expected.as_ref().err(), "byte {i}");
            for chunk in [7, 1024] {
                assert_eq!(scan(&mutated, chunk), whole, "byte {i} chunk {chunk}");
            }
            match whole {
                Ok(scanned) => {
                    accepted += 1;
                    let name = format!("byte {i} = {replacement}");
                    assert_matches_upstream_json(
                        &name,
                        &scanned,
                        &upstream_json(&mutated).unwrap(),
                    );
                }
                Err(_) => rejected += 1,
            }
        }
    }
    assert!(accepted > 1000, "{accepted}");
    assert!(rejected > 100, "{rejected}");
}

/// Sections whose verdict is not `Malformed` can exceed the longest accepted
/// encoding because the reader consumes overlong varints before deciding; the
/// budgets cover that, so the verdict matches `wire::scan`.
#[test]
fn overlong_varints_before_a_verdict_keep_wire_scan_verdicts() {
    let fixture = fixture();
    let header_len = scan(&fixture, WHOLE).unwrap().sections[0];
    let rest = &fixture[header_len..];
    let ten_byte = varint(1 << 63);
    assert_eq!(ten_byte.len(), 10);
    let wide = || {
        let mut header = b"PCZT\x02\0\0\0".to_vec();
        for _ in 0..3 {
            header.extend(varint(u64::from(u32::MAX)));
        }
        header.push(1);
        header.extend(varint(u64::from(u32::MAX)));
        header.extend(varint(u64::from(u32::MAX)));
        header.extend(varint(u64::from(u32::MAX)));
        header.push(0);
        header
    };

    let mut header = wide();
    header.extend(&ten_byte); // proprietary count: nonzero
    assert!(header.len() <= HEADER_BUDGET);
    let bytes = [header, rest.to_vec()].concat();
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Policy);
    assert_same_verdict("wide header, proprietary", &bytes);

    // An empty Sapling bundle with its anchor and `bsk`, as the Full view
    // keeps it, is the longest admitted shielded prefix: Sapling present with
    // zero spends and outputs, then (after the value sum) the anchor and
    // `bsk`. The two bytes before it close the header section: the empty
    // proprietary map and the absent transparent bundle.
    let sapling_counts = [0, 0, 1, 0, 0];
    let sapling_rest = || {
        let mut bytes = vec![1];
        bytes.extend([0; 32]);
        bytes.push(1);
        bytes.extend([9; 32]);
        bytes
    };
    let mut header = wide();
    header.extend(sapling_counts);
    header.extend(&ten_byte); // sapling value sum: nonzero
    assert!(header.len() <= HEADER_BUDGET);
    let bytes = [header, rest.to_vec()].concat();
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Policy);
    assert_same_verdict("wide header, sapling value sum", &bytes);

    let mut header = wide();
    header.extend(sapling_counts);
    header.push(0); // sapling value sum
    header.extend(sapling_rest());
    header.extend([0, 1]);
    header.extend(&ten_byte); // action count
    // The header section ends at the transparent bundle's absent tag; the
    // shielded prefix carries everything after it, and it is that section
    // whose overlong action count runs past its longest accepted encoding.
    let header_section = 8 + 3 * 5 + (1 + 5) + 5 + 5 + 1 + 1 + 1;
    assert!(header_section <= HEADER_BUDGET);
    let shielded_section = header.len() - header_section;
    assert!(shielded_section > SHIELDED_ACCEPTED_MAX && shielded_section <= SHIELDED_BUDGET);
    let bytes = [header, rest.to_vec()].concat();
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Capacity);
    assert_same_verdict("wide shielded prefix, action count", &bytes);

    // The same inside a transparent output: its proprietary count widened to
    // ten bytes is still decided within TRANSPARENT_OUTPUT_BUDGET.
    let one = build_transparent(&transparent_values(1));
    let sections = scan(&one, WHOLE).unwrap().sections;
    let end = sections[0] + sections[1];
    let mut bytes = one[..end - 1].to_vec();
    bytes.extend(&ten_byte);
    bytes.extend(&one[end..]);
    assert!(sections[1] - 1 + ten_byte.len() <= TRANSPARENT_OUTPUT_BUDGET);
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Policy);
    assert_same_verdict("wide transparent output, proprietary", &bytes);

    let mut header = wide();
    header.extend([0x80; 9]);
    header.push(0); // overlong zero
    let bytes = [header, rest.to_vec()].concat();
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Malformed);
    assert_same_verdict("wide header, overlong zero", &bytes);

    let anchored = mutate(|v| {
        v["ironwood"]["anchor"] = json!(vec![0; 32]);
        v["ironwood"]["value_sum"][0] = json!(MAX_MONEY);
        v["ironwood"]["bsk"] = json!(vec![9; 32]);
    });
    let sections = scan(&anchored, WHOLE).unwrap().sections;
    let trailer_start = anchored.len() - sections[sections.len() - 1];
    let note_version = trailer_start + 1 + 8 + 1 + 33;
    assert_eq!(anchored[note_version..note_version + 3], [1, 0, 1]);
    assert_eq!(anchored.len(), note_version + 3 + 32);
    let tail = &anchored[note_version + 1..];
    let mut bytes = anchored[..note_version].to_vec();
    bytes.extend(&ten_byte);
    bytes.extend(tail);
    assert!(bytes.len() - trailer_start > TRAILER_ACCEPTED_MAX);
    assert!(bytes.len() - trailer_start <= TRAILER_BUDGET);
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Policy);
    assert_same_verdict("wide trailer, note version", &bytes);

    let mut bytes = anchored[..note_version].to_vec();
    bytes.extend([0x81, 0]);
    bytes.extend(tail);
    assert_eq!(preflight(&bytes).unwrap_err(), ErrorCode::Malformed);
    assert_same_verdict("wide trailer, overlong one", &bytes);
}

/// Under byte-at-a-time delivery every verdict arrives before the current
/// section has consumed more than its budget.
#[test]
fn verdicts_arrive_within_the_section_budget() {
    let fixture = fixture();
    let mut checked = 0;
    let mut inputs: Vec<Vec<u8>> = rejected_corpus().into_iter().map(|(_, b, _)| b).collect();
    for i in 0..fixture.len() {
        let mut mutated = fixture.clone();
        mutated[i] = 0xff;
        inputs.push(mutated);
    }
    for bytes in inputs {
        let run = run(&bytes, 1);
        let Err(_) = run.result else { continue };
        let sections = &run.scanned.sections;
        let transparent = run
            .scanned
            .header
            .map_or(0, |header| header.transparent_outputs);
        let actions = run.scanned.actions_declared.unwrap_or(0);
        let budget = match sections.len() {
            0 => HEADER_BUDGET,
            n if n <= transparent => TRANSPARENT_OUTPUT_BUDGET,
            n if n == transparent + 1 => SHIELDED_BUDGET,
            n if n <= transparent + 1 + actions => ACTION_BUDGET,
            _ => TRAILER_BUDGET,
        };
        let pending = run.fed - sections.iter().sum::<usize>();
        assert!(pending <= budget, "{pending} > {budget}");
        checked += 1;
    }
    assert!(checked > 100, "{checked}");
}

#[test]
fn declared_length_is_the_capacity_bound_and_the_end_of_input() {
    assert_eq!(
        Scanner::new(MAX_PCZT_BYTES + 1).err(),
        Some(Error::Capacity)
    );
    assert!(Scanner::new(MAX_PCZT_BYTES).is_ok());
    for total in 0..8 {
        assert_eq!(Scanner::new(total).err(), Some(Error::Malformed));
    }

    let bytes = fixture();
    let mut extra = bytes.clone();
    extra.push(0);
    let mut scanner = Scanner::new(bytes.len()).unwrap();
    assert_eq!(
        scanner.feed(&extra).err(),
        Some(Error::State),
        "beyond the declared length"
    );

    // Declared longer than the grammar: rejected as trailing when the trailer
    // completes, without waiting for the missing byte.
    let mut scanner = Scanner::new(bytes.len() + 1).unwrap();
    let mut rest = &bytes[..];
    let verdict = loop {
        match scanner.feed(rest) {
            Ok((consumed, _)) => rest = &rest[consumed..],
            Err(error) => break error,
        }
    };
    assert_eq!(verdict, Error::Malformed);
    assert_eq!(
        scanner.feed(&[]).err(),
        Some(Error::State),
        "failure is final"
    );

    // Fewer bytes than declared so far: no verdict yet.
    let mut scanner = Scanner::new(bytes.len()).unwrap();
    let mut rest = &bytes[..bytes.len() - 1];
    while !rest.is_empty() {
        let (consumed, _) = scanner.feed(rest).unwrap();
        rest = &rest[consumed..];
    }
    assert!(!scanner.is_finished());
    let (consumed, item) = scanner.feed(&bytes[bytes.len() - 1..]).unwrap();
    assert_eq!(consumed, 1);
    assert!(matches!(item, Some(Item::Trailer(_))));
    assert!(scanner.is_finished());
    assert_eq!(scanner.feed(&[]).unwrap(), (0, None));
    assert_eq!(scanner.feed(&[0]).err(), Some(Error::State));
}
