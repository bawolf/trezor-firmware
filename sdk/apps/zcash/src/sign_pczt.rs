//! Streamed PCZT signing. The app requests the PCZT from the host in chunks
//! and feeds them to a [`Session`], confirming each payment once it is
//! verified. The totals screen is the consent, after which the app returns a
//! signature for every real spend.

use alloc::string::String;
use alloc::vec::Vec;

use orchard::keys::{FullViewingKey, SpendAuthorizingKey, SpendingKey};
use prost::Message;
use rand_core::{CryptoRng, Error as RngError, RngCore};
use trezor_app_sdk::ui::{Progress, Property};
use trezor_app_sdk::{Error, Result, crypto, wire_request_raw};
use zcash_signer::{
    Event, Hedged, MAX_PCZT_BYTES, Memo, Network, OutputKind, Policy, Review, ReviewedOutput,
    Session, Summary, TransparentOutput,
};
use zeroize::Zeroize;

use crate::helpers::{account_label, format_zec_amount};
use crate::keys::{Wiped, account_keys, spending_key};
use crate::layout::{
    Source, confirm_memo, confirm_output, confirm_total, show_warning, with_keep_alive,
};
use crate::paths::{account_from_path, format_path};
use crate::proto::messages::MessageType;
use crate::proto::zcash::spend_auth_signatures::SpendAuthSignature;
use crate::proto::zcash::{PcztAck, PcztRequest, SignPczt, SpendAuthSignatures};
use crate::strutil::hex_encode;
use crate::{uformat, unified};

const CHUNK_BYTES: u32 = 1024;
const TRANSFER_ID_BYTES: usize = 16;

/// The app's limits: a fee of at most 0.01 ZEC, and an expiry at most this
/// many blocks after the host's reference height.
const MAX_FEE: u64 = 1_000_000;
const EXPIRY_WINDOW: u32 = 100;

/// Covers `sign`, the deepest of the calls that leave copies of the keys on
/// the stack.
const WIPED_STACK_BYTES: usize = 24 * 1024;

/// Reviews the PCZT the host streams and returns its spend authorization
/// signatures once the user consents to the totals.
pub fn sign_pczt(msg: SignPczt) -> Result<SpendAuthSignatures> {
    let (network, account) = account_from_path(&msg.address_n)?;
    let pczt_length = msg.pczt_length;
    if !(1..=MAX_PCZT_BYTES).contains(&(pczt_length as usize)) {
        return Err(Error::DataError("Invalid PCZT length"));
    }
    let policy = Policy::new(
        network,
        account,
        msg.host_reference_height,
        MAX_FEE,
        EXPIRY_WINDOW,
    )
    .map_err(rejected)?;

    let keys = account_keys(network, account)?;
    let label = account_label(network, account);
    let path = format_path(network, account);
    let source = Source {
        label: &label,
        path: &path,
    };
    // The bar tracks the PCZT bytes verified.
    let mut progress = Progress::show(Some(tr!("progress__loading_transaction")), None, false)?;
    let (spending_key, fvk) = prepare(&keys, &mut progress)?;
    let mut session = begin(policy, pczt_length, &keys, &fvk).map_err(rejected)?;
    drop(keys);
    wipe_stack();

    let mut transfer_id = [0; TRANSFER_ID_BYTES];
    crypto::random_bytes(&mut transfer_id);
    let mut review = None;
    // Transparent outputs come first; the numbering continues across both.
    let mut number = 0;
    let mut warned_transparent = false;
    let mut offset = 0;
    while offset < pczt_length {
        let length = CHUNK_BYTES.min(pczt_length - offset);
        let chunk = request_chunk(&transfer_id, offset, length)?;
        let mut fed = 0;
        while fed < chunk.len() {
            // Also brings the progress screen back after a confirmation.
            let verified = u64::from(offset) + fed as u64;
            progress.report((1000 * verified / u64::from(pczt_length)) as u32)?;
            let (consumed, event) = feed(&mut session, &chunk[fed..], &fvk, &mut progress)?;
            fed += consumed;
            match event {
                Event::ConfirmTransparentOutput(output) => {
                    // Once per transaction.
                    if !warned_transparent {
                        show_warning(
                            tr!("zcash__transparent_payment_warning"),
                            "zcash_transparent_payment",
                        )?;
                        warned_transparent = true;
                    }
                    confirm_transparent_output(network, &output, number, &source)?;
                    number += 1;
                }
                Event::ConfirmOutput {
                    output,
                    user_address,
                } => {
                    // The session offers only payments; anything else is a bug.
                    if output.kind != OutputKind::Payment {
                        return Err(rejected(zcash_signer::Error::Internal));
                    }
                    confirm_payment(network, &output, user_address, number, &source)?;
                    number += 1;
                }
                Event::Review(reviewed) => review = Some(reviewed),
                // The session made no progress.
                Event::NeedMore if consumed == 0 => {
                    return Err(rejected(zcash_signer::Error::Internal));
                }
                Event::NeedMore => {}
            }
        }
        offset += length;
    }
    drop(progress);

    let review = review.ok_or(rejected(zcash_signer::Error::Internal))?;
    confirm_totals(review.summary(), network, &source)?;
    session.approve(review.token()).map_err(rejected)?;
    let signatures = sign(&mut session, &review, &spending_key)?;
    drop(spending_key);
    wipe_stack();
    Ok(SpendAuthSignatures {
        transfer_id: transfer_id.to_vec(),
        signatures,
    })
}

// The functions below are out of line so that their frames are not on the
// stack while the session verifies an action.

/// Builds the long-lived caches first, so that they sit low in the heap.
#[inline(never)]
fn prepare(
    keys: &crypto::Zip32OrchardAccount,
    progress: &mut Progress,
) -> Result<(Wiped<SpendingKey>, Wiped<FullViewingKey>)> {
    with_keep_alive(progress, zcash_signer::prewarm)?;
    let spending_key = with_keep_alive(progress, |tick| spending_key(keys, tick))??;
    let fvk = Wiped::new(FullViewingKey::from(&*spending_key));
    progress.keep_alive()?;
    Ok((spending_key, fvk))
}

/// Starts a session for the account. Nonces are hedged with the spending key,
/// a secret only this device holds.
#[inline(never)]
fn begin(
    policy: Policy,
    pczt_length: u32,
    keys: &crypto::Zip32OrchardAccount,
    fvk: &FullViewingKey,
) -> zcash_signer::Result<Session<Hedged<DeviceRng>>> {
    let mut session = Session::new(policy, Hedged::new(DeviceRng, &keys.spending_key))?;
    session.begin(pczt_length as usize, fvk, &keys.seed_fingerprint)?;
    Ok(session)
}

/// Signs every real spend.
#[inline(never)]
fn sign(
    session: &mut Session<Hedged<DeviceRng>>,
    review: &Review,
    spending_key: &SpendingKey,
) -> Result<Vec<SpendAuthSignature>> {
    let mut progress = Progress::show(Some(tr!("progress__signing_transaction")), None, false)?;
    let ask = Wiped::new(SpendAuthorizingKey::from(spending_key));
    let signatures = with_keep_alive(&mut progress, |tick| {
        session.sign(review.token(), &ask, tick)
    })?
    .map_err(rejected)?;
    drop(ask);
    progress.report(1000)?;

    Ok(signatures
        .records()
        .iter()
        .map(|record| SpendAuthSignature {
            action_index: record.action_index.into(),
            signature: record.signature.to_vec(),
        })
        .collect())
}

/// Overwrites the stack below the caller's frame, where deriving and using
/// the keys leaves copies of them.
#[inline(never)]
fn wipe_stack() {
    let mut stack = [0u8; WIPED_STACK_BYTES];
    stack.zeroize();
    core::hint::black_box(&stack);
}

#[inline(never)]
fn feed(
    session: &mut Session<Hedged<DeviceRng>>,
    bytes: &[u8],
    fvk: &FullViewingKey,
    progress: &mut Progress,
) -> Result<(usize, Event)> {
    with_keep_alive(progress, |tick| session.feed(bytes, fvk, tick))?.map_err(rejected)
}

/// Requests `length` bytes at `offset`. A `Cancel` in place of the chunk
/// cancels signing.
fn request_chunk(
    transfer_id: &[u8; TRANSFER_ID_BYTES],
    offset: u32,
    length: u32,
) -> Result<Vec<u8>> {
    let request = PcztRequest {
        transfer_id: transfer_id.to_vec(),
        offset,
        length,
        diagnostics: diagnostics(),
    };
    // Not `wire_request`, which would decode a `Cancel` as a `PcztAck`.
    let (id, reply) = wire_request_raw(&request.encode_to_vec(), MessageType::PcztRequest as u16)?;
    match MessageType::try_from(i32::from(id)) {
        Ok(MessageType::PcztAck) => {}
        Ok(MessageType::Cancel) => return Err(Error::Cancelled),
        _ => return Err(Error::DataError("Unexpected message")),
    }
    // The echoed transfer id and offset bind the chunk to this request.
    let invalid = || Error::DataError("Invalid PCZT chunk");
    let ack = PcztAck::decode(reply.as_slice()).map_err(|_| invalid())?;
    let data = ack.data.ok_or_else(invalid)?;
    if ack.transfer_id.as_deref() != Some(transfer_id.as_slice())
        || ack.offset != Some(offset)
        || data.len() != length as usize
    {
        return Err(invalid());
    }
    Ok(data)
}

/// The app's counters so far, sent with every chunk request by a debug build,
/// so that the host keeps them if Core stops the app mid-request.
#[cfg(feature = "debug")]
fn diagnostics() -> Option<crate::proto::zcash::Diagnostics> {
    Some(crate::diagnostics_message(
        trezor_app_sdk::diagnostics::snapshot(),
    ))
}

#[cfg(not(feature = "debug"))]
fn diagnostics() -> Option<crate::proto::zcash::Diagnostics> {
    None
}

/// Names a malformed, oversized or out-of-range PCZT.
fn rejected(error: zcash_signer::Error) -> Error {
    match error {
        zcash_signer::Error::Malformed => Error::DataError("Malformed PCZT"),
        zcash_signer::Error::Capacity => Error::DataError("Too many transaction actions"),
        zcash_signer::Error::Amount => Error::DataError("Zcash amount out of range"),
        _ => Error::DataError("Zcash PCZT rejected"),
    }
}

/// The address shown for a verified receiver: the wallet's unified address if
/// its Orchard receiver is `receiver`, otherwise the Orchard-only address.
fn payment_address(
    network: Network,
    receiver: [u8; 43],
    user_address: Option<String>,
) -> Result<String> {
    let orchard_only = unified::address(network, receiver)?;
    let Some(user_address) = user_address else {
        return Ok(orchard_only);
    };
    // Shown as given, so the decoder must have read every character.
    if user_address.len() < orchard_only.len()
        || !user_address
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(Error::DataError("Invalid unified address"));
    }
    if unified::orchard_receiver(network, &user_address)? != Some(receiver) {
        return Err(Error::DataError(
            "Unified address does not match the Orchard receiver",
        ));
    }
    Ok(user_address)
}

/// A payment, then its memo if it has one.
#[inline(never)]
fn confirm_payment(
    network: Network,
    output: &ReviewedOutput,
    user_address: Option<String>,
    number: usize,
    source: &Source<'_>,
) -> Result<()> {
    let address = payment_address(network, output.receiver, user_address)?;
    confirm_output(
        &address,
        &format_zec_amount(output.value, network),
        number,
        source,
    )?;
    match &output.memo {
        Memo::Empty => Ok(()),
        Memo::Text(text) => confirm_memo(text.as_str(), false, number),
        Memo::Digest(digest) => {
            let digest = hex_encode(digest).map_err(|_| Error::DataError("Invalid memo"))?;
            confirm_memo(&digest, true, number)
        }
    }
}

/// A transparent output, shown by the address solved from its signed
/// `scriptPubKey`.
#[inline(never)]
fn confirm_transparent_output(
    network: Network,
    output: &TransparentOutput,
    number: usize,
    source: &Source<'_>,
) -> Result<()> {
    let address = unified::transparent_address(network, output.kind, output.hash);
    confirm_output(
        &address,
        &format_zec_amount(output.value, network),
        number,
        source,
    )
}

/// The totals: what leaves the wallet, the fee and the source account.
#[inline(never)]
fn confirm_totals(summary: &Summary, network: Network, source: &Source<'_>) -> Result<()> {
    let payment_outputs = summary
        .outputs
        .iter()
        .filter(|output| output.kind == OutputKind::Payment)
        .count();
    let transparent_outputs = summary.transparent_outputs.len();
    // ZIP-317 counts each transparent output as an action.
    let actions = summary.outputs.len() + summary.padding_outputs + transparent_outputs;
    let total = summary
        .payment_total
        .checked_add(summary.transparent_total)
        .and_then(|total| total.checked_add(summary.fee))
        .ok_or(rejected(zcash_signer::Error::Amount))?;

    let account_items = [
        Property::plain(tr!("words__account"), source.label),
        Property::plain(tr!("address_details__derivation_path"), source.path),
    ];
    let expiry_height = uformat!("{}", summary.expiry_height);
    let public_amount = format_zec_amount(summary.transparent_total, network);
    let outputs = uformat!("{}", payment_outputs + transparent_outputs);
    let actions = uformat!("{}", actions);
    let mut fee_items = Vec::new();
    fee_items.push(Property::plain(
        tr!("zcash__expires_at_block"),
        &expiry_height,
    ));
    if transparent_outputs > 0 {
        // Repeated here because the warning came before the amounts did.
        fee_items.push(Property::plain(tr!("zcash__public_amount"), &public_amount));
    }
    fee_items.push(Property::plain(tr!("words__outputs"), &outputs));
    fee_items.push(Property::plain(tr!("zcash__total_actions"), &actions));

    confirm_total(
        &format_zec_amount(total, network),
        &format_zec_amount(summary.fee, network),
        &account_items,
        &fee_items,
    )
}

/// The device's random number generator. Only ever used inside [`Hedged`].
struct DeviceRng;

impl RngCore for DeviceRng {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        crypto::random_bytes(destination);
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> core::result::Result<(), RngError> {
        self.fill_bytes(destination);
        Ok(())
    }
}

impl CryptoRng for DeviceRng {}
