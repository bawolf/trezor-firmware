//! Generates the `sign_pczt*.json` fixtures for the device tests. Regenerate
//! with
//!
//!     ZCASH_UPDATE_FIXTURES=1 cargo test --test fixtures_sign
//!
//! Each vector's expected result is derived from how it was built, with
//! addresses from `zcash_address` and `zcash_transparent`, then checked
//! against a Session under the app's policy: a vector the app would show or
//! sign differently cannot be written.

use serde::Serialize;
use transparent::address::{Script, TransparentAddress};
use zcash_address::unified::{Address, Container, Encoding, Receiver};
use zcash_protocol::memo::MemoBytes;
use zcash_signer::{Error, MAX_MEMO_TEXT_BYTES, Memo, Network, TransparentKind};
use zcash_signer_tests::fixtures::{ALL_ALL, Case, check, network_name, network_type, wallet};
use zcash_signer_tests::*;

const REJECTED: &str = "Zcash PCZT rejected";
const MISMATCH: &str = "Unified address does not match the Orchard receiver";

#[derive(Serialize)]
struct Parameters {
    network: &'static str,
    account: u32,
    host_reference_height: u32,
    pczt: String,
}

#[derive(Default, Serialize)]
struct Expected {
    #[serde(skip_serializing_if = "Option::is_none")]
    real_spend_actions: Option<Vec<usize>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    payments: Option<Vec<Payment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    fee: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transparent_outputs: Option<Vec<Payment>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memo: Option<Shown>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<&'static str>,
}

#[derive(Debug, PartialEq, Serialize)]
struct Payment {
    address: String,
    value: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Shown {
    Text(String),
    /// The BLAKE2b-256 of the 512 memo bytes, in hex.
    Digest(String),
}

struct Vector {
    name: &'static str,
    network: Network,
    tx: Tx,
    transparent: Vec<(u64, TransparentAddress)>,
    /// A derivation claim on the real spend.
    claim: Option<([u8; 32], [u32; 3])>,
    /// Shown for every payment.
    memo: Option<Shown>,
    error: Option<&'static str>,
}

impl Vector {
    fn new(name: &'static str, network: Network, tx: Tx) -> Self {
        Self {
            name,
            network,
            tx,
            transparent: Vec::new(),
            claim: None,
            memo: None,
            error: None,
        }
    }
}

/// A note paying `outputs`, unpadded, with the ZIP-317 fee.
fn paying(outputs: Vec<Output>) -> Tx {
    let fee = zip317_fee(outputs.len());
    Tx {
        padded: false,
        ..Tx::pay(outputs, Vec::new(), fee)
    }
}

/// `count` outputs of 100_000 more each, alternately payments to distinct
/// addresses and change.
fn outputs(count: usize) -> Vec<Output> {
    (0..count)
        .map(|i| {
            let value = 100_000 * (i as u64 + 1);
            if i % 2 == 0 {
                Output {
                    to: To::Other(i as u32),
                    ..Output::payment(value)
                }
            } else {
                Output::change(value)
            }
        })
        .collect()
}

fn unified(network: Network, receivers: Vec<Receiver>) -> String {
    Address::try_from_items(receivers)
        .unwrap()
        .encode(&network_type(network))
}

fn orchard_receiver(network: Network, to: To) -> [u8; 43] {
    wallet(network, 0).address(to).to_raw_address_bytes()
}

/// The unified address of the other wallet's `index` with a transparent
/// receiver, as wallets hand out.
fn with_transparent(network: Network, index: u32) -> String {
    unified(
        network,
        vec![
            Receiver::Orchard(orchard_receiver(network, To::Other(index))),
            Receiver::P2pkh([0x10 + index as u8; 20]),
        ],
    )
}

/// Two payments with `memo`, shown as text or as its digest.
fn memo_vector(name: &'static str, memo: &[u8], as_text: bool) -> Vector {
    let memo = MemoBytes::from_bytes(memo).unwrap();
    let shown = if as_text {
        Shown::Text(String::from_utf8(memo.as_slice().to_vec()).unwrap())
    } else {
        Shown::Digest(hex(&memo_digest(&memo)))
    };
    let payment = |index: u32, value| Output {
        to: To::Other(index),
        memo: memo.clone(),
        ..Output::payment(value)
    };
    let tx = paying(vec![
        payment(0, 100_000),
        payment(1, 200_000),
        Output::change(300_000),
    ]);
    Vector {
        memo: Some(shown),
        ..Vector::new(name, Network::Mainnet, tx)
    }
}

fn transparent_vector(name: &'static str, network: Network, count: usize) -> Vector {
    let transparent: Vec<_> = (0..count)
        .map(|i| {
            let hash = [0xa0 + i as u8; 20];
            let address = if i % 2 == 0 {
                TransparentAddress::PublicKeyHash(hash)
            } else {
                TransparentAddress::ScriptHash(hash)
            };
            (100_000 + i as u64 * 10_000, address)
        })
        .collect();
    let scripts = transparent
        .iter()
        .map(|(value, address)| (*value, script(address)))
        .collect();
    let outputs = vec![Output::payment(100_000), Output::change(390_000)];
    let tx = Tx {
        padded: false,
        ..Tx::pay(outputs, scripts, zip317_fee(2 + count))
    };
    Vector {
        transparent,
        ..Vector::new(name, network, tx)
    }
}

/// The `scriptPubKey` that `zcash_transparent` pays `address` with.
fn script(address: &TransparentAddress) -> Vec<u8> {
    let mut bytes = Vec::new();
    Script::from(&address.script()).write(&mut bytes).unwrap();
    // Without the length prefix.
    bytes[1..].to_vec()
}

/// Builds each vector, derives its expected result from how it was built,
/// checks that a Session agrees, and compares or rewrites the fixture.
fn generate(file: &str, vectors: Vec<Vector>) {
    let cases = vectors
        .into_iter()
        .map(|vector| {
            let wallet = wallet(vector.network, 0);
            let built = build(&wallet, &vector.tx);
            let bytes = match vector.claim {
                Some((fingerprint, path)) => mutate(&built.bytes, |v| {
                    let spend = &mut v["ironwood"]["actions"][built.spends[0]]["spend"];
                    spend["zip32_derivation"] = derivation_json(&fingerprint, &path);
                }),
                None => built.bytes.clone(),
            };
            let result = match vector.error {
                Some(error) => {
                    assert_refused(&wallet, &bytes, error);
                    Expected {
                        error: Some(error),
                        ..Expected::default()
                    }
                }
                None => {
                    let expected = expected(&vector, &built, &bytes);
                    assert_shown(&wallet, &bytes, &vector, &expected);
                    expected
                }
            };
            Case {
                name: vector.name,
                parameters: Parameters {
                    network: network_name(vector.network),
                    account: 0,
                    host_reference_height: HEIGHT,
                    pczt: hex(&bytes),
                },
                result,
            }
        })
        .collect();
    check(file, &ALL_ALL, cases);
}

fn expected(vector: &Vector, built: &Built, bytes: &[u8]) -> Expected {
    let network = vector.network;
    let mut payments: Vec<(usize, Payment)> = vector
        .tx
        .outputs
        .iter()
        .zip(&built.outputs)
        .filter(|(output, _)| matches!(output.to, To::Other(_)))
        .map(|(output, index)| {
            let address = output.user_address.clone().unwrap_or_else(|| {
                let receiver = orchard_receiver(network, output.to);
                unified(network, vec![Receiver::Orchard(receiver)])
            });
            let value = output.value;
            (*index, Payment { address, value })
        })
        .collect();
    // Shown in action order.
    payments.sort_by_key(|(index, _)| *index);
    let payments: Vec<Payment> = payments.into_iter().map(|(_, payment)| payment).collect();
    let outputs: u64 = vector.tx.outputs.iter().map(|output| output.value).sum();
    let transparent: u64 = vector.transparent.iter().map(|(value, _)| value).sum();
    let transparent_outputs = vector
        .transparent
        .iter()
        .map(|(value, address)| Payment {
            address: address.to_zcash_address(network_type(network)).encode(),
            value: *value,
        })
        .collect::<Vec<_>>();
    Expected {
        real_spend_actions: Some(real_spends(bytes)),
        payments: Some(payments),
        fee: Some(vector.tx.notes.iter().sum::<u64>() - outputs - transparent),
        transparent_outputs: (!transparent_outputs.is_empty()).then_some(transparent_outputs),
        memo: vector.memo.clone(),
        error: None,
    }
}

/// What a Session shows for `bytes` is `expected`, and it signs the real
/// spends.
fn assert_shown(wallet: &Wallet, bytes: &[u8], vector: &Vector, expected: &Expected) {
    let name = vector.name;
    let network = vector.network;
    let events = review(wallet, bytes, 1024).unwrap();
    let payments: Vec<Payment> = events
        .payments
        .iter()
        .map(|(output, user_address)| {
            let orchard = Receiver::Orchard(output.receiver);
            let address = match user_address {
                Some(address) => {
                    assert!(names(address, &orchard), "{name}");
                    address.clone()
                }
                None => unified(network, vec![orchard]),
            };
            let value = output.value;
            Payment { address, value }
        })
        .collect();
    let payment_total: u64 = payments.iter().map(|payment| payment.value).sum();
    assert_eq!(Some(payments), expected.payments, "{name}");
    let summary = events.review().summary();
    assert_eq!(summary.payment_total, payment_total, "{name}");
    assert_eq!(Some(summary.fee), expected.fee, "{name}");
    let transparent: Vec<Payment> = events
        .transparent
        .iter()
        .map(|output| {
            let address = match output.kind {
                TransparentKind::P2pkh => TransparentAddress::PublicKeyHash(output.hash),
                TransparentKind::P2sh => TransparentAddress::ScriptHash(output.hash),
            };
            Payment {
                address: address.to_zcash_address(network_type(network)).encode(),
                value: output.value,
            }
        })
        .collect();
    let expected_transparent = expected.transparent_outputs.as_deref().unwrap_or_default();
    assert_eq!(transparent, expected_transparent, "{name}");
    for (output, _) in &events.payments {
        let memo = match &output.memo {
            Memo::Empty => None,
            Memo::Text(text) => Some(Shown::Text(text.as_str().into())),
            Memo::Digest(digest) => Some(Shown::Digest(hex(digest))),
        };
        assert_eq!(memo, expected.memo, "{name}");
    }
    assert_signatures_apply(bytes, &sign(wallet, bytes).unwrap());
}

/// Whether the unified `address` has `receiver`.
fn names(address: &str, receiver: &Receiver) -> bool {
    let (_, address) = Address::decode(address).unwrap();
    address.items().contains(receiver)
}

/// The Session refuses the PCZT, or the app refuses what it shows.
fn assert_refused(wallet: &Wallet, bytes: &[u8], error: &str) {
    let reviewed = review(wallet, bytes, 1024);
    match error {
        REJECTED => assert_eq!(reviewed.err(), Some(Error::Policy)),
        MISMATCH => {
            let events = reviewed.unwrap();
            let (output, user_address) = &events.payments[0];
            let address = user_address.as_deref().unwrap();
            assert!(!names(address, &Receiver::Orchard(output.receiver)));
        }
        _ => unreachable!("{error}"),
    }
}

fn two_actions() -> Tx {
    paying(vec![Output::payment(100_000), Output::change(890_000)])
}

#[test]
fn test_sign_pczt() {
    let own = wallet(Network::Mainnet, 0);
    // What `zcash_client_backend` hands over: change without an OVK, and the
    // recipient's unified address on the payment.
    let wallet_view = paying(vec![
        Output {
            user_address: Some(with_transparent(Network::Mainnet, 0)),
            ..Output::payment(100_000)
        },
        Output {
            ovk: None,
            ..Output::change(890_000)
        },
    ]);
    let vectors = vec![
        Vector::new("2_actions", Network::Mainnet, two_actions()),
        Vector::new("8_actions", Network::Mainnet, paying(outputs(8))),
        Vector::new("16_actions", Network::Mainnet, paying(outputs(16))),
        Vector::new("32_actions", Network::Mainnet, paying(outputs(32))),
        Vector::new(
            "view_full",
            Network::Mainnet,
            Tx {
                view: View::Full,
                ..two_actions()
            },
        ),
        Vector::new(
            "view_wallet",
            Network::Mainnet,
            Tx {
                view: View::Full,
                ..wallet_view
            },
        ),
        Vector {
            claim: Some((own.seed_fingerprint, own.account_path())),
            ..Vector::new("zip32_derivation_own", Network::Mainnet, two_actions())
        },
    ];
    generate("sign_pczt.json", vectors);
}

#[test]
fn test_sign_pczt_memos() {
    let limit = "Pay to the order of the bearer. ".repeat(8);
    assert_eq!(limit.len(), MAX_MEMO_TEXT_BYTES);
    let binary: Vec<u8> = (0..=255).chain(0..=255).collect();
    let vectors = vec![
        memo_vector("memo_short", b"Thanks for lunch!", true),
        memo_vector("memo_utf8", "Zodl ✓ café ☕ — thanks!".as_bytes(), true),
        memo_vector("memo_at_limit", limit.as_bytes(), true),
        memo_vector("memo_over_limit", format!("{limit}!").as_bytes(), false),
        memo_vector("memo_binary", &[&[0xff][..], &binary[1..]].concat(), false),
        memo_vector(
            "memo_supplementary_plane",
            "pay me \u{1F600}".as_bytes(),
            false,
        ),
        memo_vector(
            "memo_bidi_override",
            "send 1 ZEC to \u{202E}bob".as_bytes(),
            false,
        ),
    ];
    generate("sign_pczt.memos.json", vectors);
}

#[test]
fn test_sign_pczt_transparent() {
    let vectors = vec![
        transparent_vector("1_transparent_output", Network::Mainnet, 1),
        transparent_vector("2_transparent_outputs", Network::Mainnet, 2),
        transparent_vector("4_transparent_outputs", Network::Mainnet, 4),
        transparent_vector("2_transparent_outputs_testnet", Network::Testnet, 2),
    ];
    generate("sign_pczt.transparent.json", vectors);
}

/// A payment of 100_000 more per index to the other wallet's `index`, named
/// by `user_address`.
fn named_payment(index: u32, user_address: String) -> Output {
    Output {
        to: To::Other(index),
        user_address: Some(user_address),
        ..Output::payment(100_000 * (u64::from(index) + 1))
    }
}

fn change() -> Output {
    Output::change(300_000)
}

#[test]
fn test_sign_pczt_user_address() {
    let network = Network::Mainnet;
    let receiver = orchard_receiver(network, To::Other(0));
    let sapling = sapling::zip32::ExtendedSpendingKey::master(&[8; 32])
        .default_address()
        .1;
    let three = unified(
        network,
        vec![
            Receiver::Orchard(receiver),
            Receiver::Sapling(sapling.to_bytes()),
            Receiver::P2pkh([0x10; 20]),
        ],
    );
    // An unknown receiver padded to bring the address to 512 bytes.
    let longest = (0..512)
        .map(|length| {
            let unknown = Receiver::Unknown {
                typecode: 0x10,
                data: vec![0x42; length],
            };
            unified(network, vec![Receiver::Orchard(receiver), unknown])
        })
        .find(|address| address.len() == 512)
        .unwrap();
    let vectors = vec![
        Vector::new(
            "orchard_and_transparent",
            network,
            paying(vec![
                named_payment(0, with_transparent(network, 0)),
                named_payment(1, with_transparent(network, 1)),
                change(),
            ]),
        ),
        Vector::new(
            "three_receivers",
            network,
            paying(vec![named_payment(0, three), change()]),
        ),
        Vector::new(
            "user_address_512_bytes",
            network,
            paying(vec![named_payment(0, longest), change()]),
        ),
    ];
    generate("sign_pczt.user_address.json", vectors);
}

/// PCZTs the app refuses: a unified address that names another payment's
/// receiver, and derivation claims of another seed or account.
#[test]
fn test_sign_pczt_error() {
    let network = Network::Mainnet;
    let swapped = paying(vec![
        named_payment(0, with_transparent(network, 1)),
        named_payment(1, with_transparent(network, 0)),
        change(),
    ]);
    let own = wallet(network, 0);
    let other_seed = zip32::fingerprint::SeedFingerprint::from_seed(&[8; 32])
        .unwrap()
        .to_bytes();
    let mut other_account = own.account_path();
    other_account[2] += 1;
    let claims = [
        ("claims_other_seed", (other_seed, own.account_path())),
        (
            "claims_other_account",
            (own.seed_fingerprint, other_account),
        ),
    ];
    let mut vectors = vec![Vector {
        error: Some(MISMATCH),
        ..Vector::new("swapped_user_addresses", network, swapped)
    }];
    vectors.extend(claims.into_iter().map(|(name, claim)| Vector {
        claim: Some(claim),
        error: Some(REJECTED),
        ..Vector::new(name, network, two_actions())
    }));
    generate("sign_pczt_error.json", vectors);
}
