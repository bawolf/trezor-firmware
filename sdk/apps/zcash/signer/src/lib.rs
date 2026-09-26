//! Verification and signing of Zcash PCZTs that spend from the Ironwood pool
//! (NU6.3), and Orchard receiver derivation.
//!
//! A [`Session`] reads the PCZT in chunks, keeps at most one action in memory
//! and reports each payment for review. It signs only after the user approved
//! the totals it computed. The host chooses the network, account and
//! reference height; the fee and expiry limits are the app's.

#![cfg_attr(not(test), no_std)]
#![forbid(unsafe_code)]
#![cfg_attr(
    not(test),
    deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)
)]

extern crate alloc;

mod error;
mod ff1;
mod hedge;
pub mod keys;
mod limits;
mod memo;
mod prewarm;
mod scanner;
mod session;
mod sighash;

use alloc::vec::Vec;

use zcash_protocol::consensus::{
    BlockHeight, BranchId, MAIN_NETWORK, NetworkConstants, Parameters, TEST_NETWORK,
};
use zcash_protocol::value::MAX_MONEY;
use zeroize::{Zeroize, ZeroizeOnDrop};

pub use crate::error::{Error, Result};
pub use crate::hedge::Hedged;
pub use crate::limits::{MAX_ACTIONS, MAX_PCZT_BYTES};
pub use crate::memo::{MAX_MEMO_TEXT_BYTES, Memo, MemoText};
pub use crate::prewarm::prewarm;
pub use crate::session::{Event, Session, SignatureRecord, Signatures};

use crate::error::ensure;

const ZIP32_HARDENED: u32 = 1 << 31;
const ZIP32_PURPOSE: u32 = 32;

/// Highest ZIP-32 account index.
pub const MAX_ACCOUNT: u32 = ZIP32_HARDENED - 1;

/// The network a request signs for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    Mainnet,
    Testnet,
}

impl Network {
    /// The SLIP-44 coin type of the network's ZIP-32 account paths.
    pub fn coin_type(self) -> u32 {
        match self {
            Self::Mainnet => MAIN_NETWORK.network_type().coin_type(),
            Self::Testnet => TEST_NETWORK.network_type().coin_type(),
        }
    }

    fn branch_for_height(self, height: u32) -> BranchId {
        let height = BlockHeight::from_u32(height);
        match self {
            Self::Mainnet => BranchId::for_height(&MAIN_NETWORK, height),
            Self::Testnet => BranchId::for_height(&TEST_NETWORK, height),
        }
    }
}

/// What a session may sign: the host's choice of network, account and
/// reference height, and the app's fee and expiry limits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    network: Network,
    account: u32,
    /// Unverified; bounds the expiry height and selects the consensus branch.
    reference_height: u32,
    max_fee: u64,
    /// Blocks after `reference_height` within which the transaction expires.
    expiry_window: u32,
}

impl Policy {
    /// Refuses an account above [`MAX_ACCOUNT`], a reference height before
    /// NU6.3 activation, and limits the money range cannot hold.
    pub fn new(
        network: Network,
        account: u32,
        reference_height: u32,
        max_fee: u64,
        expiry_window: u32,
    ) -> Result<Self> {
        ensure(account <= MAX_ACCOUNT, Error::Policy)?;
        ensure(max_fee <= MAX_MONEY && expiry_window > 0, Error::Policy)?;
        reference_height
            .checked_add(expiry_window)
            .ok_or(Error::Policy)?;
        ensure(
            network.branch_for_height(reference_height) == BranchId::Nu6_3,
            Error::Policy,
        )?;
        Ok(Self {
            network,
            account,
            reference_height,
            max_fee,
            expiry_window,
        })
    }

    /// The account path `m/32'/coin_type'/account'`.
    fn account_path(&self) -> [u32; 3] {
        [
            ZIP32_PURPOSE | ZIP32_HARDENED,
            self.network.coin_type() | ZIP32_HARDENED,
            self.account | ZIP32_HARDENED,
        ]
    }
}

/// Whether an output pays someone else or returns change to the account.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OutputKind {
    Payment,
    /// Change to one of the account's internal addresses.
    InternalChange,
}

/// A verified Ironwood output with a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReviewedOutput {
    pub action_index: usize,
    /// The raw Orchard receiver.
    pub receiver: [u8; 43],
    pub value: u64,
    pub kind: OutputKind,
    /// Decrypted from the signed ciphertext.
    pub memo: Memo,
}

/// The `scriptPubKey` a transparent output pays, and so its address prefix.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransparentKind {
    P2pkh,
    P2sh,
}

/// A transparent output. It is always a payment: the device has no
/// transparent keys.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TransparentOutput {
    /// Position in the transparent bundle.
    pub index: usize,
    pub kind: TransparentKind,
    /// The hash in `scriptPubKey`.
    pub hash: [u8; 20],
    pub value: u64,
}

/// The totals and outputs of a verified PCZT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Summary {
    pub expiry_height: u32,
    pub input_total: u64,
    pub payment_total: u64,
    pub change_total: u64,
    /// The sum of `transparent_outputs`: the public part of the payment.
    pub transparent_total: u64,
    pub fee: u64,
    /// Zero-value outputs, which are not shown.
    pub padding_outputs: usize,
    pub outputs: Vec<ReviewedOutput>,
    pub transparent_outputs: Vec<TransparentOutput>,
}

/// Binds an approval to one verified request of one session.
#[derive(PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct Token {
    session_id: [u8; 32],
    counter: u64,
    digest: [u8; 32],
}

impl core::fmt::Debug for Token {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Token { <redacted> }")
    }
}

/// A verified PCZT awaiting the user's approval of its summary.
pub struct Review {
    token: Token,
    summary: Summary,
}

impl core::fmt::Debug for Review {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Review { <redacted> }")
    }
}

impl Review {
    pub fn token(&self) -> &Token {
        &self.token
    }

    pub fn summary(&self) -> &Summary {
        &self.summary
    }
}

/// `total + value`, refusing a sum above `MAX_MONEY`.
fn add_amount(total: u64, value: u64) -> Result<u64> {
    total
        .checked_add(value)
        .filter(|sum| *sum <= MAX_MONEY)
        .ok_or(Error::Amount)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NU6.3 is active at this height on both networks.
    const HEIGHT: u32 = 10_000_000;

    #[test]
    fn test_coin_types() {
        assert_eq!(Network::Mainnet.coin_type(), 133);
        assert_eq!(Network::Testnet.coin_type(), 1);
    }

    #[test]
    fn test_policy() {
        let policy = Policy::new(Network::Mainnet, 7, HEIGHT, 1_000_000, 100).unwrap();
        assert_eq!(
            policy.account_path(),
            [
                32 | ZIP32_HARDENED,
                133 | ZIP32_HARDENED,
                7 | ZIP32_HARDENED
            ]
        );
        assert!(Policy::new(Network::Testnet, MAX_ACCOUNT, HEIGHT, MAX_MONEY, 1).is_ok());
    }

    #[test]
    fn test_policy_refused() {
        let cases = [
            (Network::Mainnet, MAX_ACCOUNT + 1, HEIGHT, 1_000_000, 100),
            (Network::Mainnet, 0, HEIGHT, MAX_MONEY + 1, 100),
            (Network::Mainnet, 0, HEIGHT, 1_000_000, 0),
            (Network::Mainnet, 0, u32::MAX, 1_000_000, 100),
            (Network::Mainnet, 0, 3_000_000, 1_000_000, 100),
            (Network::Testnet, 0, 0, 1_000_000, 100),
        ];
        for (network, account, height, max_fee, window) in cases {
            assert_eq!(
                Policy::new(network, account, height, max_fee, window),
                Err(Error::Policy)
            );
        }
    }

    #[test]
    fn test_add_amount() {
        assert_eq!(add_amount(1, 2), Ok(3));
        assert_eq!(add_amount(MAX_MONEY, 0), Ok(MAX_MONEY));
        assert_eq!(add_amount(MAX_MONEY, 1), Err(Error::Amount));
        assert_eq!(add_amount(u64::MAX, 1), Err(Error::Amount));
    }
}
