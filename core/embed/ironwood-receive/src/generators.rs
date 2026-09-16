use trezor_pasta_curves::Fp;
use trezor_pasta_curves::arithmetic::CurveAffine;
use trezor_pasta_curves::pallas::{Affine, Point};

fn point(x: [u64; 4], y: [u64; 4]) -> Point {
    Option::<Affine>::from(Affine::from_xy(Fp::from_raw(x), Fp::from_raw(y)))
        .expect("constant is a Pallas point")
        .into()
}

pub fn spending_key_base() -> Point {
    point(
        [
            0x8d1a7284b875c963,
            0x0c7f0ce37b70a10c,
            0x3b8d187c3e5f445f,
            0x375523b328f1d606,
        ],
        [
            0x4ce33e817b0c3bc9,
            0xdfc914fec005bdd8,
            0x7b10bcfcfed624fb,
            0x1ad0357fdf1a66db,
        ],
    )
}

pub fn ivk_commitment_base() -> Point {
    point(
        [
            0x9823486e5ff8a118,
            0x02957fe2d31aedc7,
            0x1634290a40808948,
            0x25a22ccd5070134e,
        ],
        [
            0x3fe793b3e37fdda9,
            0x6b4442fb1b58a6c7,
            0xc2c890c4284b5794,
            0x29cfd29966a2faeb,
        ],
    )
}

pub fn ivk_commitment_q() -> Point {
    point(
        [
            0x6bcb2f92790f82f2,
            0x421bcc245128a232,
            0x7dcc81b85aa241fa,
            0x05bc0cf14aa9c811,
        ],
        [
            0xbe5ae5cecfaddebe,
            0x46c4351dc96da5f1,
            0xef59074620de054b,
            0x1b014cf6d41abee6,
        ],
    )
}

#[cfg(test)]
mod tests {
    use trezor_pasta_curves::group::GroupEncoding;

    use super::*;

    #[test]
    fn copied_generators_match_official_encodings() {
        assert_eq!(
            spending_key_base().to_bytes(),
            [
                0x63, 0xc9, 0x75, 0xb8, 0x84, 0x72, 0x1a, 0x8d, 0x0c, 0xa1, 0x70, 0x7b, 0xe3, 0x0c,
                0x7f, 0x0c, 0x5f, 0x44, 0x5f, 0x3e, 0x7c, 0x18, 0x8d, 0x3b, 0x06, 0xd6, 0xf1, 0x28,
                0xb3, 0x23, 0x55, 0xb7,
            ],
        );
        assert_eq!(
            ivk_commitment_base().to_bytes(),
            [
                0x18, 0xa1, 0xf8, 0x5f, 0x6e, 0x48, 0x23, 0x98, 0xc7, 0xed, 0x1a, 0xd3, 0xe2, 0x7f,
                0x95, 0x02, 0x48, 0x89, 0x80, 0x40, 0x0a, 0x29, 0x34, 0x16, 0x4e, 0x13, 0x70, 0x50,
                0xcd, 0x2c, 0xa2, 0xa5,
            ],
        );
        assert_eq!(
            ivk_commitment_q().to_bytes(),
            [
                0xf2, 0x82, 0x0f, 0x79, 0x92, 0x2f, 0xcb, 0x6b, 0x32, 0xa2, 0x28, 0x51, 0x24, 0xcc,
                0x1b, 0x42, 0xfa, 0x41, 0xa2, 0x5a, 0xb8, 0x81, 0xcc, 0x7d, 0x11, 0xc8, 0xa9, 0x4a,
                0xf1, 0x0c, 0xbc, 0x05,
            ],
        );
    }
}
