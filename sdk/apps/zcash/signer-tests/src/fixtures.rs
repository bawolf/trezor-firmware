//! The device tests' fixtures in `sdk/apps/zcash/tests/fixtures`.

use std::path::PathBuf;

use serde::Serialize;
use zcash_protocol::consensus::NetworkType;
use zcash_signer::Network;

use crate::{Wallet, unhex};

/// A test wallet: its BIP-39 mnemonic, and its seed with an empty passphrase.
pub struct Mnemonic {
    pub words: &'static str,
    seed: &'static str,
}

impl Mnemonic {
    pub fn wallet(&self, network: Network, account: u32) -> Wallet {
        Wallet::from_seed(&unhex(self.seed), network, account)
    }
}

/// The device tests' default wallet.
pub const ALL_ALL: Mnemonic = Mnemonic {
    words: "all all all all all all all all all all all all",
    seed: "c76c4ac4f4e4a00d6b274d5c39c700bb4a7ddc04fbc6f78e85ca75007b5b495f\
           74a9043eeb77bdd53aa6fc3a0e31462270316fa04b8c19114c8798706cd02ac8",
};

/// Another wallet, for a vector that must change with the seed.
pub const ALCOHOL_WOMAN: Mnemonic = Mnemonic {
    words: "alcohol woman abuse must during monitor noble actual mixed trade anger aisle",
    seed: "1ebf38d0b1fc10ac12059141276c1b8b7a410ba43d04bbe9f3a371d884a30440\
           0b6a39fda34e5b282a3717663fb337954df3dadf802a4cba3d008d5e2988f70a",
};

/// A wallet of [`ALL_ALL`].
pub fn wallet(network: Network, account: u32) -> Wallet {
    ALL_ALL.wallet(network, account)
}

pub fn network_name(network: Network) -> &'static str {
    match network {
        Network::Mainnet => "mainnet",
        Network::Testnet => "testnet",
    }
}

pub fn network_type(network: Network) -> NetworkType {
    match network {
        Network::Mainnet => NetworkType::Main,
        Network::Testnet => NetworkType::Test,
    }
}

#[derive(Serialize)]
struct Setup {
    mnemonic: &'static str,
    passphrase: &'static str,
}

#[derive(Serialize)]
pub struct Case<P, R> {
    pub name: &'static str,
    pub parameters: P,
    pub result: R,
}

#[derive(Serialize)]
struct Fixture<P, R> {
    setup: Setup,
    tests: Vec<Case<P, R>>,
}

/// Compares `cases` for the wallet of `mnemonic` with the checked-in fixture
/// `name`, or rewrites it when `ZCASH_UPDATE_FIXTURES=1`.
pub fn check<P: Serialize, R: Serialize>(name: &str, mnemonic: &Mnemonic, cases: Vec<Case<P, R>>) {
    let fixture = Fixture {
        setup: Setup {
            mnemonic: mnemonic.words,
            passphrase: "",
        },
        tests: cases,
    };
    let json = serde_json::to_string_pretty(&fixture).unwrap() + "\n";
    let path: PathBuf = [env!("CARGO_MANIFEST_DIR"), "..", "tests", "fixtures", name]
        .iter()
        .collect();
    if std::env::var("ZCASH_UPDATE_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&path, json).unwrap();
    } else {
        let checked_in = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            checked_in == json,
            "{name} is out of date; regenerate it with ZCASH_UPDATE_FIXTURES=1"
        );
    }
}
