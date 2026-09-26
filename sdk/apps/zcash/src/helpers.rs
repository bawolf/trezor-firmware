use alloc::string::String;

use zcash_signer::Network;

use crate::uformat;

const ZEC_DECIMALS: usize = 8;

/// "1,234.5 ZEC", or TAZ on testnet.
pub fn format_zec_amount(zatoshis: u64, network: Network) -> String {
    let unit = match network {
        Network::Mainnet => "ZEC",
        Network::Testnet => "TAZ",
    };
    let digits = uformat!("{}", zatoshis);
    uformat!(
        "{} {}",
        format_amount_from_digits(&digits, ZEC_DECIMALS).as_str(),
        unit
    )
}

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

fn format_amount_from_digits(digits: &str, decimals: usize) -> String {
    let mut out = String::with_capacity(digits.len() + digits.len() / 3 + 3);

    if digits.len() <= decimals {
        out.push('0');
        out.push('.');
        for _ in 0..(decimals - digits.len()) {
            out.push('0');
        }
        out.push_str(digits);
    } else {
        let split = digits.len() - decimals;
        let (int_part, frac_part) = digits.split_at(split);
        push_grouped_digits(&mut out, int_part);
        out.push('.');
        out.push_str(frac_part);
    }

    while out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }

    out
}

fn push_grouped_digits(out: &mut String, digits: &str) {
    for (i, ch) in digits.chars().enumerate() {
        if i != 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_zec_amount() {
        let cases = [
            (0, "0 ZEC"),
            (1, "0.00000001 ZEC"),
            (100_000, "0.001 ZEC"),
            (100_000_000, "1 ZEC"),
            (123_456_789_012, "1,234.56789012 ZEC"),
            (2_100_000_000_000_000, "21,000,000 ZEC"),
        ];
        for (zatoshis, expected) in cases {
            assert_eq!(format_zec_amount(zatoshis, Network::Mainnet), expected);
        }
    }

    #[test]
    fn test_format_zec_amount_testnet() {
        assert_eq!(format_zec_amount(150_000_000, Network::Testnet), "1.5 TAZ");
    }

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
