"""Streamed Ironwood PCZT signing.

The device pulls the PCZT in 1,024-byte chunks and feeds them to the native
streaming session (`trezorironwood.session_*`), which never retains more than
one action. Payment outputs are confirmed as they arrive, like Bitcoin's
signer; those confirmations are not consent. Consent is the totals screen
after the whole PCZT verified, after which the device returns one signature
record per real spend (docs/proposals/STREAMING_SIGNING_DESIGN.md §3-§4).
"""

from micropython import const
from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from typing import TypeVar

    from trezor.messages import ZcashSignPczt, ZcashSpendAuthSignatures
    from trezor.protobuf import MessageType

    from .ironwood_account import SessionIdentity

    LoadedMessageType = TypeVar("LoadedMessageType", bound=MessageType)

# Transfer constants shared with the host client (messages-zcash.proto).
MAX_PCZT_BYTES = const(65536)
CHUNK_BYTES = const(1024)
TRANSFER_ID_BYTES = const(16)

# Device-owned policy limits (trezor_ironwood::Limits). The fee cap is 0.01 ZEC;
# the expiry window bounds how far past the host's reference height a
# transaction may stay valid.
MAXIMUM_FEE = const(1_000_000)
EXPIRY_WINDOW = const(100)

# The signing core allocates on the device (Pasta's square-root table plus
# per-hash temporaries, measured peak 41 KB). The region backing it is a native
# boot-lifetime `.buf` static owned by the Rust allocator (NOT a Python object),
# rooted once and reused by every session so Pasta's sqrt table and orchard's
# OnceBox caches — built once per boot behind Rust statics that point into it —
# stay valid across sessions (docs/decisions/2026-09-18-cross-session-region-
# lifetime.md). This value is kept only for the measurement trailer's
# `region_bytes`; the real size lives in `ironwood_allocator.rs`. Not final.
REGION_BYTES = const(96 * 1024)

# The historical handler's limits: a host that stalls a chunk is dropped, and
# an unattended review is abandoned.
CHUNK_TIMEOUT_MS = const(5_000)
ATTENDED_DEADLINE_MS = const(180_000)

# session_feed step kinds.
_STEP_OUTPUT = const(1)
_STEP_REVIEW = const(2)

RECORD_LEN = const(66)
ZEC_DECIMALS = const(8)


async def _cancel_after_deadline(owner) -> None:
    from trezor import loop

    await loop.sleep(ATTENDED_DEADLINE_MS)
    loop.close(owner)


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


async def _confirm_output(
    receiver: bytes,
    value: int,
    number: int,
    coin_name: str,
    account_label: str,
    path: str,
) -> None:
    from trezor.ui import layouts

    from apps.common import coininfo

    from . import unified_addresses

    coin = coininfo.by_name(coin_name)
    address = unified_addresses.encode(
        {unified_addresses.Typecode.ORCHARD: receiver}, coin
    )
    await layouts.confirm_output(
        address,
        _amount(value, coin.coin_shortcut),
        output_index=number,
        chunkify=True,
        source_account=account_label,
        source_account_path=path,
    )


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
        fee,
        _padding_outputs,
        payment_outputs,
        action_count,
    ) = totals
    # Bitcoin's totals screen: what leaves the wallet (payments plus fee), the
    # fee, and the source account. The host reference height is a policy input
    # only and is deliberately not shown; the digest-bound expiry height is.
    await layouts.confirm_total(
        _amount(payment_total + fee, coin.coin_shortcut),
        _amount(fee, coin.coin_shortcut),
        account_items=[
            (TR.words__account, f"Zcash {network_label} {account_label}", None),
            (TR.address_details__derivation_path, path, None),
        ],
        fee_items=[
            ("Expires at block", str(expiry_height), None),
            (TR.words__outputs, str(payment_outputs), None),
            # The full bundle size the device signs (payments + change +
            # padding), i.e. the count bounded by the 32-action hard cap. Shown
            # so the user sees the true size, not just the visible payments.
            ("Total actions", str(action_count), None),
        ],
    )


async def sign_pczt(msg: ZcashSignPczt) -> ZcashSpendAuthSignatures:
    from trezor import TR, loop, utils, wire
    from trezor.enums import ButtonRequestType
    from trezor.ui.layouts import show_warning

    from apps.common import seed

    from . import ironwood_account

    if not utils.USE_IRONWOOD:
        raise wire.ProcessError("Ironwood is not supported")

    network = msg.network  # local_cache_attribute
    account = msg.account  # local_cache_attribute
    pczt_length = msg.pczt_length  # local_cache_attribute
    host_reference_height = msg.host_reference_height  # local_cache_attribute

    coin_name, network_label, coin_type = ironwood_account.validate_network_account(
        network, account
    )
    if type(pczt_length) is not int or type(host_reference_height) is not int:
        raise wire.DataError(ironwood_account.MALFORMED_REQUEST)
    if not 1 <= pczt_length <= MAX_PCZT_BYTES:
        raise wire.DataError("Invalid PCZT length")
    if not 0 <= host_reference_height <= 0xFFFF_FFFF:
        raise wire.DataError(ironwood_account.MALFORMED_REQUEST)

    seed.raise_if_not_initialized()
    session = ironwood_account.snapshot_session()
    account_label = ironwood_account.account_label(account)
    path = ironwood_account.account_path(coin_type, account)

    if ironwood_account.has_weak_backup():
        await show_warning(
            br_name="ironwood_weak_backup",
            content=TR.zcash__weak_backup_warning,
            br_code=ButtonRequestType.Warning,
        )
        ironwood_account.require_session(session)

    wallet_seed = await seed.get_seed()
    ironwood_account.require_session(session)

    owner = loop.this_task
    if owner is None:
        raise wire.ProcessError("Registered workflow required")
    deadline = _cancel_after_deadline(owner)
    loop.schedule(deadline)
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
        )
    except ValueError as exc:
        # Native ValueError messages are curated and non-secret ("Malformed
        # PCZT", "Too many transaction actions (max 32)", ...). Propagate the
        # specific one so an oversized-but-well-formed transaction reports a
        # comprehensible reason instead of a raw "malformed".
        raise wire.DataError(str(exc) or "Malformed PCZT")
    except RuntimeError:
        raise wire.ProcessError("Zcash PCZT rejected")
    finally:
        try:
            # The deadline task may be closing us right now; it then exits itself.
            if deadline is not loop.this_task:
                loop.close(deadline)
        finally:
            _cancel_native()
            del wallet_seed


def _cancel_native() -> None:
    from trezor import utils
    from trezorironwood import session_cancel

    try:
        session_cancel()
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
) -> ZcashSpendAuthSignatures:
    import utime

    from trezor import utils, wire
    from trezor.crypto import random
    from trezor.messages import ZcashPcztAck, ZcashPcztRequest, ZcashSpendAuthSignatures
    from trezorironwood import (
        session_approve,
        session_begin,
        session_feed,
        session_sign,
    )

    from . import ironwood_account

    # MEASUREMENT-ONLY latency instrumentation (signing-latency-instrumentation
    # build). All phases are Python-driven, so utime.ticks_ms around the native
    # calls captures the on-device wall-clock spent in each. This times compute
    # only (the native calls), never the UI/button waits, and does not change
    # any signing behavior. See docs/decisions/2026-09-18-signing-latency-*.
    _t0 = utime.ticks_ms()
    session_begin(
        wallet_seed,
        network,
        account,
        host_reference_height,
        MAXIMUM_FEE,
        EXPIRY_WINDOW,
        pczt_length,
    )
    derive_ms = utime.ticks_diff(utime.ticks_ms(), _t0)
    feed_ms = 0  # cumulative wall-clock inside session_feed (compute only)
    feed_seg_ms = []  # per-step feed compute, split at each output / the review
    transfer_id = random.bytes(TRANSFER_ID_BYTES)
    offset = 0
    payments = 0
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
            # One chunk may complete several outputs; each returns separately
            # and the remainder is fed again after its confirmation.
            _tf = utime.ticks_ms()  # MEASUREMENT-ONLY
            consumed, kind, payload = session_feed(data[fed:])
            feed_ms += utime.ticks_diff(utime.ticks_ms(), _tf)  # MEASUREMENT-ONLY
            utils.zero_unused_stack()
            fed += consumed
            if kind == _STEP_OUTPUT:
                _action_index, receiver, value, is_change = payload
                if is_change:
                    raise wire.ProcessError("Zcash PCZT rejected")
                feed_seg_ms.append(feed_ms - sum(feed_seg_ms))  # MEASUREMENT-ONLY
                await _confirm_output(
                    receiver, value, payments, coin_name, account_label, path
                )
                payments += 1
                ironwood_account.require_session(session)
            elif kind == _STEP_REVIEW:
                feed_seg_ms.append(feed_ms - sum(feed_seg_ms))  # MEASUREMENT-ONLY
                totals = payload
            elif consumed == 0:
                raise wire.ProcessError("Zcash PCZT rejected")
        offset += length
        del data

    if totals is None:
        raise wire.ProcessError("Zcash PCZT rejected")
    await _confirm_totals(totals, coin_name, network_label, account_label, path)
    ironwood_account.require_session(session)
    session_approve()
    _ts = utime.ticks_ms()  # MEASUREMENT-ONLY
    try:
        records = session_sign(wallet_seed)
    finally:
        utils.zero_unused_stack()
    sign_ms = utime.ticks_diff(utime.ticks_ms(), _ts)  # MEASUREMENT-ONLY
    if type(records) is not bytes or len(records) == 0 or len(records) % RECORD_LEN:
        raise wire.ProcessError("Zcash signing failed")

    # MEASUREMENT-ONLY latency trailer. An ASCII key=value breakdown of the
    # per-phase on-device wall-clock plus the region counters, so a plain host
    # (no DebugLink) reads where the ~35 s first-review and ~103 s sign time go.
    # Remove with the proto field before any release. All region counters are 0
    # on the emulator; region_bytes is the full 96 KiB region for the ratio.
    #
    # SWEEP PROTOCOL: run ascending N (2, 4, 8, 16, 32) on ONE boot. session_peak
    # is per-session (resets at session_begin), so it should stay ~flat (~45,760 B)
    # across N — that is the O(1)-RAM claim. in_use_at_begin is the persistent
    # set (Pasta table + orchard OnceBox caches) already allocated when each
    # session began; after session 1 it must stay CONSTANT — any upward drift is
    # a cross-session leak. boot_peak is the boot-monotone max (only grows).
    # MEASUREMENT-ONLY region counters. `session_region_high_water` exists only
    # in ironwood-measurement builds; in a PRODUCTION build (default) the binding
    # is absent (it exposes internal region layout to the host) so the counters
    # report 0 and the signing path is unaffected (Fable review R1/#2).
    try:
        from trezorironwood import session_region_high_water

        session_peak, in_use_at_begin, boot_peak = session_region_high_water()
    except ImportError:
        session_peak = in_use_at_begin = boot_peak = 0
    action_count = payments
    debug_timings = (
        "derive_ms=%d feed_ms=%d sign_ms=%d action_count=%d "
        "feed_seg_ms=%s session_peak_bytes=%d in_use_at_begin_bytes=%d "
        "boot_peak_bytes=%d region_bytes=%d"
        % (
            derive_ms,
            feed_ms,
            sign_ms,
            action_count,
            ",".join(str(x) for x in feed_seg_ms),
            session_peak,
            in_use_at_begin,
            boot_peak,
            REGION_BYTES,
        )
    ).encode()
    if __debug__:
        from trezor import log

        log.debug(__name__, "ironwood latency: %s", debug_timings)
    print("ironwood latency:", debug_timings)  # JTAG/emulator log readout
    return ZcashSpendAuthSignatures(
        transfer_id=transfer_id, records=records, debug_timings=debug_timings
    )
