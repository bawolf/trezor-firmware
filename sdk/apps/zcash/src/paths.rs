use alloc::string::String;

use trezor_app_sdk::{Error, Result};
use zcash_signer::Network;

use crate::uformat;

const HARDENED: u32 = 0x8000_0000;
const PURPOSE: u32 = 32;
/// The account range `[0-100]'` of the manifest's paths, which Core enforces
/// too.
const MAX_ACCOUNT: u32 = 100;

/// The network and account of a ZIP-32 account path `m/32'/coin_type'/account'`.
pub fn account_from_path(address_n: &[u32]) -> Result<(Network, u32)> {
    let forbidden = || Error::DataError("Forbidden key path");
    let &[purpose, coin_type, account] = address_n else {
        return Err(forbidden());
    };
    let network = [Network::Mainnet, Network::Testnet]
        .into_iter()
        .find(|network| coin_type == network.coin_type() | HARDENED)
        .ok_or_else(forbidden)?;
    if purpose != PURPOSE | HARDENED || account & HARDENED == 0 {
        return Err(forbidden());
    }
    let account = account & !HARDENED;
    if account > MAX_ACCOUNT {
        return Err(forbidden());
    }
    Ok((network, account))
}

/// "m/32'/133'/0'" for mainnet account 0.
pub fn format_path(network: Network, account: u32) -> String {
    uformat!("m/{}'/{}'/{}'", PURPOSE, network.coin_type(), account)
}

#[cfg(test)]
mod tests {
    use super::*;

    const H: u32 = HARDENED;

    #[test]
    fn test_account_from_path() {
        assert_eq!(
            account_from_path(&[32 | H, 133 | H, H]).ok(),
            Some((Network::Mainnet, 0))
        );
        assert_eq!(
            account_from_path(&[32 | H, 1 | H, 100 | H]).ok(),
            Some((Network::Testnet, 100))
        );
    }

    #[test]
    fn test_account_from_path_forbidden() {
        let paths: [&[u32]; 8] = [
            &[],
            &[32 | H, 133 | H],
            &[32 | H, 133 | H, H, 0],
            &[44 | H, 133 | H, H],
            &[32 | H, 60 | H, H],
            &[32 | H, 133, H],
            &[32 | H, 133 | H, 0],
            &[32 | H, 133 | H, 101 | H],
        ];
        for path in paths {
            assert!(account_from_path(path).is_err(), "{path:?}");
        }
    }

    #[test]
    fn test_format_path() {
        assert_eq!(format_path(Network::Mainnet, 0), "m/32'/133'/0'");
        assert_eq!(format_path(Network::Testnet, 7), "m/32'/1'/7'");
    }
}
