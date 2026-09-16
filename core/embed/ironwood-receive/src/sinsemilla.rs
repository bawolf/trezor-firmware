use trezor_pasta_curves::Fp;
use trezor_pasta_curves::arithmetic::{CurveAffine, CurveExt};
use trezor_pasta_curves::group::ff::PrimeField;
use trezor_pasta_curves::group::{Curve, Group};
use trezor_pasta_curves::pallas::{Point, Scalar};

use crate::{Error, Result, generators};

fn hash_to_point(domain: &str, message: &[u8]) -> Point {
    Point::hash_to_curve(domain, message)
}

fn incomplete_add(lhs: Point, rhs: Point) -> Result<Point> {
    // Sinsemilla uses incomplete addition and rejects its exceptional inputs.
    if bool::from(lhs.is_identity()) || bool::from(rhs.is_identity()) || lhs == rhs || lhs == -rhs {
        Err(Error::InvalidKey)
    } else {
        Ok(lhs + rhs)
    }
}

fn field_bit(value: &Fp, index: usize) -> u8 {
    let bytes = value.to_repr();
    (bytes[index / 8] >> (index % 8)) & 1
}

pub fn commit_ivk(ak: Fp, nk: Fp, rivk: Scalar) -> Result<Fp> {
    let mut acc = generators::ivk_commitment_q();

    for chunk in 0..51 {
        let mut index = 0u16;
        for offset in 0..10 {
            let bit_index = chunk * 10 + offset;
            let bit = if bit_index < 255 {
                field_bit(&ak, bit_index)
            } else {
                field_bit(&nk, bit_index - 255)
            };
            index |= u16::from(bit) << offset;
        }
        let s = hash_to_point("z.cash:SinsemillaS", &u32::from(index).to_le_bytes());
        acc = incomplete_add(incomplete_add(acc, s)?, acc)?;
    }

    let commitment = acc + generators::ivk_commitment_base() * rivk;
    let base = Option::<Fp>::from(
        commitment
            .to_affine()
            .coordinates()
            .map(|coordinates| *coordinates.x()),
    )
    .unwrap_or_else(Fp::zero);
    Ok(base)
}

pub fn diversify_hash(diversifier: &[u8; 11]) -> Point {
    let point = hash_to_point("z.cash:Orchard-gd", diversifier);
    if bool::from(point.is_identity()) {
        hash_to_point("z.cash:Orchard-gd", &[])
    } else {
        point
    }
}
