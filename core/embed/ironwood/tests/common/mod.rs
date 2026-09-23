#![allow(dead_code)] // Shared by separate integration-test binaries.

use std::sync::atomic::{AtomicU64, Ordering};

use ironwood::{
    Account, Engine, Error, ErrorCode, Event, Hedged, Limits, MAX_ACTIONS, MAX_TRANSPARENT_OUTPUTS,
    Network, Policy, RequestContext, Result, Review, Session,
};
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
use rand_core::{CryptoRng, OsRng, RngCore};
use serde_json::Value;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BundlePadding, DeferredPcztBuilder};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{MAIN_NETWORK, Parameters, TEST_NETWORK};
use zcash_protocol::local_consensus::LocalNetwork;
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;

pub const HEIGHT: u32 = 10_000_000;
pub const ACCOUNT: u32 = 9;
/// The corpus keys are raw spending keys, not seed-derived, so the device's
/// seed fingerprint is a fixed PUBLIC TEST VALUE the host claims must match.
pub const SEED_FINGERPRINT: [u8; 32] = [0x5e; 32];

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

/// `signing::Signing`, which `signing::begin` boxes into the scratch tier:
/// the session, its key and its review live in the tier, not on a stack.
#[allow(dead_code)]
struct Signing {
    handle: u32,
    session: Session<Hedged<OsRng>>,
    fvk: FullViewingKey,
    coin_type: u32,
    account: u32,
    review: Option<Review>,
}

/// `sign_pczt.CHUNK_BYTES`: what one `ZcashPcztAck` carries.
pub const CHUNK_BYTES: usize = 1024;

/// Everything the firmware allocates between `install_scratch` and
/// `release_scratch` for one PCZT, in its order: the boxed request, the PCZT
/// fed one `ZcashPcztAck` at a time, approve, sign. Returns the number of
/// signatures. The corpus keys stand in for the seed-derived ones; `keys()`
/// does the same key expansion `AccountKeys::derive` ends in. The fee limit
/// admits the widest bundle (ZIP-317: `5_000 * 32`).
pub fn sign_streamed(pczt: &[u8]) -> usize {
    let seed = [7u8; 64];
    // `signing::begin`.
    let (fvk, _) = keys();
    let _ = ironwood::seed_fingerprint(&seed);
    let mut session = Session::with_rng(
        policy(Network::Testnet, 1_000_000),
        Hedged::from_seed(OsRng, &seed),
    )
    .unwrap();
    session.begin(pczt.len(), &fvk, &SEED_FINGERPRINT).unwrap();
    let mut signing = Box::new(Signing {
        handle: 1,
        session,
        fvk,
        coin_type: 1,
        account: ACCOUNT,
        review: None,
    });
    // `session_feed`, one `ZcashPcztAck` at a time, each fed until consumed.
    for chunk in pczt.chunks(CHUNK_BYTES) {
        let mut fed = 0;
        while fed < chunk.len() {
            let (consumed, event) = signing.session.feed(&chunk[fed..], &signing.fvk).unwrap();
            fed += consumed;
            if let Event::Review(review) = event {
                signing.review = Some(review);
            }
        }
    }
    // `session_approve`, then `session_sign`, which ends the request.
    let review = signing.review.as_ref().expect("the PCZT reaches review");
    signing.session.approve(review.token()).unwrap();
    let (_, ask) = keys();
    let signatures = signing.session.sign(review.token(), &ask).unwrap();
    signatures.records().len()
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
        self.begin(bytes, &keys().0, &SEED_FINGERPRINT)
    }
}

/// `m/32'/coin_type'/account'` of the corpus policy: the derivation the
/// device signs under, so the only claim it admits.
pub fn own_path(network: Network) -> [u32; 3] {
    policy(network, 100_000).request().zip32_path()
}

/// A `zip32_derivation` value in the v2 JSON view.
pub fn derivation_json(seed_fingerprint: &[u8; 32], path: &[u32]) -> Value {
    serde_json::json!({
        "seed_fingerprint": seed_fingerprint.to_vec(),
        "derivation_path": path,
    })
}

/// The fixture with the device's own derivation claimed on the real spend
/// (where the standard SDK puts it) and, when `on_change`, on the change
/// output too.
pub fn with_own_derivation(on_change: bool) -> Vec<u8> {
    let path = own_path(Network::Testnet);
    mutate(|v| {
        let i = real(v);
        v["ironwood"]["actions"][i]["spend"]["zip32_derivation"] =
            derivation_json(&SEED_FINGERPRINT, &path);
        if on_change {
            let i = 1 - payment(v);
            v["ironwood"]["actions"][i]["output"]["zip32_derivation"] =
                derivation_json(&SEED_FINGERPRINT, &path);
        }
    })
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
            change_ovk_scope: Some(Scope::External),
            ..OutputPolicy::standard(false)
        },
    )
}

/// A nonempty memo on the (hidden) change output, which the memo policy
/// refuses: memos of hidden outputs must be empty.
pub fn build_with_change_memo(memo: MemoBytes) -> Vec<u8> {
    build_with_network(
        local_network(),
        600_000,
        390_000,
        MemoBytes::empty(),
        48,
        OutputPolicy {
            change_memo: Some(memo),
            ..OutputPolicy::standard(false)
        },
    )
}

/// Change encrypted with no OVK at all: what the stock SDK's default
/// `OvkPolicy::Sender` builds (`internal_ovk: None`).
pub fn build_with_change_without_ovk() -> Vec<u8> {
    build_with_network(
        local_network(),
        600_000,
        390_000,
        MemoBytes::empty(),
        45,
        OutputPolicy {
            change_ovk_scope: None,
            ..OutputPolicy::standard(false)
        },
    )
}

/// The bytes a stock-SDK wallet hands over unmodified: `OvkPolicy::Sender`
/// change, `redact_pczt_for_signer(Full)` (the empty Sapling bundle keeps its
/// anchor and `bsk`, the Ironwood `bsk` stays) and the recipient string the
/// wallet showed its user stamped on the payment as `user_address`.
pub fn build_stock_sdk_view() -> Vec<u8> {
    let bytes = build_with_network(
        local_network(),
        600_000,
        390_000,
        MemoBytes::empty(),
        47,
        OutputPolicy {
            change_ovk_scope: None,
            full_view: true,
            ..OutputPolicy::standard(false)
        },
    );
    let mut value = json(&bytes);
    let i = payment(&value);
    value["ironwood"]["actions"][i]["output"]["user_address"] =
        serde_json::Value::String("u1stocksdkrecipientstringshowntotheuser".into());
    encode(value)
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

#[derive(Clone)]
struct OutputPolicy {
    self_payment: bool,
    payment_ovk_scope: Option<Scope>,
    change_ovk_scope: Option<Scope>,
    /// The change output's memo; `None` is the empty marker.
    change_memo: Option<MemoBytes>,
    /// Keep what `redact_pczt_for_signer(Full)` keeps (the empty Sapling
    /// bundle with its anchor and `bsk`, the Ironwood `bsk`) instead of the
    /// device-profile redaction `finish` applies.
    full_view: bool,
}

impl OutputPolicy {
    const fn standard(self_payment: bool) -> Self {
        Self {
            self_payment,
            payment_ovk_scope: Some(Scope::External),
            change_ovk_scope: Some(Scope::Internal),
            change_memo: None,
            full_view: false,
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
                output_policy
                    .change_ovk_scope
                    .map(|scope| fvk.to_ovk(scope)),
                fvk.address_at(1u32, Scope::Internal),
                Zatoshis::from_u64(change).unwrap(),
                output_policy
                    .change_memo
                    .clone()
                    .unwrap_or_else(MemoBytes::empty),
            )
            .unwrap();
    }
    if output_policy.full_view {
        finish_full_view(builder, &mut rng)
    } else {
        finish(builder, &mut rng)
    }
}

fn finalize<P: Parameters>(builder: DeferredPcztBuilder<P>, rng: &mut ChaCha20Rng) -> Pczt {
    let result = builder
        .build_for_pczt(rng, &zip317::FeeRule::standard())
        .unwrap();
    IoFinalizer::new(Creator::build_from_parts(result.pczt_parts).unwrap())
        .finalize_io()
        .unwrap()
}

/// `redact_pczt_for_signer(SignerView::Full)` keeps everything the device
/// grammar now admits and ignores; only the spend witnesses are cleared, as
/// the Full view clears them.
fn finish_full_view<P: Parameters>(
    builder: DeferredPcztBuilder<P>,
    rng: &mut ChaCha20Rng,
) -> Vec<u8> {
    Redactor::new(finalize(builder, rng))
        .redact_ironwood_with(|mut ironwood| {
            ironwood.redact_actions(|mut action| action.clear_spend_witness());
        })
        .finish()
        .serialize()
        .unwrap()
}

fn finish<P: Parameters>(builder: DeferredPcztBuilder<P>, rng: &mut ChaCha20Rng) -> Vec<u8> {
    let pczt = finalize(builder, rng);
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
/// 1..=CORPUS_MAX_ACTIONS sweep is representative. When `MAX_ACTIONS` was
/// raised 8→32 for 16/32-action signing, this stayed 8 so the equivalence
/// corpus keeps its 135-case size; 16/32-action parse+sign+verify is proven on
/// the emulator (host-side pczt verify), not by this host corpus.
pub const CORPUS_MAX_ACTIONS: usize = 8;

/// Synthetic local-consensus fixture covering every admitted action count.
///
/// The equivalence corpus asks for at most [`CORPUS_MAX_ACTIONS`]; the arena
/// model in `region_budget.rs` asks for [`MAX_ACTIONS`], because the scratch
/// tier has to hold the widest session the wire admits and nothing smaller
/// proves it does. A bundle of `outputs` actions costs `outputs` host Orchard
/// output builds, so only that one test pays for the wide end.
///
/// The fee is ZIP-317's, `5_000 * actions`, which is over the 100_000 zat
/// limit the corpus policy carries from 21 actions up: a caller past that has
/// to raise `maximum_fee` with the count.
pub fn build_actions(outputs: usize) -> Vec<u8> {
    assert!((1..=MAX_ACTIONS).contains(&outputs));
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

/// A canonical 25-byte P2PKH `scriptPubKey`: `76 a9 14 <hash160> 88 ac`.
pub fn p2pkh(hash160: [u8; 20]) -> Vec<u8> {
    let mut script = vec![0x76, 0xa9, 0x14];
    script.extend_from_slice(&hash160);
    script.extend_from_slice(&[0x88, 0xac]);
    script
}

/// A canonical 23-byte P2SH `scriptPubKey`: `a9 14 <hash160> 87`.
pub fn p2sh(hash160: [u8; 20]) -> Vec<u8> {
    let mut script = vec![0xa9, 0x14];
    script.extend_from_slice(&hash160);
    script.push(0x87);
    script
}

/// One transparent output in the v2 JSON view.
pub fn transparent_output_json(value: u64, script_pubkey: &[u8]) -> Value {
    serde_json::json!({
        "value": value,
        "script_pubkey": script_pubkey,
        "redeem_script": Value::Null,
        "bip32_derivation": {},
        "user_address": Value::Null,
        "proprietary": {},
    })
}

/// The fixture's transparent bundle replaced by `outputs`.
pub fn with_transparent_bundle(bytes: &[u8], outputs: Vec<Value>) -> Vec<u8> {
    let mut value = json(bytes);
    value["transparent"] = serde_json::json!({ "inputs": [], "outputs": outputs });
    encode(value)
}

/// The scripts `build_transparent` uses: P2PKH and P2SH alternating, so both
/// admitted shapes and both CompactSize lengths appear in every fixture with
/// more than one output.
pub fn transparent_script_for(index: usize) -> Vec<u8> {
    let hash = [0xa0 + index as u8; 20];
    if index.is_multiple_of(2) {
        p2pkh(hash)
    } else {
        p2sh(hash)
    }
}

/// The transparent payment values `build_transparent` uses.
pub fn transparent_values(count: usize) -> Vec<u64> {
    (0..count).map(|i| 100_000 + i as u64 * 10_000).collect()
}

/// Fee of every `build_transparent` fixture, whatever its transparent bundle.
pub const TRANSPARENT_FIXTURE_FEE: u64 = 10_000;
pub const TRANSPARENT_FIXTURE_PAYMENT: u64 = 600_000;
pub const TRANSPARENT_FIXTURE_CHANGE: u64 = 390_000;

/// A fee rule that charges exactly what it was built with.
///
/// `DeferredPcztBuilder::build_for_pczt` requires the shielded value balance
/// to equal the fee rule's fee, and it knows nothing about transparent
/// outputs (`transparent: None` is hard-coded in `zcash_primitives 0.30.1`,
/// so there is no builder path to a v6 PCZT with a transparent bundle). A
/// *balanced* deshield fixture -- one whose Ironwood `value_sum` really is
/// `fee + Σ transparent outputs`, which is the identity the device checks --
/// therefore means telling the builder the whole amount that leaves the
/// shielded pool and splicing the transparent bundle in afterwards through
/// the v2 JSON view, exactly as every other mutation fixture here does.
struct FixedFee(u64);

impl zcash_primitives::transaction::fees::FeeRule for FixedFee {
    type Error = core::convert::Infallible;

    #[allow(clippy::too_many_arguments)]
    fn fee_required<P: Parameters>(
        &self,
        _params: &P,
        _target_height: zcash_protocol::consensus::BlockHeight,
        _transparent_input_sizes: impl IntoIterator<
            Item = zcash_primitives::transaction::fees::transparent::InputSize,
        >,
        _transparent_output_sizes: impl IntoIterator<Item = usize>,
        _sapling_input_count: usize,
        _sapling_output_count: usize,
        _orchard_action_count: usize,
        _ironwood_action_count: usize,
    ) -> std::result::Result<Zatoshis, Self::Error> {
        Ok(Zatoshis::from_u64(self.0).expect("fixture fee within the money range"))
    }
}

/// A deshield: one Ironwood spend funding a shielded payment, shielded change
/// and the transparent `outputs`, each `(value, scriptPubKey)`.
///
/// `funded` is what the shielded side releases beyond the fee. Pass the sum of
/// the output values for a balanced transaction -- the Ironwood `value_sum` is
/// then `TRANSPARENT_FIXTURE_FEE + Σ values`, so the device's identity
/// `value_sum == fee + Σ transparent outputs` holds and the fee it computes is
/// `TRANSPARENT_FIXTURE_FEE` whatever the bundle is. Passing anything else
/// builds a transaction whose accounting does not add up, which is what the
/// identity has to catch.
///
/// The transparent bundle is spliced in BEFORE `finalize_io`, because that is
/// what signs the dummy spend, and a dummy signature is over the sighash the
/// transparent outputs are part of.
pub fn build_deshield(outputs: &[(u64, Vec<u8>)], funded: u64) -> Vec<u8> {
    deshield(outputs, funded, Shielded::PaymentAndChange)
}

/// The shielded side of a deshield fixture: what the one Ironwood spend funds
/// besides the transparent outputs and the fee. Both shapes release the same
/// value, so [`TRANSPARENT_FIXTURE_FEE`] is the fee either way; they differ
/// only in how many Ironwood actions carry it.
enum Shielded {
    /// A payment to another wallet and change back: TWO actions, the shape
    /// every transparent-output test uses.
    PaymentAndChange,
    /// One payment carrying both amounts, in an unpadded bundle: ONE action.
    /// Transparent outputs and Ironwood actions share one ZIP-317 cap
    /// (`MAX_TRANSPARENT_OUTPUTS = MAX_ACTIONS - 1`), so this is the only
    /// shielded side that leaves room for a full transparent bundle.
    OnePayment,
}

/// A balanced deshield with [`MAX_TRANSPARENT_OUTPUTS`] transparent outputs
/// and one Ironwood action: the widest transparent bundle the device admits,
/// and the case that pre-sizes the projection's transparent `Vec` to its cap.
pub fn build_wide_deshield() -> Vec<u8> {
    let values = transparent_values(MAX_TRANSPARENT_OUTPUTS);
    let outputs: Vec<(u64, Vec<u8>)> = values
        .iter()
        .enumerate()
        .map(|(i, value)| (*value, transparent_script_for(i)))
        .collect();
    deshield(&outputs, values.iter().sum(), Shielded::OnePayment)
}

fn deshield(outputs: &[(u64, Vec<u8>)], funded: u64, shielded: Shielded) -> Vec<u8> {
    let mut rng = ChaCha20Rng::from_seed([0xd0 ^ outputs.len() as u8; 32]);
    let (fvk, _) = keys();
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    let input =
        TRANSPARENT_FIXTURE_PAYMENT + TRANSPARENT_FIXTURE_CHANGE + TRANSPARENT_FIXTURE_FEE + funded;
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
            fvk.address_at(0u32, Scope::External),
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
        match shielded {
            Shielded::PaymentAndChange => BundlePadding::DEFAULT,
            // One spend paired with one output is one action only if nothing
            // pads the bundle back up.
            Shielded::OnePayment => BundlePadding::UNPADDED,
        },
    )
    .unwrap();
    builder
        .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
        .unwrap();
    builder
        .add_ironwood_output::<zip317::FeeError>(
            Some(fvk.to_ovk(Scope::External)),
            other.address_at(0u32, Scope::External),
            Zatoshis::from_u64(match shielded {
                Shielded::PaymentAndChange => TRANSPARENT_FIXTURE_PAYMENT,
                Shielded::OnePayment => TRANSPARENT_FIXTURE_PAYMENT + TRANSPARENT_FIXTURE_CHANGE,
            })
            .unwrap(),
            MemoBytes::empty(),
        )
        .unwrap();
    if matches!(shielded, Shielded::PaymentAndChange) {
        builder
            .add_ironwood_output::<zip317::FeeError>(
                Some(fvk.to_ovk(Scope::Internal)),
                fvk.address_at(1u32, Scope::Internal),
                Zatoshis::from_u64(TRANSPARENT_FIXTURE_CHANGE).unwrap(),
                MemoBytes::empty(),
            )
            .unwrap();
    }
    let result = builder
        .build_for_pczt(&mut rng, &FixedFee(TRANSPARENT_FIXTURE_FEE + funded))
        .unwrap();
    let mut pczt = Creator::build_from_parts(result.pczt_parts).unwrap();
    if !outputs.is_empty() {
        let bytes = pczt.clone().serialize().unwrap();
        let rows = outputs
            .iter()
            .map(|(value, script)| transparent_output_json(*value, script))
            .collect();
        pczt = Pczt::parse(&with_transparent_bundle(&bytes, rows)).unwrap();
    }
    let pczt = IoFinalizer::new(pczt).finalize_io().unwrap();
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

/// A balanced deshield paying `values` with [`transparent_script_for`].
pub fn build_transparent(values: &[u64]) -> Vec<u8> {
    let outputs: Vec<(u64, Vec<u8>)> = values
        .iter()
        .enumerate()
        .map(|(i, value)| (*value, transparent_script_for(i)))
        .collect();
    build_deshield(&outputs, values.iter().sum())
}

/// The fixture with one transparent input alongside its outputs: the shape
/// the device refuses structurally, because zero transparent inputs is what
/// makes ZIP-244 §S.2 collapse to §T.2.
pub fn with_transparent_bundle_and_input(bytes: &[u8]) -> Vec<u8> {
    let mut value = json(bytes);
    let outputs = value["transparent"]["outputs"].clone();
    value["transparent"] = serde_json::json!({
        "inputs": [{
            "prevout_txid": vec![0x11u8; 32],
            "prevout_index": 0,
            "sequence": Value::Null,
            "required_time_lock_time": Value::Null,
            "required_height_lock_time": Value::Null,
            "script_sig": Value::Null,
            "value": 1_000_000,
            "script_pubkey": p2pkh([0x22; 20]),
            "redeem_script": Value::Null,
            "partial_signatures": {},
            "sighash_type": 1,
            "bip32_derivation": {},
            "ripemd160_preimages": {},
            "sha256_preimages": {},
            "hash160_preimages": {},
            "hash256_preimages": {},
            "proprietary": {},
        }],
        "outputs": outputs,
    });
    encode(value)
}
