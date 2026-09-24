"""Streamed Ironwood PCZT signing.

The device pulls the PCZT in 1,024-byte chunks and feeds them to the native
streaming session (`trezorzcash.session_*`), which never retains more than
one action. Payment outputs are confirmed as they arrive, like Bitcoin's
signer; those confirmations are not consent. Consent is the totals screen
after the whole PCZT verified, after which the device returns one signature
record per real spend (docs/common/zcash-ironwood-signing.md §1-§2).
"""

from micropython import const
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from typing import TypeVar

    from trezor.messages import ZcashSignPczt, ZcashSpendAuthSignatures
    from trezor.protobuf import MessageType

    from apps.common.coininfo import CoinInfo

    from .helpers import SessionIdentity

    LoadedMessageType = TypeVar("LoadedMessageType", bound=MessageType)

# Transfer constants shared with the host client (messages-zcash.proto).
MAX_PCZT_BYTES = const(65536)
CHUNK_BYTES = const(1024)
TRANSFER_ID_BYTES = const(16)

# Device-owned policy limits (ironwood::Limits). The fee cap is 0.01 ZEC;
# the expiry window bounds how far past the host's reference height a
# transaction may stay valid.
MAXIMUM_FEE = const(1_000_000)
EXPIRY_WINDOW = const(100)

# Drops a host that stalls a single chunk. An abandoned sign is closed by
# autolock like any other workflow, and the `finally` in `sign_pczt` wipes the
# native session; progress reports keep an active sign's idle timer alive.
CHUNK_TIMEOUT_MS = const(5_000)

# Bytes of per-session scratch the native allocator carves the signing session
# from. Must match `trezorzcash.SCRATCH_BYTES`, which is the minimum
# `session_begin` accepts; the two move together.
SCRATCH_BYTES = const(48 * 1024)

# Curated, non-secret native ValueError messages allowed to reach the host
# verbatim. Any other ValueError (MicroPython unpack, parse_u32, format_amount,
# address encode, ...) is collapsed to MALFORMED so incidental text never leaks
# to the host. Must match `micropython/zcash.rs::failure`.
_MALFORMED = "Malformed PCZT"
_TOO_MANY_ACTIONS = "Too many transaction actions (max 32)"
_AMOUNT_OUT_OF_RANGE = "Zcash amount out of range"

# session_feed step kinds.
_STEP_OUTPUT = const(1)
_STEP_REVIEW = const(2)
_STEP_TRANSPARENT_OUTPUT = const(3)

# Base58Check version selector of a transparent output. Emitted by
# `session_feed` in core/embed/rust/src/micropython/zcash.rs, which picks
# the same 0/1 (`T_P2PKH` / `T_P2SH` in core/embed/rust/src/ironwood/
# signing.rs); the two lists must move together.
_T_P2PKH = const(0)
_T_P2SH = const(1)

# Memo kinds of an output step. Emitted by `session_feed` in
# core/embed/rust/src/micropython/zcash.rs, which picks the same 0/1/2;
# the two lists must move together. The native session recovers the memo
# from the signed ciphertext, classifies it and hands over only what is
# shown, so the 512 memo bytes never reach Python: nothing for an empty
# memo, the UTF-8 text of a memo within the 256-byte display budget, or the
# 32-byte BLAKE2b-256 of all 512 bytes for anything else (binary, reserved
# leading byte, not UTF-8, over the budget, or a character the device
# cannot draw as itself).
_MEMO_NONE = const(0)
_MEMO_TEXT = const(1)
_MEMO_DIGEST = const(2)

RECORD_LEN = const(66)
ZEC_DECIMALS = const(8)


async def _call(
    msg: MessageType, expected_type: type[LoadedMessageType]
) -> LoadedMessageType:
    from trezor import loop, wire
    from trezor.wire import context

    # The call runs as a child task of the race, so it has to carry the context.
    reply = await loop.race(
        context.with_context(context.get_context(), context.call(msg, expected_type)),
        loop.sleep(CHUNK_TIMEOUT_MS),
    )
    if not expected_type.is_type_of(reply):
        raise wire.ActionCancelled("PCZT transfer timed out")
    return reply


def _amount(zatoshis: int, shortcut: str) -> str:
    from trezor.strings import format_amount

    return f"{format_amount(zatoshis, ZEC_DECIMALS)} {shortcut}"


def _payment_address(receiver: bytes, user_address: str | None, coin: CoinInfo) -> str:
    """The address to show for a verified payment receiver (§7): the wallet's
    unified address if its Orchard receiver is `receiver`, the Orchard-only
    address if the wallet sent none. Raises DataError otherwise."""
    from trezor import wire

    from .unified_addresses import Typecode, decode, encode

    orchard_only = encode({Typecode.ORCHARD: receiver}, coin)
    if user_address is None:
        return orchard_only
    # The string is shown as given, so the decoder must check every character
    # of it: letters and digits only (it stops at a NUL), and no shorter than
    # the shortest address that can carry this receiver.
    if len(user_address) < len(orchard_only) or not all(
        "0" <= c <= "9" or "a" <= c <= "z" or "A" <= c <= "Z" for c in user_address
    ):
        raise wire.DataError("Invalid unified address.")
    if decode(user_address, coin).get(Typecode.ORCHARD) != receiver:
        raise wire.DataError("Unified address does not match the Orchard receiver.")
    return user_address


async def _confirm_output(
    receiver: bytes,
    user_address: str | None,
    value: int,
    number: int,
    coin_name: str,
    account_label: str,
    path: str,
) -> None:
    from trezor.ui import layouts

    from apps.common import coininfo

    coin = coininfo.by_name(coin_name)
    address = _payment_address(receiver, user_address, coin)
    await layouts.confirm_output(
        address,
        _amount(value, coin.coin_shortcut),
        output_index=number,
        # Always chunked: a unified address (106+ characters) is not
        # comparable unbroken on a consent screen.
        chunkify=True,
        source_account=account_label,
        source_account_path=path,
    )


def _transparent_address(kind: int, hash160: bytes, coin_name: str) -> str:
    """The `t1…`/`t3…` (`tm…`/`t2…` on testnet) address of a transparent
    output, from the 20-byte hash the device solved out of the signed
    `scriptPubKey`. The version bytes are `coininfo`'s (7352/7357 mainnet,
    7461/7354 testnet) and the encoder is the one Bitcoin signing already
    uses; no new crypto ships for this screen."""
    from trezor.crypto import base58

    from apps.common import address_type, coininfo

    coin = coininfo.by_name(coin_name)
    if kind == _T_P2PKH:
        version = coin.address_type
    elif kind == _T_P2SH:
        version = coin.address_type_p2sh
    else:
        raise ValueError(_MALFORMED)
    if version is None:
        raise ValueError(_MALFORMED)
    return base58.encode_check(address_type.tobytes(version) + hash160, coin.b58_hash)


async def _confirm_transparent_output(
    kind: int,
    hash160: bytes,
    value: int,
    number: int,
    coin_name: str,
    account_label: str,
    path: str,
) -> None:
    from trezor.ui import layouts

    from apps.common import coininfo

    coin = coininfo.by_name(coin_name)
    await layouts.confirm_output(
        _transparent_address(kind, hash160, coin_name),
        _amount(value, coin.coin_shortcut),
        output_index=number,
        # A transparent address is 35 characters, not 106, but grouping it in
        # fours is how `ZcashGetAddress` and `_confirm_output` already render
        # what the user must compare, and the screen must not change its
        # habits between the shielded and transparent halves of one payment.
        chunkify=True,
        source_account=account_label,
        source_account_path=path,
    )


async def _warn_transparent(session: SessionIdentity) -> None:
    """One screen per transaction, before the first transparent output.

    Not per output: a deshield with several recipients is still one decision
    about privacy. Its own ButtonRequest name so a host can neither suppress
    it nor mistake it for an output confirmation."""
    from trezor import TR
    from trezor.enums import ButtonRequestType
    from trezor.ui.layouts import show_warning

    from . import helpers

    await show_warning(
        br_name="zcash_transparent_payment",
        content=TR.zcash__transparent_payment_warning,
        br_code=ButtonRequestType.Warning,
    )
    helpers.require_session(session)


async def _confirm_memo(memo_kind: int, memo: bytes, number: int) -> None:
    from trezor import TR
    from trezor.enums import ButtonRequestType
    from trezor.ui import layouts
    from trezor.utils import hexlify_if_bytes

    title = f"{TR.words__recipient} #{number + 1}"
    if memo_kind == _MEMO_TEXT:
        # UTF-8 validated natively; at most 256 bytes.
        await layouts.confirm_value(
            title,
            memo.decode(),
            TR.zcash__memo,
            br_name="confirm_memo",
            br_code=ButtonRequestType.ConfirmOutput,
            verb=TR.buttons__continue,
            is_data=False,
        )
    elif memo_kind == _MEMO_DIGEST:
        # Binary, reserved, non-UTF-8 or over the budget: the BLAKE2b-256 of
        # the 512 memo bytes, which the wallet can recompute.
        await layouts.confirm_value(
            title,
            hexlify_if_bytes(memo),
            TR.zcash__memo_hash,
            br_name="confirm_memo",
            br_code=ButtonRequestType.ConfirmOutput,
            verb=TR.buttons__continue,
            is_data=True,
        )
    else:
        raise ValueError(_MALFORMED)


async def _confirm_totals(
    totals: tuple,
    coin_name: str,
    network_label: str,
    account_label: str,
    path: str,
) -> None:
    from trezor import TR
    from trezor.ui import layouts

    from apps.common import coininfo

    coin = coininfo.by_name(coin_name)
    (
        expiry_height,
        _blocks_until_expiry,
        _input_total,
        payment_total,
        _change_total,
        transparent_total,
        fee,
        _padding_outputs,
        payment_outputs,
        transparent_outputs,
        action_count,
    ) = totals
    fee_items = [
        (TR.zcash__expires_at_block, str(expiry_height), None),
        (TR.words__outputs, str(payment_outputs + transparent_outputs), None),
        # ZIP-317 counts a standard transparent output as one logical action,
        # and the device charges it against the same 32-action hard cap, so
        # this is the sum. Shown so the user sees the true size of what they
        # authorize (payments + change + padding + transparent outputs), not
        # just the visible payments.
        (TR.zcash__total_actions, str(action_count + transparent_outputs), None),
    ]
    if transparent_outputs:
        # How much of the payment is public. Repeated here because the warning
        # screen came before the amounts did.
        fee_items.insert(
            1,
            (
                TR.zcash__public_amount,
                _amount(transparent_total, coin.coin_shortcut),
                None,
            ),
        )
    # Bitcoin's totals screen: what leaves the wallet (payments plus fee), the
    # fee, and the source account. The host reference height is a policy input
    # only and is deliberately not shown; the digest-bound expiry height is.
    await layouts.confirm_total(
        _amount(payment_total + transparent_total + fee, coin.coin_shortcut),
        _amount(fee, coin.coin_shortcut),
        account_items=[
            (TR.words__account, f"Zcash {network_label} {account_label}", None),
            (TR.address_details__derivation_path, path, None),
        ],
        fee_items=fee_items,
    )


async def sign_pczt(msg: ZcashSignPczt) -> ZcashSpendAuthSignatures:
    from trezor import TR, utils, wire
    from trezor.enums import ButtonRequestType
    from trezor.ui.layouts import show_warning

    from apps.common import seed

    from . import helpers

    if not utils.USE_ZCASH_SHIELDED:
        raise wire.ProcessError("Zcash shielded support is not enabled")

    network = msg.network  # local_cache_attribute
    account = msg.account  # local_cache_attribute
    pczt_length = msg.pczt_length  # local_cache_attribute
    host_reference_height = msg.host_reference_height  # local_cache_attribute

    coin_name, network_label, coin_type = helpers.validate_network_account(
        network, account
    )
    if type(pczt_length) is not int or type(host_reference_height) is not int:
        raise wire.DataError(helpers.MALFORMED_REQUEST)
    if not 1 <= pczt_length <= MAX_PCZT_BYTES:
        raise wire.DataError("Invalid PCZT length")
    if not 0 <= host_reference_height <= 0xFFFF_FFFF:
        raise wire.DataError(helpers.MALFORMED_REQUEST)

    seed.raise_if_not_initialized()
    session = helpers.snapshot_session()
    account_label = helpers.account_label(account)
    path = helpers.account_path(coin_type, account)

    if helpers.has_weak_backup():
        await show_warning(
            br_name="zcash_weak_backup",
            content=TR.zcash__weak_backup_warning,
            br_code=ButtonRequestType.Warning,
        )
        helpers.require_session(session)

    wallet_seed = await seed.get_seed()
    helpers.require_session(session)

    # `_stream_and_sign` writes the native handle here as soon as it has one,
    # so the `finally` below can cancel *its* session rather than whatever is
    # live. Before `session_begin` returns the list is empty and the cancel is
    # blind, which is what a pre-begin failure needs.
    handle_out: list[int] = []

    # The session's own allocations are carved from this buffer, not from the
    # native region: only what must outlive the session (Pasta's square-root
    # table, orchard's commitment-domain caches) is boot-rooted, and that is
    # 40 KiB of AUX1 the GC heap never had. The scratch is borrowed from the
    # heap for the length of one sign and given back below, so every other
    # coin's workflow runs at the full heap. It has to be allocated here,
    # before `session_begin`, and referenced until `session_cancel` has run:
    # the allocator holds a raw pointer into it for the whole session.
    scratch = bytearray(SCRATCH_BYTES)

    try:
        return await _stream_and_sign(
            wallet_seed,
            network,
            account,
            host_reference_height,
            pczt_length,
            session,
            coin_name,
            network_label,
            account_label,
            path,
            handle_out,
            scratch,
        )
    except ValueError as exc:
        # Only the two curated native messages reach the host verbatim; any other
        # ValueError is collapsed to the generic malformed string so incidental
        # MicroPython/parse text never leaks.
        msg = str(exc)
        raise wire.DataError(
            msg
            if msg in (_MALFORMED, _TOO_MANY_ACTIONS, _AMOUNT_OUT_OF_RANGE)
            else _MALFORMED
        )
    except RuntimeError:
        raise wire.ProcessError("Zcash PCZT rejected")
    finally:
        # Runs on normal return, on host-cancel, and on autolock: when the idle
        # timer closes this workflow the GeneratorExit unwinds through here, so
        # the native session (and its secrets) is always torn down.
        # `session_cancel` wipes the scratch and hands it back before this
        # returns, so dropping the reference here cannot leave the allocator
        # pointing into collected memory.
        _cancel_native(handle_out[0] if handle_out else None)
        del scratch
        del wallet_seed


def _cancel_native(handle: int | None) -> None:
    from trezor import utils
    from trezorzcash import session_cancel

    try:
        session_cancel(handle)
    finally:
        # Clear completed native stack frames before any await.
        utils.zero_unused_stack()


async def _stream_and_sign(
    wallet_seed: bytes,
    network: int,
    account: int,
    host_reference_height: int,
    pczt_length: int,
    session: SessionIdentity,
    coin_name: str,
    network_label: str,
    account_label: str,
    path: str,
    handle_out: list[int],
    scratch: bytearray,
) -> ZcashSpendAuthSignatures:
    from trezor import TR, utils, wire
    from trezor.crypto import random
    from trezor.messages import ZcashPcztAck, ZcashPcztRequest, ZcashSpendAuthSignatures
    from trezor.ui.layouts.progress import progress
    from trezorzcash import (
        session_approve,
        session_begin,
        session_feed,
        session_sign,
    )

    from . import helpers

    # Up before `session_begin`, which is itself seconds of native work. The
    # bar tracks PCZT bytes verified: the action count is only known at the
    # final review step, but `pczt_length` is known up front.
    progress_layout = progress(TR.progress__loading_transaction)
    progress_layout.report(0)

    # `session_begin` derives the account keys from the seed, and `zip32`'s
    # `HardenedOnlyKey` is not `Zeroize`: wipe the native frames before the
    # first host round trip.
    try:
        handle = session_begin(
            wallet_seed,
            network,
            account,
            host_reference_height,
            MAXIMUM_FEE,
            EXPIRY_WINDOW,
            pczt_length,
            scratch,
        )
    finally:
        utils.zero_unused_stack()
    # Hand the handle to the caller's `finally` before anything can fail: from
    # here on a teardown cancels this session by name, not by "whatever is
    # live".
    handle_out.append(handle)
    transfer_id = random.bytes(TRANSFER_ID_BYTES)
    offset = 0
    # One running number across both halves of the payment: the transparent
    # outputs stream before the shielded actions, so they are outputs #1..#N
    # and the shielded payments continue from there.
    payments = 0
    warned_transparent = False
    totals = None
    while offset < pczt_length:
        length = min(CHUNK_BYTES, pczt_length - offset)
        reply = await _call(
            ZcashPcztRequest(transfer_id=transfer_id, offset=offset, length=length),
            ZcashPcztAck,
        )
        if (
            reply.transfer_id != transfer_id
            or reply.offset != offset
            or len(reply.data) != length
        ):
            raise wire.DataError("Invalid PCZT chunk")
        data = memoryview(reply.data)
        del reply
        fed = 0
        while fed < length:
            # Report before each blocking feed: after a confirmation it brings
            # the screen back (the finished layout faded the backlight), and it
            # touches the idle timer, so an actively progressing sign never
            # autolocks while one parked at a ButtonRequest still does.
            progress_layout.report(1000 * (offset + fed) // pczt_length)
            # One chunk may complete several outputs; each returns separately
            # and the remainder is fed again after its confirmation.
            consumed, kind, payload = session_feed(handle, data[fed:])
            utils.zero_unused_stack()
            fed += consumed
            if kind == _STEP_TRANSPARENT_OUTPUT:
                _index, t_kind, hash160, value = payload
                if not warned_transparent:
                    await _warn_transparent(session)
                    warned_transparent = True
                await _confirm_transparent_output(
                    t_kind, hash160, value, payments, coin_name, account_label, path
                )
                payments += 1
                helpers.require_session(session)
            elif kind == _STEP_OUTPUT:
                _action_index, receiver, value, memo_kind, memo, user_address = payload
                await _confirm_output(
                    receiver,
                    user_address,
                    value,
                    payments,
                    coin_name,
                    account_label,
                    path,
                )
                if memo_kind != _MEMO_NONE:
                    helpers.require_session(session)
                    await _confirm_memo(memo_kind, memo, payments)
                payments += 1
                helpers.require_session(session)
            elif kind == _STEP_REVIEW:
                totals = payload
            elif consumed == 0:
                raise wire.ProcessError("Zcash PCZT rejected")
        offset += length
        del data

    if totals is None:
        raise wire.ProcessError("Zcash PCZT rejected")
    await _confirm_totals(totals, coin_name, network_label, account_label, path)
    helpers.require_session(session)
    session_approve(handle)
    # One RedPallas signature per real spend, seconds of native work.
    progress_layout = progress(TR.progress__signing_transaction)
    progress_layout.report(0)
    try:
        records = session_sign(handle, wallet_seed)
    finally:
        utils.zero_unused_stack()
    progress_layout.report(1000)
    if type(records) is not bytes or len(records) == 0 or len(records) % RECORD_LEN:
        raise wire.ProcessError("Zcash signing failed")

    return ZcashSpendAuthSignatures(transfer_id=transfer_id, records=records)
