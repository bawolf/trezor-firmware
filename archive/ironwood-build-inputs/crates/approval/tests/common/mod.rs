use ironwood_approval::{Engine, Policy};
use orchard::{
    bundle::BundleVersion,
    keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey},
    note_encryption::IronwoodDomain,
    value::NoteValue,
};
use pczt::{
    Pczt,
    roles::{creator::Creator, io_finalizer::IoFinalizer, redactor::Redactor},
};
use rand_chacha::{ChaCha20Rng, rand_core::SeedableRng};
use serde_json::Value;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::{
    builder::{BundlePadding, DeferredPcztBuilder},
    fees::zip317,
};
use zcash_protocol::{local_consensus::LocalNetwork, memo::MemoBytes, value::Zatoshis};

pub const HEIGHT: u32 = 10_000_000;
pub fn keys() -> (FullViewingKey, SpendAuthorizingKey) {
    let sk = SpendingKey::from_bytes([0; 32]).unwrap(); // PUBLIC TEST SEED ONLY
    (FullViewingKey::from(&sk), SpendAuthorizingKey::from(&sk))
}
pub fn engine() -> Engine<rand_core::OsRng> {
    Engine::new(Policy::regtest(HEIGHT, 100_000).unwrap(), keys().0).unwrap()
}
pub fn fixture() -> Vec<u8> {
    static BYTES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    BYTES
        .get_or_init(|| build(600_000, 390_000, MemoBytes::empty(), false))
        .clone()
}
pub fn build(payment: u64, change: u64, memo: MemoBytes, self_payment: bool) -> Vec<u8> {
    let mut rng = ChaCha20Rng::from_seed([42; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    let own_address = fvk.address_at(0u32, Scope::External);
    let recipient = if self_payment {
        own_address
    } else {
        other.address_at(0u32, Scope::External)
    };
    let version = BundleVersion::ironwood_v3();
    let mut funding = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        version,
        version.default_flags(),
        orchard::Anchor::empty_tree(),
    )
    .unwrap();
    funding
        .add_output(
            None,
            own_address,
            NoteValue::from_raw(payment + change + 10_000),
            MemoBytes::empty().into_bytes(),
        )
        .unwrap();
    let (bundle, meta) = funding.build_for_pczt(&mut rng).unwrap();
    let action = &bundle.actions()[meta.output_action_index(0).unwrap()];
    let (note, _, _) = try_note_decryption(
        &IronwoodDomain::for_pczt_action(action),
        &fvk.to_ivk(Scope::External).prepare(),
        action,
    )
    .unwrap();
    let network = LocalNetwork {
        overwinter: Some(1.into()),
        sapling: Some(2.into()),
        blossom: Some(3.into()),
        heartwood: Some(4.into()),
        canopy: Some(5.into()),
        nu5: Some(6.into()),
        nu6: Some(7.into()),
        nu6_1: Some(8.into()),
        nu6_2: Some(9.into()),
        nu6_3: Some(10.into()),
    };
    let mut builder = DeferredPcztBuilder::new::<zip317::FeeError>(
        network,
        HEIGHT.into(),
        BundlePadding::DEFAULT,
        BundlePadding::DEFAULT,
    )
    .unwrap();
    builder
        .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
        .unwrap();
    builder
        .add_ironwood_output::<zip317::FeeError>(
            Some(fvk.to_ovk(Scope::External)),
            recipient,
            Zatoshis::from_u64(payment).unwrap(),
            memo,
        )
        .unwrap();
    if change > 0 {
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(Scope::External)),
                fvk.address_at(1u32, Scope::Internal),
                Zatoshis::from_u64(change).unwrap(),
                MemoBytes::empty(),
            )
            .unwrap();
    }
    let result = builder
        .build_for_pczt(&mut rng, &zip317::FeeRule::standard())
        .unwrap();
    let pczt = IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io()
        .unwrap();
    Redactor::new(pczt)
        .redact_sapling_with(|mut s| {
            s.clear_bsk();
            s.clear_anchor();
        })
        .redact_ironwood_with(|mut i| {
            i.clear_bsk();
            i.redact_actions(|mut a| a.clear_spend_witness());
        })
        .finish()
        .serialize()
        .unwrap()
}
pub fn json(bytes: &[u8]) -> Value {
    serde_json::to_value(pczt::v2::Pczt::try_from(Pczt::parse(bytes).unwrap()).unwrap()).unwrap()
}
pub fn encode(value: Value) -> Vec<u8> {
    serde_json::from_value::<pczt::v2::Pczt>(value)
        .unwrap()
        .serialize()
}
pub fn mutate(f: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut v = json(&fixture());
    f(&mut v);
    encode(v)
}
pub fn real(v: &Value) -> usize {
    v["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a["spend"]["value"].as_u64().unwrap() > 0)
        .unwrap()
}
pub fn payment(v: &Value) -> usize {
    v["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|a| a["output"]["value"] == 600_000)
        .unwrap()
}
pub fn flip(v: &mut Value) {
    let n = v[0].as_u64().unwrap();
    v[0] = (n ^ 1).into();
}

/// Public synthetic fixture covering every admitted action count; no engine changes.
pub fn build_actions(outputs: usize) -> Vec<u8> {
    assert!((1..=8).contains(&outputs));
    let mut rng = ChaCha20Rng::from_seed([outputs as u8; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    // ZIP 317 retains its two-action minimum fee even for one unpadded action.
    let input = (1..=outputs as u64).sum::<u64>() * 100_000 + outputs.max(2) as u64 * 5_000;
    let version = BundleVersion::ironwood_v3();
    let recipient = fvk.address_at(0u32, Scope::External);
    let mut funding = orchard::builder::Builder::new(
        orchard::builder::BundleType::DEFAULT,
        version,
        version.default_flags(),
        orchard::Anchor::empty_tree(),
    )
    .unwrap();
    funding
        .add_output(
            None,
            recipient,
            NoteValue::from_raw(input),
            MemoBytes::empty().into_bytes(),
        )
        .unwrap();
    let (bundle, meta) = funding.build_for_pczt(&mut rng).unwrap();
    let action = &bundle.actions()[meta.output_action_index(0).unwrap()];
    let (note, _, _) = try_note_decryption(
        &IronwoodDomain::for_pczt_action(action),
        &fvk.to_ivk(Scope::External).prepare(),
        action,
    )
    .unwrap();
    let network = LocalNetwork {
        overwinter: Some(1.into()),
        sapling: Some(2.into()),
        blossom: Some(3.into()),
        heartwood: Some(4.into()),
        canopy: Some(5.into()),
        nu5: Some(6.into()),
        nu6: Some(7.into()),
        nu6_1: Some(8.into()),
        nu6_2: Some(9.into()),
        nu6_3: Some(10.into()),
    };
    let mut builder = DeferredPcztBuilder::new::<zip317::FeeError>(
        network,
        HEIGHT.into(),
        BundlePadding::DEFAULT,
        BundlePadding::UNPADDED,
    )
    .unwrap();
    builder
        .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
        .unwrap();
    for i in 0..outputs {
        let recipient = if i % 2 == 0 {
            other.address_at(i as u32, Scope::External)
        } else {
            fvk.address_at(i as u32, Scope::Internal)
        };
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(Scope::External)),
                recipient,
                Zatoshis::from_u64((i as u64 + 1) * 100_000).unwrap(),
                MemoBytes::empty(),
            )
            .unwrap();
    }
    let result = builder
        .build_for_pczt(&mut rng, &zip317::FeeRule::standard())
        .unwrap();
    let pczt = IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io()
        .unwrap();
    Redactor::new(pczt)
        .redact_sapling_with(|mut s| {
            s.clear_bsk();
            s.clear_anchor();
        })
        .redact_ironwood_with(|mut i| {
            i.clear_bsk();
            i.redact_actions(|mut a| a.clear_spend_witness());
        })
        .finish()
        .serialize()
        .unwrap()
}

pub fn build_inputs(values: &[u64]) -> Vec<u8> {
    let mut rng = ChaCha20Rng::from_seed([88; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    let network = LocalNetwork {
        overwinter: Some(1.into()),
        sapling: Some(2.into()),
        blossom: Some(3.into()),
        heartwood: Some(4.into()),
        canopy: Some(5.into()),
        nu5: Some(6.into()),
        nu6: Some(7.into()),
        nu6_1: Some(8.into()),
        nu6_2: Some(9.into()),
        nu6_3: Some(10.into()),
    };
    let mut builder = DeferredPcztBuilder::new::<zip317::FeeError>(
        network,
        HEIGHT.into(),
        BundlePadding::DEFAULT,
        BundlePadding::DEFAULT,
    )
    .unwrap();
    let version = BundleVersion::ironwood_v3();
    for (i, value) in values.iter().enumerate() {
        let mut funding = orchard::builder::Builder::new(
            orchard::builder::BundleType::DEFAULT,
            version,
            version.default_flags(),
            orchard::Anchor::empty_tree(),
        )
        .unwrap();
        funding
            .add_output(
                None,
                fvk.address_at(i as u32, Scope::External),
                NoteValue::from_raw(*value),
                MemoBytes::empty().into_bytes(),
            )
            .unwrap();
        let (bundle, meta) = funding.build_for_pczt(&mut rng).unwrap();
        let action = &bundle.actions()[meta.output_action_index(0).unwrap()];
        let (note, _, _) = try_note_decryption(
            &IronwoodDomain::for_pczt_action(action),
            &fvk.to_ivk(Scope::External).prepare(),
            action,
        )
        .unwrap();
        builder
            .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
            .unwrap();
    }
    let fee = values.len().max(2) as u64 * 5_000;
    let payment = values.iter().sum::<u64>() - fee;
    builder
        .add_ironwood_output::<zip317::FeeError>(
            Some(fvk.to_ovk(Scope::External)),
            other.address_at(0u32, Scope::External),
            Zatoshis::from_u64(payment).unwrap(),
            MemoBytes::empty(),
        )
        .unwrap();
    let result = builder
        .build_for_pczt(&mut rng, &zip317::FeeRule::standard())
        .unwrap();
    let pczt = IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io()
        .unwrap();
    Redactor::new(pczt)
        .redact_sapling_with(|mut s| {
            s.clear_bsk();
            s.clear_anchor();
        })
        .redact_ironwood_with(|mut i| {
            i.clear_bsk();
            i.redact_actions(|mut a| a.clear_spend_witness());
        })
        .finish()
        .serialize()
        .unwrap()
}
