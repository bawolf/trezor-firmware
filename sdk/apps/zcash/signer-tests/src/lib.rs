//! PCZT builders and a [`Session`] driver for the tests of `zcash-signer`.
//!
//! PCZTs are built with librustzcash: the `zcash_primitives` builder and the
//! `pczt` roles. They are edited through the `pczt` v2 serde view. Everything
//! is seeded, so a built PCZT is the same on every run.

use core::convert::Infallible;

use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use orchard::note::{NoteVersion, RandomSeed, Rho};
use orchard::note_encryption::IronwoodDomain;
use orchard::value::NoteValue;
use orchard::{Address, Note};
use pasta_curves::group::ff::PrimeField;
use pasta_curves::pallas;
use pczt::Pczt;
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::redactor::Redactor;
use pczt::roles::signer::{Signer, SpendAuthSignature};
use pczt::roles::verifier::{OrchardError, Verifier};
use rand_chacha::ChaCha20Rng;
use rand_core::{CryptoRng, RngCore, SeedableRng};
use serde_json::{Value, json};
use zcash_note_encryption::{Domain, EphemeralKeyBytes};
use zcash_primitives::transaction::builder::{BundlePadding, DeferredPcztBuilder};
use zcash_primitives::transaction::fees::{FeeRule, transparent::InputSize};
use zcash_protocol::consensus::{self, BlockHeight, Parameters};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zcash_signer::{
    Event, Network, Policy, Result, Review, ReviewedOutput, Session, Signatures, TransparentOutput,
};

/// An edit of a PCZT's serde view.
pub type Edit = fn(&mut Value);

/// How streaming a PCZT ends: in a review, or refused.
pub type Outcome = Result<()>;

/// The host's reference height; NU6.3 is active on both networks.
pub const HEIGHT: u32 = 10_000_000;
/// The fee and expiry limits of [`Wallet::policy`].
pub const MAX_FEE: u64 = 1_000_000;
pub const EXPIRY_WINDOW: u32 = 100;

/// Chunk sizes a PCZT is fed in: byte by byte, small, 1 KiB, whole.
pub const CHUNKINGS: [usize; 4] = [1, 64, 1024, usize::MAX];

const HARDENED: u32 = 1 << 31;

/// A public test seed; never funded.
const TEST_SEED: [u8; 32] = [7; 32];
const TEST_ACCOUNT: u32 = 9;
/// The seed of the wallet that payments go to.
const OTHER_SEED: [u8; 32] = [8; 32];

/// An account's keys, and the policy a session signs it under.
pub struct Wallet {
    pub network: Network,
    pub account: u32,
    pub fvk: FullViewingKey,
    pub ask: SpendAuthorizingKey,
    pub seed_fingerprint: [u8; 32],
}

impl Wallet {
    pub fn from_seed(seed: &[u8], network: Network, account: u32) -> Self {
        let sk = SpendingKey::from_zip32_seed(
            seed,
            network.coin_type(),
            zip32::AccountId::try_from(account).unwrap(),
        )
        .unwrap();
        Self {
            network,
            account,
            fvk: FullViewingKey::from(&sk),
            ask: SpendAuthorizingKey::from(&sk),
            seed_fingerprint: zip32::fingerprint::SeedFingerprint::from_seed(seed)
                .unwrap()
                .to_bytes(),
        }
    }

    pub fn testnet() -> Self {
        Self::from_seed(&TEST_SEED, Network::Testnet, TEST_ACCOUNT)
    }

    pub fn mainnet() -> Self {
        Self::from_seed(&TEST_SEED, Network::Mainnet, TEST_ACCOUNT)
    }

    /// `m/32'/coin_type'/account'`.
    pub fn account_path(&self) -> [u32; 3] {
        [
            32 | HARDENED,
            self.network.coin_type() | HARDENED,
            self.account | HARDENED,
        ]
    }

    pub fn policy(&self) -> Policy {
        Policy::new(self.network, self.account, HEIGHT, MAX_FEE, EXPIRY_WINDOW).unwrap()
    }

    pub fn session(&self) -> Session<ChaCha20Rng> {
        self.session_with(self.policy(), 0)
    }

    pub fn session_with(&self, policy: Policy, seed: u8) -> Session<ChaCha20Rng> {
        Session::new(policy, ChaCha20Rng::from_seed([seed; 32])).unwrap()
    }

    /// Starts a request of `declared` bytes for this account.
    pub fn begin<R: RngCore + CryptoRng>(
        &self,
        session: &mut Session<R>,
        declared: usize,
    ) -> Result<()> {
        session.begin(declared, &self.fvk, &self.seed_fingerprint)
    }

    /// Where an output of `to` goes.
    pub fn address(&self, to: To) -> Address {
        match to {
            To::Other(index) => Wallet::from_seed(&OTHER_SEED, self.network, 0)
                .fvk
                .address_at(index, Scope::External),
            To::External(index) => self.fvk.address_at(index, Scope::External),
            To::Internal(index) => self.fvk.address_at(index, Scope::Internal),
        }
    }
}

/// The recipient of an output, by diversifier index.
#[derive(Clone, Copy, Debug)]
pub enum To {
    /// Another wallet.
    Other(u32),
    /// One of the account's own external addresses.
    External(u32),
    /// One of the account's internal (change) addresses.
    Internal(u32),
}

#[derive(Clone, Debug)]
pub struct Output {
    pub to: To,
    pub value: u64,
    pub memo: MemoBytes,
    /// Which of the account's OVKs encrypts the output to the sender.
    pub ovk: Option<Scope>,
    pub user_address: Option<String>,
}

impl Output {
    pub fn payment(value: u64) -> Self {
        Self {
            to: To::Other(0),
            value,
            memo: MemoBytes::empty(),
            ovk: Some(Scope::External),
            user_address: None,
        }
    }

    pub fn change(value: u64) -> Self {
        Self {
            to: To::Internal(1),
            ovk: Some(Scope::Internal),
            ..Self::payment(value)
        }
    }
}

/// Which fields the host leaves in the PCZT it sends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// librustzcash's `SignerView::Full`: only the spend witnesses are removed,
    /// so the empty Sapling bundle and both binding keys stay.
    Full,
    /// Also without anchors and binding keys.
    Bare,
}

/// A transaction from the wallet's notes. The notes' value that the Ironwood
/// outputs do not take leaves the bundle: the transparent outputs, then the
/// fee.
#[derive(Clone, Debug)]
pub struct Tx {
    pub notes: Vec<u64>,
    pub outputs: Vec<Output>,
    /// `(value, scriptPubKey)`.
    pub transparent: Vec<(u64, Vec<u8>)>,
    /// Padded to two actions, as wallets do, rather than one per spend or
    /// output.
    pub padded: bool,
    pub expiry: u32,
    pub view: View,
    pub seed: u8,
}

impl Tx {
    /// One note paying for `outputs`, the transparent outputs and `fee`.
    pub fn pay(outputs: Vec<Output>, transparent: Vec<(u64, Vec<u8>)>, fee: u64) -> Self {
        let total = outputs.iter().map(|output| output.value).sum::<u64>()
            + transparent.iter().map(|(value, _)| value).sum::<u64>()
            + fee;
        Self {
            notes: vec![total],
            outputs,
            transparent,
            padded: true,
            expiry: HEIGHT + 40,
            view: View::Bare,
            seed: 0,
        }
    }

    /// A payment of 600_000 with 390_000 change and a 10_000 fee.
    pub fn simple() -> Self {
        Self::pay(
            vec![Output::payment(600_000), Output::change(390_000)],
            Vec::new(),
            10_000,
        )
    }

    /// [`Tx::simple`] from two notes, so without a dummy spend, whose signature
    /// covers the sighash.
    pub fn two_notes() -> Self {
        Self {
            notes: vec![500_000, 500_000],
            ..Self::simple()
        }
    }
}

/// The ZIP-317 fee of `logical_actions`: 5_000 each, for at least two.
pub fn zip317_fee(logical_actions: usize) -> u64 {
    5_000 * logical_actions.max(2) as u64
}

/// A built PCZT, and the action each note and output landed in.
pub struct Built {
    pub bytes: Vec<u8>,
    pub spends: Vec<usize>,
    pub outputs: Vec<usize>,
    /// The key of each dummy spend, by action.
    pub dummies: Vec<(usize, SpendingKey)>,
}

/// Charges whatever leaves the shielded pool: the builder knows nothing of
/// transparent outputs, which are added to the PCZT afterwards.
struct Released(u64);

impl FeeRule for Released {
    type Error = Infallible;

    fn fee_required<P: Parameters>(
        &self,
        _: &P,
        _: BlockHeight,
        _: impl IntoIterator<Item = InputSize>,
        _: impl IntoIterator<Item = usize>,
        _: usize,
        _: usize,
        _: usize,
        _: usize,
    ) -> core::result::Result<Zatoshis, Infallible> {
        Ok(Zatoshis::from_u64(self.0).unwrap())
    }
}

pub fn build(wallet: &Wallet, tx: &Tx) -> Built {
    let mut rng = ChaCha20Rng::from_seed([tx.seed; 32]);
    let params = match wallet.network {
        Network::Mainnet => consensus::Network::MainNetwork,
        Network::Testnet => consensus::Network::TestNetwork,
    };
    let padding = if tx.padded {
        BundlePadding::DEFAULT
    } else {
        BundlePadding::UNPADDED
    };
    let mut builder = DeferredPcztBuilder::new::<Infallible>(
        params,
        HEIGHT.into(),
        BundlePadding::DEFAULT,
        padding,
    )
    .unwrap()
    .with_expiry_height(tx.expiry.into());
    for (index, value) in tx.notes.iter().enumerate() {
        let note = fund(wallet.address(To::External(index as u32)), *value, &mut rng);
        builder
            .add_ironwood_spend::<Infallible>(wallet.fvk.clone(), note)
            .unwrap();
    }
    for output in &tx.outputs {
        builder
            .add_ironwood_output::<Infallible>(
                output.ovk.map(|scope| wallet.fvk.to_ovk(scope)),
                wallet.address(output.to),
                Zatoshis::from_u64(output.value).unwrap(),
                output.memo.clone(),
            )
            .unwrap();
    }
    let released = tx.notes.iter().sum::<u64>() - tx.outputs.iter().map(|o| o.value).sum::<u64>();
    let result = builder
        .build_for_pczt(&mut rng, &Released(released))
        .unwrap();
    let spends = (0..tx.notes.len())
        .map(|i| result.ironwood_meta.spend_action_index(i).unwrap())
        .collect();
    let outputs: Vec<usize> = (0..tx.outputs.len())
        .map(|i| result.ironwood_meta.output_action_index(i).unwrap())
        .collect();

    let mut pczt = Creator::build_from_parts(result.pczt_parts).unwrap();
    if !tx.transparent.is_empty() {
        // Before IO finalization, whose dummy signatures cover them.
        let bytes = mutate(&serialize(pczt), |value| {
            let outputs: Vec<Value> = tx
                .transparent
                .iter()
                .map(|(value, script)| transparent_output_json(*value, script))
                .collect();
            value["transparent"] = json!({ "inputs": [], "outputs": outputs });
        });
        pczt = Pczt::parse(&bytes).unwrap();
    }
    let (pczt, dummies) = finalize(pczt, &mut rng);
    let pczt = redact(pczt, tx.view);
    let mut value = json(&serialize(pczt));
    for (output, index) in tx.outputs.iter().zip(&outputs) {
        if let Some(address) = &output.user_address {
            value["ironwood"]["actions"][*index]["output"]["user_address"] = json!(address);
        }
    }
    Built {
        bytes: encode(value),
        spends,
        outputs,
        dummies,
    }
}

/// A note of `value` to `address`, as if received earlier.
fn fund(address: Address, value: u64, rng: &mut ChaCha20Rng) -> Note {
    loop {
        let mut bytes = [0; 64];
        rng.fill_bytes(&mut bytes);
        let rho: Option<Rho> = Rho::from_bytes(&bytes[..32].try_into().unwrap()).into();
        let Some(rho) = rho else { continue };
        let rseed: Option<RandomSeed> =
            RandomSeed::from_bytes(bytes[32..].try_into().unwrap(), &rho).into();
        let note = rseed.and_then(|rseed| {
            Note::from_parts(
                address,
                NoteValue::from_raw(value),
                rho,
                rseed,
                NoteVersion::V3,
            )
            .into()
        });
        if let Some(note) = note {
            return note;
        }
    }
}

/// The IO Finalizer, with its dummy spends signed by `rng` instead of the OS
/// RNG so that the PCZT is reproducible. Returns the dummy spends' keys.
fn finalize(pczt: Pczt, rng: &mut ChaCha20Rng) -> (Pczt, Vec<(usize, SpendingKey)>) {
    let unfinalized = json(&serialize(pczt.clone()));
    let mut signer = Signer::new(IoFinalizer::new(pczt).finalize_io().unwrap()).unwrap();
    let sighash = signer.shielded_sighash();
    let mut dummies = Vec::new();
    for (index, action) in actions(&unfinalized).iter().enumerate() {
        if action["spend"]["dummy_sk"].is_null() {
            continue;
        }
        let sk = SpendingKey::from_bytes(byte_array(&action["spend"]["dummy_sk"])).unwrap();
        let alpha = pallas::Scalar::from_repr(byte_array(&action["spend"]["alpha"])).unwrap();
        let signature = SpendAuthorizingKey::from(&sk)
            .randomize(&alpha)
            .sign(&mut *rng, &sighash);
        signer.apply_ironwood_signature(index, signature).unwrap();
        dummies.push((index, sk));
    }
    (signer.finish(), dummies)
}

fn redact(pczt: Pczt, view: View) -> Pczt {
    let redactor = Redactor::new(pczt);
    match view {
        View::Full => redactor.redact_ironwood_with(|mut ironwood| {
            ironwood.redact_actions(|mut action| {
                action.clear_spend_witness();
                action.clear_spend_dummy_sk();
            })
        }),
        View::Bare => redactor
            .redact_sapling_with(|mut sapling| {
                sapling.clear_bsk();
                sapling.clear_anchor();
            })
            .redact_ironwood_with(|mut ironwood| {
                ironwood.clear_bsk();
                ironwood.clear_anchor();
                ironwood.redact_actions(|mut action| action.clear_spend_witness());
            }),
    }
    .finish()
}

fn serialize(pczt: Pczt) -> Vec<u8> {
    pczt.serialize().unwrap()
}

/// The `pczt` v2 serde view of `bytes`.
pub fn json(bytes: &[u8]) -> Value {
    serde_json::to_value(pczt::v2::Pczt::try_from(Pczt::parse(bytes).unwrap()).unwrap()).unwrap()
}

pub fn encode(value: Value) -> Vec<u8> {
    serde_json::from_value::<pczt::v2::Pczt>(value)
        .unwrap()
        .serialize()
}

/// `bytes` with `edit` applied to its serde view.
pub fn mutate(bytes: &[u8], edit: impl FnOnce(&mut Value)) -> Vec<u8> {
    let mut value = json(bytes);
    edit(&mut value);
    encode(value)
}

pub fn actions(value: &Value) -> &Vec<Value> {
    value["ironwood"]["actions"].as_array().unwrap()
}

/// The first action `which` selects.
pub fn action(value: &mut Value, which: impl Fn(&Value) -> bool) -> &mut Value {
    let index = actions(value).iter().position(which).unwrap();
    &mut value["ironwood"]["actions"][index]
}

/// The action of [`Tx::simple`]'s payment.
pub fn payment_action(value: &mut Value) -> &mut Value {
    action(value, |a| a["output"]["value"] == 600_000)
}

/// The shielded sighash that the `pczt` Signer computes.
pub fn sighash(bytes: &[u8]) -> [u8; 32] {
    Signer::new(Pczt::parse(bytes).unwrap())
        .unwrap()
        .shielded_sighash()
}

/// The BLAKE2b-256 of a memo's 512 bytes, which the device shows for a memo
/// it does not show as text.
pub fn memo_digest(memo: &MemoBytes) -> [u8; 32] {
    blake2b_simd::Params::new()
        .hash_length(32)
        .hash(memo.as_array())
        .as_bytes()
        .try_into()
        .unwrap()
}

/// Where two encodings first differ: the offset of the field that tells them
/// apart.
pub fn offset_of_difference(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).position(|(a, b)| a != b).unwrap()
}

/// The canonical LEB128 varint of `value`.
pub fn varint(mut value: u64) -> Vec<u8> {
    let mut bytes = Vec::new();
    while value >= 0x80 {
        bytes.push(value as u8 | 0x80);
        value >>= 7;
    }
    bytes.push(value as u8);
    bytes
}

/// Flips the low bit of a byte array's first byte.
pub fn flip(value: &mut Value) {
    value[0] = (value[0].as_u64().unwrap() ^ 1).into();
}

pub fn byte_array<const N: usize>(value: &Value) -> [u8; N] {
    serde_json::from_value::<Vec<u8>>(value.clone())
        .unwrap()
        .try_into()
        .unwrap()
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
        .collect()
}

/// The actions whose spend has a value: the ones the device signs.
pub fn real_spends(bytes: &[u8]) -> Vec<usize> {
    let value = json(bytes);
    actions(&value)
        .iter()
        .enumerate()
        .filter(|(_, action)| action["spend"]["value"].as_u64().unwrap() > 0)
        .map(|(index, _)| index)
        .collect()
}

/// `bytes` with the OCK of every output that has a value, under the OVK of the
/// output's scope.
pub fn with_ocks(bytes: &[u8], fvk: &FullViewingKey) -> Vec<u8> {
    let mut ocks = Vec::new();
    Verifier::new(Pczt::parse(bytes).unwrap())
        .with_ironwood(|bundle| -> core::result::Result<(), OrchardError<()>> {
            for action in bundle.actions() {
                let output = action.output();
                if output.value().unwrap().inner() == 0 {
                    ocks.push(None);
                    continue;
                }
                let scope = match fvk.scope_for_address(&output.recipient().unwrap()) {
                    Some(Scope::Internal) => Scope::Internal,
                    _ => Scope::External,
                };
                let ock = IronwoodDomain::derive_ock(
                    &fvk.to_ovk(scope),
                    action.cv_net(),
                    &output.cmx().to_bytes(),
                    &EphemeralKeyBytes(output.encrypted_note().epk_bytes),
                );
                ocks.push(Some(ock.0));
            }
            Ok(())
        })
        .unwrap();
    mutate(bytes, |value| {
        for (index, ock) in ocks.into_iter().enumerate() {
            value["ironwood"]["actions"][index]["output"]["ock"] = json!(ock);
        }
    })
}

/// A `zip32_derivation` in the serde view.
pub fn derivation_json(seed_fingerprint: &[u8; 32], path: &[u32]) -> Value {
    json!({ "seed_fingerprint": seed_fingerprint, "derivation_path": path })
}

/// `OP_DUP OP_HASH160 <hash> OP_EQUALVERIFY OP_CHECKSIG`.
pub fn p2pkh(hash: [u8; 20]) -> Vec<u8> {
    [&[0x76, 0xa9, 0x14][..], &hash, &[0x88, 0xac]].concat()
}

/// `OP_HASH160 <hash> OP_EQUAL`.
pub fn p2sh(hash: [u8; 20]) -> Vec<u8> {
    [&[0xa9, 0x14][..], &hash, &[0x87]].concat()
}

/// `count` transparent outputs of 100_000 plus 10_000 per index, alternating
/// P2PKH and P2SH.
pub fn transparent_outputs(count: usize) -> Vec<(u64, Vec<u8>)> {
    (0..count)
        .map(|i| {
            let hash = [0xa0 + i as u8; 20];
            let script = if i % 2 == 0 { p2pkh(hash) } else { p2sh(hash) };
            (100_000 + i as u64 * 10_000, script)
        })
        .collect()
}

pub fn transparent_output_json(value: u64, script_pubkey: &[u8]) -> Value {
    json!({
        "value": value,
        "script_pubkey": script_pubkey,
        "redeem_script": null,
        "bip32_derivation": {},
        "user_address": null,
        "proprietary": {},
    })
}

/// What a session showed for a PCZT.
#[derive(Debug, Default)]
pub struct Events {
    /// Each `ConfirmOutput`, with its `user_address`.
    pub payments: Vec<(ReviewedOutput, Option<String>)>,
    pub transparent: Vec<TransparentOutput>,
    pub review: Option<Review>,
}

impl Events {
    pub fn review(&self) -> &Review {
        self.review.as_ref().expect("reviewed")
    }
}

/// Streams `bytes` into `session` in `chunk`-byte pieces after declaring
/// `declared` bytes, feeding again whatever a call leaves unconsumed.
pub fn stream<R: RngCore + CryptoRng>(
    session: &mut Session<R>,
    wallet: &Wallet,
    bytes: &[u8],
    declared: usize,
    chunk: usize,
) -> Result<Events> {
    wallet.begin(session, declared)?;
    let mut events = Events::default();
    for piece in bytes.chunks(chunk) {
        let mut rest = piece;
        while !rest.is_empty() {
            let (consumed, event) = session.feed(rest, &wallet.fvk, &mut || {})?;
            // Only a held-back transparent output is returned without input.
            assert!(consumed > 0 || matches!(event, Event::ConfirmTransparentOutput(_)));
            rest = &rest[consumed..];
            assert!(events.review.is_none(), "an event after the review");
            match event {
                Event::NeedMore => {}
                Event::ConfirmOutput {
                    output,
                    user_address,
                } => events.payments.push((output, user_address)),
                Event::ConfirmTransparentOutput(output) => {
                    // The encoding puts the transparent bundle first.
                    assert!(events.payments.is_empty());
                    events.transparent.push(output);
                }
                Event::Review(review) => events.review = Some(review),
            }
        }
    }
    Ok(events)
}

/// Streams `bytes` into a new session of the wallet's policy.
pub fn review(wallet: &Wallet, bytes: &[u8], chunk: usize) -> Result<Events> {
    stream(&mut wallet.session(), wallet, bytes, bytes.len(), chunk)
}

/// Reviews, approves and signs `bytes` in `session`.
pub fn sign_with<R: RngCore + CryptoRng>(
    wallet: &Wallet,
    mut session: Session<R>,
    bytes: &[u8],
) -> Result<Signatures> {
    let events = stream(&mut session, wallet, bytes, bytes.len(), 1024)?;
    let token = events.review().token();
    session.approve(token)?;
    session.sign(token, &wallet.ask, &mut || {})
}

pub fn sign(wallet: &Wallet, bytes: &[u8]) -> Result<Signatures> {
    sign_with(wallet, wallet.session(), bytes)
}

/// Applies the signatures with the `pczt` Signer, which verifies each against
/// its action's `rk` and the sighash it computes itself. They must be one per
/// real spend.
pub fn assert_signatures_apply(bytes: &[u8], signatures: &Signatures) {
    let indices: Vec<usize> = signatures
        .records()
        .iter()
        .map(|record| usize::from(record.action_index))
        .collect();
    assert_eq!(indices, real_spends(bytes));
    let mut signer = Signer::new(Pczt::parse(bytes).unwrap()).unwrap();
    for record in signatures.records() {
        signer
            .apply_orchard_spend_auth_signature(&SpendAuthSignature::from_parts(
                orchard::ValuePool::Ironwood,
                usize::from(record.action_index),
                record.signature,
            ))
            .unwrap();
    }
}

/// Whether librustzcash accepts the PCZT: it parses, its effects extract, and
/// every action verifies against `fvk`.
pub fn librustzcash_verifies(bytes: &[u8], fvk: &FullViewingKey) -> bool {
    let Ok(pczt) = Pczt::parse(bytes) else {
        return false;
    };
    if Signer::new(pczt.clone()).is_err() {
        return false;
    }
    Verifier::new(pczt)
        .with_ironwood(|bundle| -> core::result::Result<(), OrchardError<()>> {
            for action in bundle.actions() {
                action.verify_cv_net()?;
                action.spend().verify_nullifier(Some(fvk))?;
                action.spend().verify_rk(Some(fvk))?;
                action.output().verify_note_commitment(action.spend())?;
            }
            Ok(())
        })
        .is_ok()
}

/// Asserts the outcome of streaming `bytes` under every chunking.
pub fn assert_outcome(wallet: &Wallet, name: &str, bytes: &[u8], expected: Outcome) {
    for chunk in CHUNKINGS {
        let outcome = review(wallet, bytes, chunk).map(|events| {
            assert!(events.review.is_some(), "{name}: no review");
        });
        assert_eq!(outcome, expected, "{name}: chunk {chunk}");
    }
}
