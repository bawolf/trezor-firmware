//! Host tool for the emulator signing test: builds an Ironwood PCZT for a
//! wallet seed and verifies the device's signature records against it.
//!
//! `build` mirrors `tests/common/mod.rs::build_actions` (one funding note,
//! alternating payment and internal-change outputs, unpadded bundle) on the
//! real Mainnet or Testnet parameters, with keys derived from a ZIP-32 seed
//! exactly as the device derives them (`m/32'/coin_type'/account'`), so the
//! session accepts the wire FVK. `verify` applies each record with the pczt
//! crate's signer, which checks it against the action's `rk` and the
//! host-computed sighash: the same check a wallet performs.
//!
//! ```text
//! ironwood_fixture build <seed-hex> <mainnet|testnet> <account> <height> <outputs> <out> \
//!     [view=device|full|sdk] [zip32=none|own|other-seed|other-account]
//! ironwood_fixture verify <pczt-file> <records-file>
//! ```
//!
//! `view` selects what the host hands over: `device` (default) is the
//! device-profile redaction the desktop driver applies; `full` is
//! `redact_pczt_for_signer(SignerView::Full)` as a stock SDK wallet emits it
//! (the empty Sapling bundle keeps its anchor and `bsk`, the Ironwood `bsk`
//! stays); `sdk` is `full` plus the SDK's default `OvkPolicy::Sender` (change
//! encrypted with no OVK) and the recipient string stamped on every payment
//! as `user_address`.
//!
//! `zip32` puts a `zip32_derivation` claim on every real spend, as the SDK
//! does for an account imported as `Spending { seed_fingerprint, index }`:
//! `own` is the wallet's own seed fingerprint (`zip32::fingerprint`, the
//! canonical implementation) and `m/32'/coin_type'/account'`; `other-seed`
//! and `other-account` are claims the device must refuse.
//!
//! `build` prints a JSON summary (fvk, seed fingerprint, payments, fee,
//! expiry, shape) on stdout.

use std::fs;
use std::process::ExitCode;

use orchard::bundle::BundleVersion;
use orchard::keys::{FullViewingKey, Scope, SpendingKey};
use orchard::note_encryption::IronwoodDomain;
use orchard::value::NoteValue;
use pczt::Pczt;
use pczt::roles::creator::Creator;
use pczt::roles::io_finalizer::IoFinalizer;
use pczt::roles::redactor::Redactor;
use pczt::roles::signer::{Signer, SpendAuthSignature};
use pczt::v2::Pczt as PcztV2;
use rand_chacha::ChaCha20Rng;
use rand_chacha::rand_core::SeedableRng;
use serde_json::json;
use zcash_note_encryption::try_note_decryption;
use zcash_primitives::transaction::builder::{BundlePadding, DeferredPcztBuilder};
use zcash_primitives::transaction::fees::zip317;
use zcash_protocol::consensus::{MAIN_NETWORK, Parameters, TEST_NETWORK};
use zcash_protocol::memo::MemoBytes;
use zcash_protocol::value::Zatoshis;
use zip32::fingerprint::SeedFingerprint;

const RECORD_LEN: usize = 66;
const POOL_IRONWOOD: u8 = 0x03;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    let result = match args.get(1).map(String::as_str) {
        Some("build") if args.len() >= 8 => build(&args[2..]),
        Some("verify") if args.len() == 4 => verify(&args[2], &args[3]),
        _ => Err("usage: build <seed-hex> <mainnet|testnet> <account> <height> <outputs> <out> [view=device|full|sdk] [zip32=none|own|other-seed|other-account] | verify <pczt> <records>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn hex_decode(text: &str) -> Result<Vec<u8>, String> {
    if !text.len().is_multiple_of(2) {
        return Err("odd hex length".into());
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).map_err(|e| e.to_string()))
        .collect()
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// What the host hands the device (see the module docs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Device,
    Full,
    Sdk,
}

/// The `zip32_derivation` claim put on every real spend (see the module docs).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Zip32Claim {
    None,
    Own,
    OtherSeed,
    OtherAccount,
}

struct Options {
    view: View,
    zip32: Zip32Claim,
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options {
        view: View::Device,
        zip32: Zip32Claim::None,
    };
    for arg in args {
        match arg.split_once('=') {
            Some(("view", "device")) => options.view = View::Device,
            Some(("view", "full")) => options.view = View::Full,
            Some(("view", "sdk")) => options.view = View::Sdk,
            Some(("zip32", "none")) => options.zip32 = Zip32Claim::None,
            Some(("zip32", "own")) => options.zip32 = Zip32Claim::Own,
            Some(("zip32", "other-seed")) => options.zip32 = Zip32Claim::OtherSeed,
            Some(("zip32", "other-account")) => options.zip32 = Zip32Claim::OtherAccount,
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    Ok(options)
}

const ZIP32_HARDENED: u32 = 1 << 31;

fn build(args: &[String]) -> Result<(), String> {
    let seed = hex_decode(&args[0])?;
    let account: u32 = args[2].parse().map_err(|_| "bad account")?;
    let height: u32 = args[3].parse().map_err(|_| "bad height")?;
    let outputs: usize = args[4].parse().map_err(|_| "bad outputs")?;
    if !(1..=trezor_ironwood::MAX_ACTIONS).contains(&outputs) {
        return Err(format!(
            "outputs must be 1..={}",
            trezor_ironwood::MAX_ACTIONS
        ));
    }
    let options = parse_options(&args[6..])?;
    let (bytes, summary) = match args[1].as_str() {
        "mainnet" => build_with(MAIN_NETWORK, 133, &seed, account, height, outputs, &options)?,
        "testnet" => build_with(TEST_NETWORK, 1, &seed, account, height, outputs, &options)?,
        _ => return Err("network must be mainnet or testnet".into()),
    };
    fs::write(&args[5], bytes).map_err(|e| e.to_string())?;
    println!("{summary}");
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn build_with<P: Parameters>(
    network: P,
    coin_type: u32,
    seed: &[u8],
    account: u32,
    height: u32,
    outputs: usize,
    options: &Options,
) -> Result<(Vec<u8>, serde_json::Value), String> {
    // `zip32::AccountId` is inferred from the parameter; the crate has no zip32
    // dep.
    let sk = SpendingKey::from_zip32_seed(
        seed,
        coin_type,
        account.try_into().map_err(|_| "bad account")?,
    )
    .map_err(|e| format!("zip32: {e:?}"))?;
    let fvk = FullViewingKey::from(&sk);
    // The payee is a wallet this seed does not own. PUBLIC TEST KEY ONLY.
    let other = FullViewingKey::from(&SpendingKey::from_bytes([1; 32]).unwrap());
    let mut rng = ChaCha20Rng::from_seed([outputs as u8; 32]);

    // Fund one note exactly covering the outputs and the ZIP-317 fee, as
    // `build_actions` does, so the bundle has no extra change.
    let input = (1..=outputs as u64).sum::<u64>() * 100_000 + outputs.max(2) as u64 * 5_000;
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
        network,
        height.into(),
        BundlePadding::DEFAULT,
        BundlePadding::UNPADDED,
    )
    .unwrap();
    builder
        .add_ironwood_spend::<zip317::FeeError>(fvk.clone(), note)
        .unwrap();
    let mut payments = Vec::new();
    for i in 0..outputs {
        let value = (i as u64 + 1) * 100_000;
        let (recipient, ovk) = if i % 2 == 0 {
            let address = other.address_at(i as u32, Scope::External);
            payments.push(json!({
                "receiver": hex_encode(&address.to_raw_address_bytes()),
                "value": value,
            }));
            (address, Some(fvk.to_ovk(Scope::External)))
        } else {
            // `OvkPolicy::Sender`, the SDK default, gives change no OVK.
            let ovk = (options.view != View::Sdk).then(|| fvk.to_ovk(Scope::Internal));
            (fvk.address_at(i as u32, Scope::Internal), ovk)
        };
        builder
            .add_ironwood_output::<zip317::FeeError>(
                ovk,
                recipient,
                Zatoshis::from_u64(value).unwrap(),
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
    let redactor = Redactor::new(pczt);
    let redactor = match options.view {
        View::Device => redactor
            .redact_sapling_with(|mut sapling| {
                sapling.clear_bsk();
                sapling.clear_anchor();
            })
            .redact_ironwood_with(|mut ironwood| {
                ironwood.clear_bsk();
                ironwood.redact_actions(|mut action| action.clear_spend_witness());
            }),
        // `SignerView::Full` clears only the spend witnesses.
        View::Full | View::Sdk => redactor.redact_ironwood_with(|mut ironwood| {
            ironwood.redact_actions(|mut action| action.clear_spend_witness());
        }),
    };
    let mut bytes = redactor.finish().serialize().unwrap();
    // The v2 JSON view: the low-level fields are private, so `user_address`
    // is set through it, and the summary reports the shape from it so a test
    // can assert what it exercised.
    let v2_view = |bytes: &[u8]| -> Result<serde_json::Value, String> {
        serde_json::to_value(
            PcztV2::try_from(Pczt::parse(bytes).map_err(|e| format!("parse: {e:?}"))?)
                .map_err(|e| format!("v2: {e:?}"))?,
        )
        .map_err(|e| e.to_string())
    };
    if options.view == View::Sdk {
        // The SDK stamps the recipient string it showed the user on every
        // payment output; the device ignores it.
        let mut view = v2_view(&bytes)?;
        for action in view["ironwood"]["actions"]
            .as_array_mut()
            .ok_or("no ironwood actions")?
        {
            let value = action["output"]["value"].as_u64().unwrap_or(0);
            let external = payments.iter().any(|p| p["value"].as_u64() == Some(value));
            if value > 0 && external {
                action["output"]["user_address"] = json!(format!("u1payee{value}"));
            }
        }
        bytes = serde_json::from_value::<PcztV2>(view)
            .map_err(|e| e.to_string())?
            .serialize();
    }
    let seed_fingerprint = SeedFingerprint::from_seed(seed)
        .ok_or("seed must be 32..=252 bytes")?
        .to_bytes();
    if options.zip32 != Zip32Claim::None {
        let (fingerprint, account_index) = match options.zip32 {
            Zip32Claim::OtherSeed => {
                let mut other = seed.to_vec();
                other[0] ^= 0xff;
                (
                    SeedFingerprint::from_seed(&other).unwrap().to_bytes(),
                    account,
                )
            }
            Zip32Claim::OtherAccount => (seed_fingerprint, account + 1),
            _ => (seed_fingerprint, account),
        };
        let path = [
            32 | ZIP32_HARDENED,
            coin_type | ZIP32_HARDENED,
            account_index | ZIP32_HARDENED,
        ];
        let mut view = v2_view(&bytes)?;
        for action in view["ironwood"]["actions"]
            .as_array_mut()
            .ok_or("no ironwood actions")?
        {
            if action["spend"]["value"].as_u64().unwrap_or(0) > 0 {
                action["spend"]["zip32_derivation"] = json!({
                    "seed_fingerprint": fingerprint.to_vec(),
                    "derivation_path": path,
                });
            }
        }
        bytes = serde_json::from_value::<PcztV2>(view)
            .map_err(|e| e.to_string())?
            .serialize();
    }
    let view = v2_view(&bytes)?;
    let user_addresses = view["ironwood"]["actions"]
        .as_array()
        .ok_or("no ironwood actions")?
        .iter()
        .filter(|action| action["output"]["user_address"].is_string())
        .count();
    let shape = json!({
        "sapling_present": view["sapling"].is_object(),
        "ironwood_bsk_present": view["ironwood"]["bsk"].is_array(),
        "user_addresses": user_addresses,
    });

    let payment_total: u64 = payments.iter().map(|p| p["value"].as_u64().unwrap()).sum();
    let fee = outputs.max(2) as u64 * 5_000;
    let summary = json!({
        "fvk": hex_encode(&fvk.to_bytes()),
        "seed_fingerprint": hex_encode(&seed_fingerprint),
        "actions": outputs,
        "payments": payments,
        "payment_total": payment_total,
        "fee": fee,
        "expiry_height": height + 40,
        "pczt_length": bytes.len(),
        "shape": shape,
    });
    Ok((bytes, summary))
}

fn verify(pczt_path: &str, records_path: &str) -> Result<(), String> {
    let bytes = fs::read(pczt_path).map_err(|e| e.to_string())?;
    let records = fs::read(records_path).map_err(|e| e.to_string())?;
    if records.is_empty() || !records.len().is_multiple_of(RECORD_LEN) {
        return Err("records length is not a multiple of 66".into());
    }
    let pczt = Pczt::parse(&bytes).map_err(|e| format!("parse: {e:?}"))?;
    // Every real spend must be signed by exactly one record. Spend values are
    // read through the v2 JSON view, as tests/common/mod.rs does, because the
    // low-level action fields are private.
    let view = serde_json::to_value(
        pczt::v2::Pczt::try_from(pczt.clone()).map_err(|e| format!("v2: {e:?}"))?,
    )
    .map_err(|e| e.to_string())?;
    let real: Vec<usize> = view["ironwood"]["actions"]
        .as_array()
        .ok_or("no ironwood actions")?
        .iter()
        .enumerate()
        .filter(|(_, action)| action["spend"]["value"].as_u64().is_some_and(|v| v > 0))
        .map(|(index, _)| index)
        .collect();
    let mut signer = Signer::new(pczt).map_err(|e| format!("signer: {e:?}"))?;
    let mut signed = Vec::new();
    for record in records.chunks(RECORD_LEN) {
        if record[0] != POOL_IRONWOOD {
            return Err(format!("record pool {:#x} is not Ironwood", record[0]));
        }
        let index = usize::from(record[1]);
        let signature: [u8; 64] = record[2..].try_into().unwrap();
        signer
            .apply_orchard_spend_auth_signature(&SpendAuthSignature::from_parts(
                orchard::ValuePool::Ironwood,
                index,
                signature,
            ))
            .map_err(|e| format!("action {index}: signature rejected: {e:?}"))?;
        signed.push(index);
    }
    if signed != real {
        return Err(format!(
            "signed actions {signed:?} but real spends are {real:?}"
        ));
    }
    let _ = signer.finish();
    println!(
        "verified {} signature(s) for actions {signed:?}",
        signed.len()
    );
    Ok(())
}
