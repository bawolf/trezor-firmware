use blake2b_simd::Params;
use trezor_pasta_curves::Fp;
use trezor_pasta_curves::arithmetic::{CurveAffine, FieldExt};
use trezor_pasta_curves::group::ff::{Field, PrimeField};
use trezor_pasta_curves::group::{Curve, GroupEncoding};
use trezor_pasta_curves::pallas::Scalar;
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::{Error, Network, Result, ff1, generators, sinsemilla};

const RESTORED_SLIP39_SEED_BYTES: usize = 16;
const MIN_ZIP32_SEED_BYTES: usize = 32;
const MAX_SEED_BYTES: usize = 252;
const ZIP32_HARDENED_BIT: u32 = 1 << 31;
const ZIP32_ORCHARD_PURPOSE: u32 = 32;

#[derive(Zeroize, ZeroizeOnDrop)]
struct ExtendedSpendingKey {
    spending_key: [u8; 32],
    chain_code: [u8; 32],
}

struct FullViewingKey {
    ak: Fp,
    nk: Fp,
    rivk: Scalar,
}

fn blake2b_64(personalization: &[u8; 16], parts: &[&[u8]]) -> [u8; 64] {
    let mut state = Params::new()
        .hash_length(64)
        .personal(personalization)
        .to_state();
    for part in parts {
        state.update(part);
    }
    let mut output = [0u8; 64];
    output.copy_from_slice(state.finalize().as_bytes());
    output
}

fn prf_expand(key: &[u8], parts: &[&[u8]]) -> [u8; 64] {
    let mut state = Params::new()
        .hash_length(64)
        .personal(b"Zcash_ExpandSeed")
        .to_state();
    state.update(key);
    for part in parts {
        state.update(part);
    }
    let mut output = [0u8; 64];
    output.copy_from_slice(state.finalize().as_bytes());
    output
}

impl ExtendedSpendingKey {
    fn master(seed: &[u8]) -> Result<Self> {
        let mut material = blake2b_64(b"ZcashIP32Orchard", &[seed]);
        let key = Self {
            spending_key: material[..32].try_into().expect("fixed-size slice"),
            chain_code: material[32..].try_into().expect("fixed-size slice"),
        };
        material.zeroize();
        key.validate()?;
        Ok(key)
    }

    fn derive_hardened_child(&mut self, index: u32) -> Result<()> {
        let index = (index | ZIP32_HARDENED_BIT).to_le_bytes();
        let mut material = prf_expand(&self.chain_code, &[&[0x81], &self.spending_key, &index]);
        self.spending_key.copy_from_slice(&material[..32]);
        self.chain_code.copy_from_slice(&material[32..]);
        material.zeroize();
        self.validate()
    }

    fn validate(&self) -> Result<()> {
        FullViewingKey::from_spending_key(&self.spending_key)?.validate()
    }
}

impl FullViewingKey {
    fn from_spending_key(spending_key: &[u8; 32]) -> Result<Self> {
        let mut ask_material = prf_expand(spending_key, &[&[0x06]]);
        let ask = Scalar::from_bytes_wide(&ask_material);
        ask_material.zeroize();
        if bool::from(ask.is_zero()) {
            return Err(Error::InvalidKey);
        }

        let mut nk_material = prf_expand(spending_key, &[&[0x07]]);
        let nk = Fp::from_bytes_wide(&nk_material);
        nk_material.zeroize();

        let mut rivk_material = prf_expand(spending_key, &[&[0x08]]);
        let rivk = Scalar::from_bytes_wide(&rivk_material);
        rivk_material.zeroize();

        let ak = Option::<Fp>::from(
            (generators::spending_key_base() * ask)
                .to_affine()
                .coordinates()
                .map(|coordinates| *coordinates.x()),
        )
        .ok_or(Error::InvalidKey)?;

        Ok(Self { ak, nk, rivk })
    }

    fn validate(&self) -> Result<()> {
        self.incoming_viewing_key()?;

        let mut material = prf_expand(
            self.rivk.to_repr().as_ref(),
            &[
                &[0x83],
                self.ak.to_repr().as_ref(),
                self.nk.to_repr().as_ref(),
            ],
        );
        let internal_rivk = Scalar::from_bytes_wide(&material);
        material.zeroize();
        let internal = Self {
            ak: self.ak,
            nk: self.nk,
            rivk: internal_rivk,
        };
        internal.incoming_viewing_key()?;
        Ok(())
    }

    fn incoming_viewing_key(&self) -> Result<Scalar> {
        let ivk_base = sinsemilla::commit_ivk(self.ak, self.nk, self.rivk)?;
        let ivk = Option::<Scalar>::from(Scalar::from_repr(ivk_base.to_repr()))
            .ok_or(Error::InvalidKey)?;
        if bool::from(ivk.is_zero()) {
            Err(Error::InvalidKey)
        } else {
            Ok(ivk)
        }
    }

    fn diversifier_key(&self) -> [u8; 32] {
        let mut material = prf_expand(
            self.rivk.to_repr().as_ref(),
            &[
                &[0x82],
                self.ak.to_repr().as_ref(),
                self.nk.to_repr().as_ref(),
            ],
        );
        let key = material[..32].try_into().expect("fixed-size slice");
        material.zeroize();
        key
    }
}

/// Derives one external Orchard receiver without a Rust global allocator.
/// Kept out of line so firmware stack analysis reports the operation boundary.
#[inline(never)]
pub fn derive_external_receiver(
    seed: &[u8],
    network: Network,
    account: u32,
    diversifier_index: [u8; 11],
) -> Result<[u8; 43]> {
    if seed.len() != RESTORED_SLIP39_SEED_BYTES
        && !(MIN_ZIP32_SEED_BYTES..=MAX_SEED_BYTES).contains(&seed.len())
    {
        return Err(Error::InvalidSeedLength);
    }
    if account > 0x7fff_ffff {
        return Err(Error::InvalidAccount);
    }

    let mut extended = ExtendedSpendingKey::master(seed)?;
    for child in [ZIP32_ORCHARD_PURPOSE, network.coin_type(), account] {
        extended.derive_hardened_child(child)?;
    }
    let full_viewing_key = FullViewingKey::from_spending_key(&extended.spending_key)?;
    extended.zeroize();

    let mut diversifier_key = full_viewing_key.diversifier_key();
    let diversifier = ff1::encrypt_diversifier_index(&diversifier_key, diversifier_index);
    diversifier_key.zeroize();
    let incoming_viewing_key = full_viewing_key.incoming_viewing_key()?;
    let transmission_key = sinsemilla::diversify_hash(&diversifier) * incoming_viewing_key;

    let mut receiver = [0u8; 43];
    receiver[..11].copy_from_slice(&diversifier);
    receiver[11..].copy_from_slice(transmission_key.to_bytes().as_ref());
    Ok(receiver)
}
