#![allow(dead_code)] // Shared by separate integration-test binaries.

use std::sync::atomic::{AtomicU64, Ordering};

use orchard::bundle::BundleVersion;
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use orchard::note_encryption::IronwoodDomain;
use orchard::value::NoteValue;
use pczt::Pczt;
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::redactor::Redactor;
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use rand_core::{CryptoRng, RngCore};
use serde_json::Value;
use trezor_ironwood::{
    Account, Engine, Error, ErrorCode, Limits, Network, Policy, RequestContext, Result, Review,
};
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BundlePadding, DeferredPcztBuilder};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{MAIN_NETWORK, Parameters, TEST_NETWORK};
use zcash_protocol::local_consensus::LocalNetwork;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;

pub const HEIGHT: u32 = 10_000_000;
pub const ACCOUNT: u32 = 9;

pub fn keys() -> (FullViewingKey, SpendAuthorizingKey) {
    let sk = SpendingKey::from_bytes([0; 32]).unwrap(); // PUBLIC TEST SEED ONLY
    (FullViewingKey::from(&sk), SpendAuthorizingKey::from(&sk))
}

pub fn policy(network: Network, maximum_fee: u64) -> Policy {
    Policy::new(
        RequestContext::new(network, Account::new(ACCOUNT).unwrap(), HEIGHT),
        Limits::new(maximum_fee, 100).unwrap(),
    )
    .unwrap()
}

pub fn assert_error(error: Error, code: ErrorCode) {
    assert_eq!(error.code(), code);
}

pub fn engine() -> Engine<ChaCha20Rng> {
    static SEQUENCE: AtomicU64 = AtomicU64::new(1);
    let value = SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let mut seed = [0; 32];
    seed[..8].copy_from_slice(&value.to_le_bytes());
    Engine::with_rng(
        policy(Network::Testnet, 100_000),
        ChaCha20Rng::from_seed(seed),
    )
    .unwrap()
}

pub trait TestEngineExt {
    fn begin_test(&mut self, bytes: &[u8]) -> Result<Review>;
}

impl<R: RngCore + CryptoRng> TestEngineExt for Engine<R> {
    fn begin_test(&mut self, bytes: &[u8]) -> Result<Review> {
        self.begin(bytes, &keys().0)
    }
}

fn local_network() -> LocalNetwork {
    LocalNetwork {
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
    }
}

pub fn fixture() -> Vec<u8> {
    // Synthetic local-consensus/regtest-compatible fixture used as a stable
    // oracle. Production network handling is covered separately by
    // `network_fixture` with the real Mainnet and Testnet parameters.
    static BYTES: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    BYTES
        .get_or_init(|| build(600_000, 390_000, MemoBytes::empty(), false))
        .clone()
}

pub fn network_fixture(network: Network) -> Vec<u8> {
    match network {
        Network::Mainnet => build_with_network(
            MAIN_NETWORK,
            600_000,
            390_000,
            MemoBytes::empty(),
            0x31,
            OutputPolicy::standard(false),
        ),
        Network::Testnet => build_with_network(
            TEST_NETWORK,
            600_000,
            390_000,
            MemoBytes::empty(),
            0x32,
            OutputPolicy::standard(false),
        ),
    }
}

pub fn build(payment: u64, change: u64, memo: MemoBytes, self_payment: bool) -> Vec<u8> {
    build_with_network(
        local_network(),
        payment,
        change,
        memo,
        42,
        OutputPolicy::standard(self_payment),
    )
}

pub fn build_with_wrong_change_ovk() -> Vec<u8> {
    build_with_network(
        local_network(),
        600_000,
        390_000,
        MemoBytes::empty(),
        43,
        OutputPolicy {
            change_ovk_scope: Scope::External,
            ..OutputPolicy::standard(false)
        },
    )
}

pub fn build_with_discarded_payment_ovk() -> Vec<u8> {
    build_with_network(
        local_network(),
        600_000,
        390_000,
        MemoBytes::empty(),
        44,
        OutputPolicy {
            payment_ovk_scope: None,
            ..OutputPolicy::standard(false)
        },
    )
}

pub fn build_with_dummy_spend_padding() -> Vec<u8> {
    build_with_network(
        local_network(),
        990_000,
        0,
        MemoBytes::empty(),
        46,
        OutputPolicy::standard(false),
    )
}

#[derive(Clone, Copy)]
struct OutputPolicy {
    self_payment: bool,
    payment_ovk_scope: Option<Scope>,
    change_ovk_scope: Scope,
}

impl OutputPolicy {
    const fn standard(self_payment: bool) -> Self {
        Self {
            self_payment,
            payment_ovk_scope: Some(Scope::External),
            change_ovk_scope: Scope::Internal,
        }
    }
}

fn build_with_network<P: Parameters>(
    network: P,
    payment: u64,
    change: u64,
    memo: MemoBytes,
    seed_byte: u8,
    output_policy: OutputPolicy,
) -> Vec<u8> {
    let mut rng = ChaCha20Rng::from_seed([seed_byte; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    let own_address = fvk.address_at(0u32, Scope::External);
    let recipient = if output_policy.self_payment {
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
            output_policy
                .payment_ovk_scope
                .map(|scope| fvk.to_ovk(scope)),
            recipient,
            Zatoshis::from_u64(payment).unwrap(),
            memo,
        )
        .unwrap();
    if change > 0 {
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(output_policy.change_ovk_scope)),
                fvk.address_at(1u32, Scope::Internal),
                Zatoshis::from_u64(change).unwrap(),
                MemoBytes::empty(),
            )
            .unwrap();
    }
    finish(builder, &mut rng)
}

fn finish<P: Parameters>(builder: DeferredPcztBuilder<P>, rng: &mut ChaCha20Rng) -> Vec<u8> {
    let result = builder
        .build_for_pczt(rng, &zip317::FeeRule::standard())
        .unwrap();
    let pczt = IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io()
        .unwrap();
    Redactor::new(pczt)
        .redact_sapling_with(|mut sapling| {
            sapling.clear_bsk();
            sapling.clear_anchor();
        })
        .redact_ironwood_with(|mut ironwood| {
            ironwood.clear_bsk();
            ironwood.redact_actions(|mut action| action.clear_spend_witness());
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

pub fn signatures(pczt: &Pczt) -> Vec<(usize, [u8; 64])> {
    let value = json(&pczt.clone().serialize().unwrap());
    pczt.ironwood()
        .actions()
        .iter()
        .enumerate()
        .filter_map(|(index, action)| {
            if value["ironwood"]["actions"][index]["spend"]["value"]
                .as_u64()
                .is_some_and(|value| value > 0)
            {
                (*action.spend().spend_auth_sig()).map(|signature| (index, signature))
            } else {
                None
            }
        })
        .collect()
}

pub fn mutate(f: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = json(&fixture());
    f(&mut value);
    encode(value)
}

pub fn real(value: &Value) -> usize {
    value["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|action| action["spend"]["value"].as_u64().unwrap() > 0)
        .unwrap()
}

pub fn payment(value: &Value) -> usize {
    value["ironwood"]["actions"]
        .as_array()
        .unwrap()
        .iter()
        .position(|action| action["output"]["value"] == 600_000)
        .unwrap()
}

pub fn flip(value: &mut Value) {
    let n = value[0].as_u64().unwrap();
    value[0] = (n ^ 1).into();
}

/// Largest bundle the synthetic equivalence fixtures construct. Deliberately
/// fixed and INDEPENDENT of `MAX_ACTIONS`: the Session↔Engine equivalence it
/// proves is per-action logic that does not vary with the action count, so a
/// 1..=CORPUS_MAX_ACTIONS sweep is representative. When `MAX_ACTIONS` was raised
/// 8→32 for 16/32-action signing, this stayed 8 so the equivalence corpus keeps
/// its 135-case size; 16/32-action parse+sign+verify is proven on the emulator
/// (host-side pczt verify), not by this host corpus.
pub const CORPUS_MAX_ACTIONS: usize = 8;

/// Synthetic local-consensus fixture covering every admitted action count.
pub fn build_actions(outputs: usize) -> Vec<u8> {
    assert!((1..=CORPUS_MAX_ACTIONS).contains(&outputs));
    let mut rng = ChaCha20Rng::from_seed([outputs as u8; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
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
    let mut builder = DeferredPcztBuilder::new::<zip317::FeeError>(
        local_network(),
        HEIGHT.into(),
        BundlePadding::DEFAULT,
        BundlePadding::UNPADDED,
    )
    .unwrap();
    builder
        .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
        .unwrap();
    for i in 0..outputs {
        let (recipient, ovk_scope) = if i % 2 == 0 {
            (other.address_at(i as u32, Scope::External), Scope::External)
        } else {
            (fvk.address_at(i as u32, Scope::Internal), Scope::Internal)
        };
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(ovk_scope)),
                recipient,
                Zatoshis::from_u64((i as u64 + 1) * 100_000).unwrap(),
                MemoBytes::empty(),
            )
            .unwrap();
    }
    finish(builder, &mut rng)
}

pub fn build_inputs(values: &[u64]) -> Vec<u8> {
    let mut rng = ChaCha20Rng::from_seed([88; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    let mut builder = DeferredPcztBuilder::new::<zip317::FeeError>(
        local_network(),
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
    finish(builder, &mut rng)
}
