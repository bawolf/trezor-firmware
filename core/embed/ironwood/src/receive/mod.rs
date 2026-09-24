//! Orchard receiver and viewing-key derivation.
//!
//! The implementation is a current Rust port of the key primitives from
//! Trezor's historical `zcash-orchard` branch. It intentionally exposes only
//! typed product operations and no generic cryptographic building blocks.

mod ff1;
mod generators;
mod keys;
mod sinsemilla;

const MAINNET_COIN_TYPE: u32 = 133;
const TESTNET_COIN_TYPE: u32 = 1;

pub use keys::{derive_external_receiver, derive_full_viewing_key};

/// Network selected and validated by the device application.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    const fn coin_type(self) -> u32 {
        match self {
            Self::Mainnet => MAINNET_COIN_TYPE,
            Self::Testnet => TESTNET_COIN_TYPE,
        }
    }
}

impl TryFrom<u32> for Network {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        match value {
            0 => Ok(Self::Mainnet),
            1 => Ok(Self::Testnet),
            _ => Err(Error::InvalidNetwork),
        }
    }
}

/// Receiver derivation failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    InvalidNetwork,
    InvalidSeedLength,
    InvalidAccount,
    InvalidKey,
}

pub type Result<T> = core::result::Result<T, Error>;
