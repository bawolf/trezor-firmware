//! The transaction-level fields, and the policy the host chooses.

use serde_json::{Value, json};
use zcash_protocol::consensus::BranchId;
use zcash_protocol::value::MAX_MONEY;
use zcash_signer::Error::{Malformed, Policy};
use zcash_signer::{MAX_ACCOUNT, Network};
use zcash_signer_tests::*;

/// Edits of the header. A dummy spend's signature covers every field the
/// sighash does, so only an edit outside it can be accepted.
const GLOBAL: &[(&str, Edit, Outcome)] = &[
    (
        "implicit zero lock time",
        |v| v["global"]["fallback_lock_time"] = Value::Null,
        Ok(()),
    ),
    (
        "lock time",
        |v| v["global"]["fallback_lock_time"] = json!(1),
        Err(Policy),
    ),
    (
        "mainnet coin type",
        |v| v["global"]["coin_type"] = json!(133),
        Err(Policy),
    ),
    ("v5", |v| v["global"]["tx_version"] = json!(5), Err(Policy)),
    (
        "version group",
        |v| v["global"]["version_group_id"] = json!(0),
        Err(Policy),
    ),
    (
        "sprout branch",
        |v| v["global"]["consensus_branch_id"] = json!(0),
        Err(Policy),
    ),
    (
        "NU6.2 branch",
        |v| v["global"]["consensus_branch_id"] = json!(u32::from(BranchId::Nu6_2)),
        Err(Policy),
    ),
    (
        "unknown branch",
        |v| v["global"]["consensus_branch_id"] = json!(0xdead_beef_u32),
        Err(Policy),
    ),
    (
        "modifiable",
        |v| v["global"]["tx_modifiable"] = json!(128),
        Err(Policy),
    ),
    (
        "proprietary",
        |v| v["global"]["proprietary"] = json!({ "x": [1] }),
        Err(Policy),
    ),
    ("five-byte varints", five_byte_varints, Err(Policy)),
];

/// Every `u32` of the header at its longest encoding.
fn five_byte_varints(value: &mut Value) {
    for field in [
        "tx_version",
        "version_group_id",
        "consensus_branch_id",
        "fallback_lock_time",
        "expiry_height",
        "coin_type",
    ] {
        value["global"][field] = json!(u32::MAX);
    }
}

#[test]
fn test_global_fields() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    for (name, edit, expected) in GLOBAL {
        assert_outcome(&wallet, name, &mutate(&bytes, edit), *expected);
    }
}

#[test]
fn test_expiry_window() {
    let wallet = Wallet::testnet();
    let cases = [
        (HEIGHT, Err(Policy)),
        (HEIGHT + 1, Ok(())),
        (HEIGHT + EXPIRY_WINDOW, Ok(())),
        (HEIGHT + EXPIRY_WINDOW + 1, Err(Policy)),
    ];
    for (expiry, expected) in cases {
        let tx = Tx {
            expiry,
            ..Tx::simple()
        };
        let name = format!("expiry {expiry}");
        assert_outcome(&wallet, &name, &build(&wallet, &tx).bytes, expected);
    }
}

#[test]
fn test_network_must_match() {
    for (wallet, other) in [
        (Wallet::mainnet(), Network::Testnet),
        (Wallet::testnet(), Network::Mainnet),
    ] {
        let bytes = build(&wallet, &Tx::simple()).bytes;
        assert_outcome(&wallet, "own network", &bytes, Ok(()));
        let policy =
            zcash_signer::Policy::new(other, wallet.account, HEIGHT, MAX_FEE, EXPIRY_WINDOW)
                .unwrap();
        let mut session = wallet.session_with(policy, 0);
        assert_eq!(
            stream(&mut session, &wallet, &bytes, bytes.len(), 1024).err(),
            Some(Policy)
        );
    }
}

#[test]
fn test_policy_limits() {
    let policy = |network, account, height, max_fee, window| {
        zcash_signer::Policy::new(network, account, height, max_fee, window).map(|_| ())
    };
    let cases = [
        (
            "account 0",
            policy(Network::Mainnet, 0, HEIGHT, MAX_FEE, EXPIRY_WINDOW),
            Ok(()),
        ),
        (
            "highest account",
            policy(
                Network::Mainnet,
                MAX_ACCOUNT,
                HEIGHT,
                MAX_FEE,
                EXPIRY_WINDOW,
            ),
            Ok(()),
        ),
        (
            "account past ZIP 32",
            policy(
                Network::Mainnet,
                MAX_ACCOUNT + 1,
                HEIGHT,
                MAX_FEE,
                EXPIRY_WINDOW,
            ),
            Err(Policy),
        ),
        (
            "mainnet NU6.3 activation",
            policy(Network::Mainnet, 0, 3_428_143, MAX_FEE, EXPIRY_WINDOW),
            Ok(()),
        ),
        (
            "mainnet before NU6.3",
            policy(Network::Mainnet, 0, 3_428_142, MAX_FEE, EXPIRY_WINDOW),
            Err(Policy),
        ),
        (
            "testnet before NU6.3",
            policy(Network::Testnet, 0, 4_133_999, MAX_FEE, EXPIRY_WINDOW),
            Err(Policy),
        ),
        (
            "fee limit at MAX_MONEY",
            policy(Network::Mainnet, 0, HEIGHT, MAX_MONEY, EXPIRY_WINDOW),
            Ok(()),
        ),
        (
            "fee limit past MAX_MONEY",
            policy(Network::Mainnet, 0, HEIGHT, MAX_MONEY + 1, EXPIRY_WINDOW),
            Err(Policy),
        ),
        (
            "empty expiry window",
            policy(Network::Mainnet, 0, HEIGHT, MAX_FEE, 0),
            Err(Policy),
        ),
        (
            "expiry past u32",
            policy(Network::Mainnet, 0, u32::MAX, MAX_FEE, 1),
            Err(Policy),
        ),
    ];
    for (name, actual, expected) in cases {
        assert_eq!(actual, expected, "{name}");
    }
}

#[test]
fn test_noncanonical_encodings() {
    let wallet = Wallet::testnet();
    let bytes = build(&wallet, &Tx::simple()).bytes;
    let mut v1 = bytes.clone();
    v1[4] = 1;
    // `tx_version` 6 as two bytes.
    let overlong = [&bytes[..8], &[0x86, 0], &bytes[9..]].concat();
    for (name, mutated) in [("v1", v1), ("overlong varint", overlong)] {
        assert_outcome(&wallet, name, &mutated, Err(Malformed));
    }
}

/// A header at its longest, with a varint read to its tenth byte before the
/// verdict.
#[test]
fn test_ten_byte_varint_in_header() {
    let wallet = Wallet::testnet();
    let bytes = mutate(&build(&wallet, &Tx::simple()).bytes, five_byte_varints);
    let other = mutate(&bytes, |v| v["global"]["proprietary"] = json!({ "x": [1] }));
    let count = offset_of_difference(&bytes, &other);
    for (name, count_bytes, expected) in [
        ("nonzero", varint(1 << 63), Policy),
        (
            "overlong zero",
            [[0x80; 9].as_slice(), &[0]].concat(),
            Malformed,
        ),
    ] {
        let wide = [&bytes[..count], &count_bytes, &bytes[count + 1..]].concat();
        assert_outcome(&wallet, name, &wide, Err(expected));
    }
}
