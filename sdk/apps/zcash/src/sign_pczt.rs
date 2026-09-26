//! Streamed Ironwood PCZT signing.
//!
//! The app pulls the PCZT from the host in 1,024-byte chunks and feeds them to
//! `ironwood::Session`, which never retains more than one action. Payment
//! outputs are confirmed as they arrive, like Bitcoin's signer; those
//! confirmations are not consent. Consent is the totals screen after the
//! whole PCZT verified, after which the app returns one signature record per
//! real spend. The rules are the `ironwood` crate's; "§N" is a section of its
//! design document (see the crate docs).

use alloc::string::String;
use alloc::vec::Vec;

use ironwood::receive::Network;
use ironwood::{
    Account, ErrorCode, Event, Hedged, Limits, Memo, OutputKind, Policy, Projection,
    RequestContext, Review, ReviewedOutput, Session, TransparentOutput,
};
use orchard::keys::{FullViewingKey, SpendAuthorizingKey, SpendingKey};
use prost::Message;
use rand_core::{CryptoRng, Error as RngError, RngCore};
use trezor_app_sdk::ui::{Progress, Property};
use trezor_app_sdk::{Error, Result, crypto, wire_request_raw};

use crate::account::{
    AccountKeys, Wiped, account_from_request, account_keys, account_label, account_path, malformed,
    network_from_request, network_label,
};
use crate::layout::{
    Source, confirm_memo, confirm_output, confirm_total, keeping_alive, show_warning,
    show_weak_backup_warning,
};
use crate::proto::messages::MessageType;
use crate::proto::zcash::{
    ZcashPcztAck, ZcashPcztRequest, ZcashSignPczt, ZcashSpendAuthSignatures,
};
use crate::unified;

/// Transfer limits shared with the host (`zcash.proto`).
const CHUNK_BYTES: u32 = 1024;
const TRANSFER_ID_BYTES: usize = 16;

/// Device-owned policy limits (`ironwood::Limits`): a fee of at most 0.01 ZEC,
/// and an expiry at most this many blocks past the host's reference height.
const MAXIMUM_FEE: u64 = 1_000_000;
const EXPIRY_WINDOW: u32 = 100;

/// A signature record: `pool ‖ action_index ‖ signature` (§1).
const POOL_IRONWOOD: u8 = 0x03;
const RECORD_BYTES: usize = 1 + 1 + 64;

const ZEC_DECIMALS: u32 = 8;

/// The request's selections, validated before any key is requested.
struct Request {
    network: Network,
    account: u32,
    pczt_length: u32,
    host_reference_height: u32,
}

impl Request {
    fn from_message(msg: ZcashSignPczt) -> Result<Self> {
        let request = Self {
            network: network_from_request(msg.network)?,
            account: account_from_request(msg.account)?,
            pczt_length: msg.pczt_length.ok_or_else(malformed)?,
            host_reference_height: msg.host_reference_height.ok_or_else(malformed)?,
        };
        if !(1..=ironwood::MAX_PCZT_BYTES as u32).contains(&request.pczt_length) {
            return Err(Error::DataError("Invalid PCZT length"));
        }
        Ok(request)
    }
}

/// Reviews the PCZT the host streams and returns the spend authorization
/// signatures once the user consents to the totals.
pub(crate) fn sign_pczt(msg: ZcashSignPczt) -> Result<ZcashSpendAuthSignatures> {
    let request = Request::from_message(msg)?;
    let keys = account_keys(request.network, request.account)?;
    if keys.weak_backup {
        show_weak_backup_warning()?;
    }
    let label = account_label(request.account);
    let path = account_path(request.network, request.account);
    let source = Source {
        label: &label,
        path: &path,
    };

    // Up before the session starts: the bar tracks PCZT bytes verified.
    let mut progress = Progress::show(Some(tr!("progress__loading_transaction")), None, false)?;
    let (spending_key, fvk) = prepare(&keys, &mut progress)?;
    let mut session = begin(&request, &keys, &fvk).map_err(rejected)?;
    // Wipes the raw key bytes once the session has its hedge seed.
    drop(keys);

    let mut transfer_id = [0; TRANSFER_ID_BYTES];
    crypto::random_bytes(&mut transfer_id);
    let mut review = None;
    // One running number across both halves of the payment: the transparent
    // outputs stream before the shielded actions, so they are outputs #1..#N
    // and the shielded payments continue from there.
    let mut number = 0;
    let mut warned_transparent = false;
    let mut offset = 0;
    while offset < request.pczt_length {
        let length = CHUNK_BYTES.min(request.pczt_length - offset);
        let chunk = request_chunk(&transfer_id, offset, length)?;
        let mut fed = 0;
        while fed < chunk.len() {
            // Also brings the progress screen back after a confirmation.
            let verified = u64::from(offset) + fed as u64;
            progress.report((1000 * verified / u64::from(request.pczt_length)) as u32)?;
            // One chunk may complete several outputs; each comes back
            // separately, and the rest is fed again after its confirmation.
            let (consumed, event) = feed(&mut session, &chunk[fed..], &fvk, &mut progress)?;
            fed += consumed;
            match event {
                Event::ConfirmTransparentOutput(output) => {
                    if !warned_transparent {
                        // Once per transaction, before the first transparent
                        // output: a deshield to several recipients is still one
                        // decision about privacy.
                        show_warning(
                            tr!("zcash__transparent_payment_warning"),
                            "zcash_transparent_payment",
                        )?;
                        warned_transparent = true;
                    }
                    confirm_transparent_output(request.network, &output, number, &source)?;
                    number += 1;
                }
                Event::ConfirmOutput {
                    output,
                    user_address,
                } => {
                    // The session offers only payments; change and padding are
                    // proved to be the device's own and are never shown. A
                    // regression in the session stops here, not on a screen.
                    if output.kind != OutputKind::Payment {
                        return Err(rejected(ErrorCode::State));
                    }
                    confirm_payment(request.network, &output, user_address, number, &source)?;
                    number += 1;
                }
                Event::Review(reviewed) => review = Some(reviewed),
                Event::None if consumed == 0 => return Err(rejected(ErrorCode::State)),
                Event::None => {}
            }
        }
        offset += length;
    }
    drop(progress);

    let review = review.ok_or(rejected(ErrorCode::State))?;
    confirm_totals(review.projection(), &request, &label, &path)?;
    session.approve(review.token()).map_err(rejected)?;

    Ok(ZcashSpendAuthSignatures {
        transfer_id: transfer_id.to_vec(),
        records: sign(&mut session, &review, spending_key)?,
    })
}

// The functions marked `#[inline(never)]` below keep their locals off the
// stack while the session verifies an action, the app's deepest call chain:
// inlined into `sign_pczt`, the screens, key setup and signing overflowed the
// 32 KiB stack there on the device.

/// Builds the persistent Pasta and orchard caches before the session's first
/// allocation, so they sit low in the heap, then checks the account's spending
/// key and derives its full viewing key.
#[inline(never)]
fn prepare(
    keys: &AccountKeys,
    progress: &mut Progress,
) -> Result<(Wiped<SpendingKey>, Wiped<FullViewingKey>)> {
    keeping_alive(progress, ironwood::prewarm_with_progress)?;
    // `orchard` refuses the rare key bytes that are not a valid spending key.
    let spending_key = keeping_alive(progress, |tick| {
        SpendingKey::from_bytes_with_progress(keys.spending_key, tick)
    })?;
    let spending_key =
        Wiped(Option::from(spending_key).ok_or(Error::DataError("Zcash key derivation failed"))?);
    let fvk = Wiped(FullViewingKey::from(&*spending_key));
    progress.keep_alive()?;
    Ok((spending_key, fvk))
}

/// One RedPallas signature per real spend, as `pool ‖ action_index ‖
/// signature` records. The spending key is dropped once `ask` is derived.
#[inline(never)]
fn sign(
    session: &mut Session<Hedged<DeviceRng>>,
    review: &Review,
    spending_key: Wiped<SpendingKey>,
) -> Result<Vec<u8>> {
    let mut progress = Progress::show(Some(tr!("progress__signing_transaction")), None, false)?;
    let ask = Wiped(SpendAuthorizingKey::from(&*spending_key));
    drop(spending_key);
    let signatures = keeping_alive(&mut progress, |tick| {
        session.sign_with_progress(review.token(), &ask, tick)
    })?
    .map_err(rejected)?;
    drop(ask);
    progress.report(1000)?;

    let mut records = Vec::with_capacity(signatures.records().len() * RECORD_BYTES);
    for record in signatures.records() {
        records.push(POOL_IRONWOOD);
        records.push(record.action_index);
        records.extend_from_slice(&record.signature);
    }
    if records.is_empty() {
        return Err(Error::DataError("Zcash signing failed"));
    }
    Ok(records)
}

/// Starts a session for the account's full viewing key. The nonce hedge is
/// keyed on the spending key, a secret only this device holds.
fn begin(
    request: &Request,
    keys: &AccountKeys,
    fvk: &FullViewingKey,
) -> ironwood::Result<Session<Hedged<DeviceRng>>> {
    let network = match request.network {
        Network::Mainnet => ironwood::Network::Mainnet,
        Network::Testnet => ironwood::Network::Testnet,
    };
    let policy = Policy::new(
        RequestContext::new(
            network,
            Account::new(request.account)?,
            request.host_reference_height,
        ),
        Limits::new(MAXIMUM_FEE, EXPIRY_WINDOW)?,
    )?;
    let mut session = Session::with_rng(policy, Hedged::from_seed(DeviceRng, &keys.spending_key))?;
    session.begin(request.pczt_length as usize, fvk, &keys.seed_fingerprint)?;
    Ok(session)
}

/// Feeds `bytes` to the session, reporting progress while it verifies.
#[inline(never)]
fn feed(
    session: &mut Session<Hedged<DeviceRng>>,
    bytes: &[u8],
    fvk: &FullViewingKey,
    progress: &mut Progress,
) -> Result<(usize, Event)> {
    keeping_alive(progress, |tick| {
        session.feed_with_progress(bytes, fvk, tick)
    })?
    .map_err(rejected)
}

/// Pulls `length` bytes at `offset` from the host. A `ZcashCancel` in place of
/// the chunk cancels the request.
fn request_chunk(
    transfer_id: &[u8; TRANSFER_ID_BYTES],
    offset: u32,
    length: u32,
) -> Result<Vec<u8>> {
    let request = ZcashPcztRequest {
        transfer_id: transfer_id.to_vec(),
        offset,
        length,
        diagnostics: diagnostics(),
    };
    // Not `wire_request`, which decodes whatever the host answers as the
    // expected response: the id tells a cancel from a chunk.
    let (id, reply) = wire_request_raw(
        &request.encode_to_vec(),
        MessageType::ZcashPcztRequest as u16,
    )?;
    match MessageType::try_from(i32::from(id)) {
        Ok(MessageType::ZcashPcztAck) => {}
        Ok(MessageType::ZcashCancel) => return Err(Error::Cancelled),
        _ => return Err(Error::DataError("Unexpected message")),
    }
    let invalid = || Error::DataError("Invalid PCZT chunk");
    let ack = ZcashPcztAck::decode(reply.as_slice()).map_err(|_| invalid())?;
    let data = ack.data.ok_or_else(invalid)?;
    if ack.transfer_id.as_deref() != Some(transfer_id.as_slice())
        || ack.offset != Some(offset)
        || data.len() != length as usize
    {
        return Err(invalid());
    }
    Ok(data)
}

/// The app's counters so far, sent with every chunk request by a debug build:
/// if Core stops the app mid-request (a silence over 1 s, or a fault), the
/// host still has the latest values.
#[cfg(feature = "debug")]
fn diagnostics() -> Option<crate::proto::zcash::ZcashDiagnostics> {
    Some(crate::diagnostics_message(
        trezor_app_sdk::diagnostics::snapshot(),
    ))
}

#[cfg(not(feature = "debug"))]
fn diagnostics() -> Option<crate::proto::zcash::ZcashDiagnostics> {
    None
}

/// The wire failure of a session error: a malformed, oversized or out-of-range
/// PCZT is named; any other refusal has no detail.
fn rejected(code: ErrorCode) -> Error {
    match code {
        ErrorCode::Malformed => Error::DataError("Malformed PCZT"),
        ErrorCode::Capacity => Error::DataError("Too many transaction actions (max 32)"),
        ErrorCode::Amount => Error::DataError("Zcash amount out of range"),
        _ => Error::DataError("Zcash PCZT rejected"),
    }
}

/// The address to show for a verified payment receiver (§7): the wallet's
/// unified address if its Orchard receiver is `receiver`, the Orchard-only
/// address if the wallet sent none.
fn payment_address(
    network: Network,
    receiver: [u8; 43],
    user_address: Option<String>,
) -> Result<String> {
    let orchard_only = unified::address(network, receiver)?;
    let Some(user_address) = user_address else {
        return Ok(orchard_only);
    };
    // The string is shown as given, so the decoder must have checked every
    // character of it: letters and digits only, and no shorter than the
    // shortest address that can carry this receiver.
    if user_address.len() < orchard_only.len()
        || !user_address
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(Error::DataError("Invalid unified address."));
    }
    if unified::orchard_receiver(network, &user_address)? != Some(receiver) {
        return Err(Error::DataError(
            "Unified address does not match the Orchard receiver.",
        ));
    }
    Ok(user_address)
}

/// A payment output (§7), then its memo if it has one.
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
        &format_amount(output.value, network),
        number,
        source,
    )?;
    match &output.memo {
        Memo::Empty => Ok(()),
        Memo::Text(text) => confirm_memo(text.as_str(), false, number),
        Memo::Digest(digest) => confirm_memo(&hex(digest), true, number),
    }
}

/// A transparent output, shown by the address the device solved from its
/// signed `scriptPubKey` (§8); the wallet's `user_address` is ignored.
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
        &format_amount(output.value, network),
        number,
        source,
    )
}

/// Bitcoin's totals screen: what leaves the wallet (payments plus fee), the
/// fee and the source account, with the digest-bound expiry height, the
/// public part of the payment, and the true size of the transaction. The
/// host's reference height is a policy input only and is not shown.
#[inline(never)]
fn confirm_totals(
    projection: &Projection,
    request: &Request,
    account_label: &str,
    path: &str,
) -> Result<()> {
    let network = request.network;
    let payment_outputs = projection
        .outputs
        .iter()
        .filter(|output| output.kind == OutputKind::Payment)
        .count();
    let transparent_outputs = projection.transparent_outputs.len();
    // ZIP-317 counts a standard transparent output as one logical action, and
    // the device charges it against the same 32-action cap: payments, change,
    // padding and transparent outputs.
    let actions = projection.outputs.len() + projection.padding_outputs + transparent_outputs;
    let total = projection
        .payment_total
        .checked_add(projection.transparent_total)
        .and_then(|total| total.checked_add(projection.fee))
        .ok_or(rejected(ErrorCode::Amount))?;

    let account = uformat!("Zcash {} {}", network_label(network), account_label);
    let account_items = [
        Property::plain(tr!("words__account"), &account),
        Property::plain(tr!("address_details__derivation_path"), path),
    ];
    let expiry_height = uformat!("{}", projection.expiry_height);
    let public_amount = format_amount(projection.transparent_total, network);
    let outputs = uformat!("{}", payment_outputs + transparent_outputs);
    let actions = uformat!("{}", actions);
    let mut fee_items = Vec::with_capacity(4);
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
        &format_amount(total, network),
        &format_amount(projection.fee, network),
        &account_items,
        &fee_items,
    )
}

/// "1,234.5 ZEC" (TAZ on testnet), as Core's `format_amount` with 8 decimals.
fn format_amount(zatoshis: u64, network: Network) -> String {
    let scale = 10u64.pow(ZEC_DECIMALS);
    let digits = uformat!("{}", zatoshis / scale);
    let mut amount = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index) % 3 == 0 {
            amount.push(',');
        }
        amount.push(digit);
    }
    let fraction = zatoshis % scale;
    if fraction > 0 {
        let fraction = uformat!("{}", scale + fraction);
        amount.push('.');
        amount.push_str(fraction[1..].trim_end_matches('0'));
    }
    amount.push(' ');
    amount.push_str(match network {
        Network::Mainnet => "ZEC",
        Network::Testnet => "TAZ",
    });
    amount
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(2 * bytes.len());
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0xf)]));
    }
    text
}

/// The device's random number generator. Only ever handed to the session
/// inside [`Hedged`], so a nonce depends on the key as well as on the RNG.
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
