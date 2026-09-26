use alloc::string::String;

use zcash_signer::Network;

use crate::uformat;

/// "Mainnet" or "Testnet".
pub fn network_label(network: Network) -> &'static str {
    match network {
        Network::Mainnet => tr!("words__mainnet"),
        Network::Testnet => tr!("words__testnet"),
    }
}

/// "Zcash account #1 (Mainnet)" for mainnet account 0, as Core names it.
pub fn account_label(network: Network, account: u32) -> String {
    uformat!(
        "{} #{} ({})",
        tr!("zcash__account"),
        account + 1,
        network_label(network)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_account_label() {
        assert_eq!(
            account_label(Network::Mainnet, 0),
            "Zcash account #1 (Mainnet)"
        );
        assert_eq!(
            account_label(Network::Testnet, 99),
            "Zcash account #100 (Testnet)"
        );
    }
}
