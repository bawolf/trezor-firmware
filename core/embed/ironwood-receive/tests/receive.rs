use trezor_ironwood_receive::{Error, Network, derive_external_receiver};

const SEED: [u8; 32] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25,
    26, 27, 28, 29, 30, 31,
];
const INDICES: [[u8; 11]; 6] = [
    [0; 11],
    [1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
    [0xff; 11],
];
const EXPECTED: [[u8; 43]; 6] = [
    [
        250, 114, 126, 98, 40, 69, 134, 149, 45, 16, 37, 101, 160, 15, 209, 94, 133, 165, 154, 17,
        239, 100, 119, 215, 141, 215, 84, 108, 42, 46, 47, 236, 61, 97, 139, 167, 28, 124, 33, 200,
        24, 192, 0,
    ],
    [
        39, 59, 148, 35, 11, 117, 105, 197, 206, 30, 27, 63, 54, 122, 214, 208, 16, 23, 168, 249,
        68, 146, 195, 164, 58, 220, 38, 158, 168, 205, 74, 166, 27, 26, 18, 255, 43, 216, 254, 35,
        59, 198, 54,
    ],
    [
        236, 229, 29, 228, 92, 119, 128, 57, 140, 234, 194, 138, 53, 162, 115, 183, 106, 92, 205,
        151, 19, 85, 39, 30, 135, 207, 159, 205, 194, 144, 122, 68, 98, 209, 10, 104, 227, 124, 70,
        119, 147, 154, 189,
    ],
    [
        124, 137, 5, 241, 238, 36, 49, 146, 234, 118, 140, 194, 27, 17, 173, 227, 171, 50, 202, 35,
        246, 238, 126, 203, 15, 131, 200, 179, 242, 62, 154, 25, 20, 13, 189, 251, 114, 60, 132,
        219, 194, 224, 139,
    ],
    [
        191, 67, 212, 220, 159, 134, 251, 182, 242, 240, 209, 105, 146, 119, 101, 111, 160, 249,
        183, 221, 185, 184, 121, 64, 200, 49, 103, 32, 184, 218, 140, 120, 12, 74, 38, 4, 6, 39,
        115, 74, 62, 56, 178,
    ],
    [
        62, 198, 2, 207, 225, 196, 111, 154, 121, 245, 198, 127, 217, 2, 198, 211, 25, 217, 224,
        93, 38, 200, 222, 17, 45, 28, 112, 22, 155, 209, 133, 49, 146, 231, 121, 146, 114, 121,
        100, 194, 145, 162, 168,
    ],
];

#[test]
fn matches_current_orchard_for_full_88_bit_indices() {
    for (index, expected) in INDICES.into_iter().zip(EXPECTED) {
        assert_eq!(
            derive_external_receiver(&SEED, Network::Testnet, 9, index).unwrap(),
            expected,
        );
    }
}

#[test]
fn network_and_account_are_bound() {
    assert_eq!(
        derive_external_receiver(&SEED, Network::Mainnet, 9, [0; 11]).unwrap(),
        [
            227, 64, 99, 101, 66, 236, 225, 200, 18, 133, 237, 78, 171, 68, 138, 219, 181, 168,
            192, 244, 211, 134, 238, 255, 51, 126, 136, 230, 145, 95, 108, 62, 193, 182, 234, 131,
            90, 136, 213, 102, 18, 210, 189,
        ],
    );
    assert_eq!(
        derive_external_receiver(&SEED, Network::Testnet, 10, [0; 11]).unwrap(),
        [
            181, 93, 248, 193, 225, 148, 187, 86, 187, 39, 246, 41, 15, 0, 168, 82, 149, 38, 234,
            8, 96, 253, 126, 197, 35, 137, 101, 206, 172, 23, 10, 59, 33, 12, 156, 203, 217, 35,
            205, 226, 70, 254, 160,
        ],
    );
}

#[test]
fn rejects_unsupported_seed_lengths() {
    for seed in [&[0u8; 15][..], &[0u8; 17], &[0u8; 31], &[0u8; 253]] {
        assert_eq!(
            derive_external_receiver(seed, Network::Testnet, 0, [0; 11]),
            Err(Error::InvalidSeedLength),
        );
    }
}

#[test]
fn rejects_account_outside_zip32_range() {
    assert_eq!(
        derive_external_receiver(&SEED, Network::Testnet, 1 << 31, [0; 11]),
        Err(Error::InvalidAccount),
    );
}

#[test]
fn accepts_zip32_boundaries() {
    assert_eq!(
        derive_external_receiver(&SEED, Network::Testnet, 0x7fff_ffff, [0; 11]).unwrap(),
        [
            118, 56, 145, 137, 0, 111, 105, 250, 52, 186, 139, 228, 97, 32, 118, 182, 102, 194, 77,
            161, 80, 134, 196, 229, 89, 44, 73, 249, 90, 164, 74, 84, 18, 120, 5, 154, 213, 225,
            32, 210, 233, 130, 154,
        ],
    );
    assert_eq!(
        derive_external_receiver(&[0xa5; 252], Network::Testnet, 0, [0; 11]).unwrap(),
        [
            68, 51, 9, 102, 238, 142, 5, 237, 98, 57, 203, 66, 204, 143, 156, 72, 224, 137, 55,
            187, 95, 145, 112, 201, 127, 48, 240, 92, 225, 236, 144, 34, 225, 149, 65, 95, 153, 73,
            41, 198, 13, 231, 36,
        ],
    );
}

#[test]
fn preserves_restored_128_bit_slip39_mapping() {
    assert_eq!(
        derive_external_receiver(&[0xa5; 16], Network::Testnet, 0, [0; 11]).unwrap(),
        [
            139, 68, 106, 212, 168, 110, 99, 147, 100, 116, 133, 17, 50, 88, 101, 190, 245, 119,
            138, 242, 47, 82, 15, 208, 249, 94, 236, 128, 246, 234, 62, 242, 1, 47, 212, 234, 171,
            187, 50, 187, 20, 174, 161,
        ],
    );
}

#[test]
fn network_codes_are_bounded() {
    assert_eq!(Network::try_from(0), Ok(Network::Mainnet));
    assert_eq!(Network::try_from(1), Ok(Network::Testnet));
    assert_eq!(Network::try_from(2), Err(Error::InvalidNetwork));
    assert_eq!(Network::try_from(u32::MAX), Err(Error::InvalidNetwork));
}
