//! Generates `get_address*.json` and `get_viewing_key.json` for the device
//! tests. Regenerate with
//!
//!     ZCASH_UPDATE_FIXTURES=1 cargo test --test fixtures_keys
//!
//! The expected strings come from `orchard` and `zcash_address`.

use orchard::keys::Scope;
use serde::Serialize;
use zcash_address::unified::{Address, Encoding, Fvk, Receiver, Ufvk};
use zcash_signer::Network;
use zcash_signer::keys::external_receiver;
use zcash_signer_tests::fixtures::{
    ALCOHOL_WOMAN, ALL_ALL, Case, Mnemonic, check, network_name, network_type, wallet,
};
use zcash_signer_tests::hex;

#[derive(Serialize)]
struct AddressParameters {
    network: &'static str,
    account: u32,
    diversifier_index: u128,
}

#[derive(Serialize)]
struct AddressResult {
    address: String,
}

fn address_case(
    mnemonic: &Mnemonic,
    name: &'static str,
    network: Network,
    account: u32,
    index: u128,
) -> Case<AddressParameters, AddressResult> {
    let bytes: [u8; 11] = index.to_le_bytes()[..11].try_into().unwrap();
    let fvk = mnemonic.wallet(network, account).fvk;
    let receiver = fvk
        .address_at(bytes, Scope::External)
        .to_raw_address_bytes();
    // What the app derives.
    assert_eq!(external_receiver(&fvk, bytes, &mut || {}), receiver);
    let address = Address::try_from_items(vec![Receiver::Orchard(receiver)]).unwrap();
    Case {
        name,
        parameters: AddressParameters {
            network: network_name(network),
            account,
            diversifier_index: index,
        },
        result: AddressResult {
            address: address.encode(&network_type(network)),
        },
    }
}

#[test]
fn test_get_address() {
    let cases = [
        ("mainnet_account_0", Network::Mainnet, 0, 0),
        ("mainnet_account_0_index_1", Network::Mainnet, 0, 1),
        ("mainnet_account_1", Network::Mainnet, 1, 0),
        ("testnet_account_0", Network::Testnet, 0, 0),
        ("mainnet_largest_index", Network::Mainnet, 0, (1 << 88) - 1),
    ];
    let cases = cases
        .into_iter()
        .map(|(name, network, account, index)| {
            address_case(&ALL_ALL, name, network, account, index)
        })
        .collect();
    check("get_address.json", &ALL_ALL, cases);
}

/// The same request with another seed shows another address.
#[test]
fn test_get_address_other_seed() {
    let case = address_case(&ALCOHOL_WOMAN, "other_seed", Network::Mainnet, 0, 0);
    check("get_address.other_seed.json", &ALCOHOL_WOMAN, vec![case]);
}

#[derive(Serialize)]
struct ViewingKeyParameters {
    network: &'static str,
    account: u32,
    include_seed_fingerprint: bool,
}

#[derive(Serialize)]
struct ViewingKeyResult {
    key: String,
    seed_fingerprint: Option<String>,
}

#[test]
fn test_get_viewing_key() {
    let cases = [
        ("mainnet", Network::Mainnet, false),
        ("mainnet_seed_fingerprint", Network::Mainnet, true),
        ("testnet", Network::Testnet, false),
        ("testnet_seed_fingerprint", Network::Testnet, true),
    ];
    let cases = cases
        .into_iter()
        .map(|(name, network, include_seed_fingerprint)| {
            let wallet = wallet(network, 0);
            let key = Ufvk::try_from_items(vec![Fvk::Orchard(wallet.fvk.to_bytes())]).unwrap();
            Case {
                name,
                parameters: ViewingKeyParameters {
                    network: network_name(network),
                    account: 0,
                    include_seed_fingerprint,
                },
                result: ViewingKeyResult {
                    key: key.encode(&network_type(network)),
                    seed_fingerprint: include_seed_fingerprint
                        .then(|| hex(&wallet.seed_fingerprint)),
                },
            }
        })
        .collect();
    check("get_viewing_key.json", &ALL_ALL, cases);
}
