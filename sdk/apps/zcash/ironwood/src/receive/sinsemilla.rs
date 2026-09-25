use ironwood_pasta_curves::Fp;
use ironwood_pasta_curves::arithmetic::{CurveAffine, CurveExt};
use ironwood_pasta_curves::group::ff::PrimeField;
use ironwood_pasta_curves::group::{Curve, Group};
use ironwood_pasta_curves::pallas::{Point, Scalar};

use super::{Error, Result, generators};

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

/// `progress` is called after each of the 51 chunks, the slow part.
pub fn commit_ivk(ak: Fp, nk: Fp, rivk: Scalar, progress: &mut dyn FnMut()) -> Result<Fp> {
    let mut acc = generators::ivk_commitment_q();
    // `hash_to_curve` returns a closure holding the domain-separated hasher
    // state, so it is built once and applied to all 51 chunks.
    let hash_s = Point::hash_to_curve("z.cash:SinsemillaS");

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
        let s = hash_s(&u32::from(index).to_le_bytes());
        acc = incomplete_add(incomplete_add(acc, s)?, acc)?;
        progress();
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
    let hash_gd = Point::hash_to_curve("z.cash:Orchard-gd");
    let point = hash_gd(diversifier);
    if bool::from(point.is_identity()) {
        hash_gd(&[])
    } else {
        point
    }
}
