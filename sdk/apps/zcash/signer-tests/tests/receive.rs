//! Receivers of the account's external addresses.

use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use zcash_signer::MAX_ACCOUNT;
use zcash_signer::keys::external_receiver;
use zcash_signer_tests::unhex;

const SEED: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];

fn fvk(coin_type: u32, account: u32) -> FullViewingKey {
    let account = zip32::AccountId::try_from(account).unwrap();
    FullViewingKey::from(&SpendingKey::from_zip32_seed(&SEED, coin_type, account).unwrap())
}

/// `(coin type, account, diversifier index, receiver)`.
#[rustfmt::skip]
const RECEIVERS: [(u32, u32, u128, &str); 9] = [
    (1, 9, 0, "fa727e62284586952d102565a00fd15e85a59a11ef6477d78dd7546c2a2e2fec3d618ba71c7c21c818c000"),
    (1, 9, 1, "273b94230b7569c5ce1e1b3f367ad6d01017a8f94492c3a43adc269ea8cd4aa61b1a12ff2bd8fe233bc636"),
    (1, 9, 1 << 8, "ece51de45c7780398ceac28a35a273b76a5ccd971355271e87cf9fcdc2907a4462d10a68e37c4677939abd"),
    (1, 9, 1 << 32, "7c8905f1ee243192ea768cc21b11ade3ab32ca23f6ee7ecb0f83c8b3f23e9a19140dbdfb723c84dbc2e08b"),
    (1, 9, 1 << 80, "bf43d4dc9f86fbb6f2f0d1699277656fa0f9b7ddb9b87940c8316720b8da8c780c4a26040627734a3e38b2"),
    (1, 9, (1 << 88) - 1, "3ec602cfe1c46f9a79f5c67fd902c6d319d9e05d26c8de112d1c70169bd1853192e77992727964c291a2a8"),
    (133, 9, 0, "e340636542ece1c81285ed4eab448adbb5a8c0f4d386eeff337e88e6915f6c3ec1b6ea835a88d56612d2bd"),
    (1, 10, 0, "b55df8c1e194bb56bb27f6290f00a8529526ea0860fd7ec5238965ceac170a3b210c9ccbd923cde246fea0"),
    (1, MAX_ACCOUNT, 0, "76389189006f69fa34ba8be4612076b666c24da15086c4e5592c49f95aa44a541278059ad5e120d2e9829a"),
];

#[test]
fn test_external_receiver() {
    for (coin_type, account, index, expected) in RECEIVERS {
        let fvk = fvk(coin_type, account);
        let index: [u8; 11] = index.to_le_bytes()[..11].try_into().unwrap();
        let receiver = external_receiver(&fvk, index, &mut || {});
        assert_eq!(
            receiver.to_vec(),
            unhex(expected),
            "{coin_type} {account} {index:?}"
        );
        let orchard = fvk
            .address_at(index, Scope::External)
            .to_raw_address_bytes();
        assert_eq!(receiver, orchard);
    }
}

#[test]
fn test_progress() {
    let mut calls = 0;
    external_receiver(&fvk(1, 0), [0; 11], &mut || calls += 1);
    assert!(calls > 0);
}
