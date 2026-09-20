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

# The handler's timeouts. CHUNK_TIMEOUT_MS drops a host that stalls a single
# chunk. ATTENDED_DEADLINE_MS is an IDLE timeout, not a whole-session cap: it is
# pushed forward on every activity (each chunk received, each ButtonRequest
# shown; see `_rearm_deadline`), so it only fires after this much genuine
# INACTIVITY — a walk-away / unattended device — and never cancels a legitimate
# long 32-action sign that is actively streaming or being confirmed (Fable
# review #M2).
CHUNK_TIMEOUT_MS = const(5_000)
ATTENDED_DEADLINE_MS = const(180_000)

# Curated, non-secret native ValueError messages allowed to reach the host
# verbatim. Any other ValueError (MicroPython unpack, parse_u32, format_amount,
# address encode, ...) is collapsed to MALFORMED so incidental text never leaks
# to the host (Fable review #S3). Must match `micropython/ironwood.rs::failure`.
_MALFORMED = "Malformed PCZT"
_TOO_MANY_ACTIONS = "Too many transaction actions (max 32)"

# session_feed step kinds.
_STEP_OUTPUT = const(1)
_STEP_REVIEW = const(2)

RECORD_LEN = const(66)
ZEC_DECIMALS = const(8)


async def _cancel_after_deadline(owner) -> None:
    # Idle watchdog: sleeps for ATTENDED_DEADLINE_MS, but every activity re-arms
    # it (`_rearm_deadline` reschedules this task further into the future), so it
    # only reaches `loop.close(owner)` after that much INACTIVITY. It is not a
    # whole-session cap (Fable review #M2).
    from trezor import loop

    await loop.sleep(ATTENDED_DEADLINE_MS)
    loop.close(owner)


def _rearm_deadline(deadline) -> None:
    import utime

    from trezor import loop

    # Push the idle deadline forward from now. Called on each chunk received and
    # before each ButtonRequest, so the deadline bounds inactivity at any single
    # await rather than the whole signing session (Fable review #M2).
    loop.schedule(
        deadline,
        None,
        utime.ticks_add(utime.ticks_ms(), ATTENDED_DEADLINE_MS),
        reschedule=True,
    )


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
            deadline,
        )
    except ValueError as exc:
        # Only the two curated native messages reach the host verbatim; any other
        # ValueError is collapsed to the generic malformed string so incidental
        # MicroPython/parse text never leaks (Fable review #S3).
        msg = str(exc)
        raise wire.DataError(msg if msg in (_MALFORMED, _TOO_MANY_ACTIONS) else _MALFORMED)
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
    deadline,
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
        # Host activity: re-arm the idle deadline (Fable review #M2).
        _rearm_deadline(deadline)
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
                # New ButtonRequest: re-arm the idle deadline (Fable review #M2).
                _rearm_deadline(deadline)
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
    # Consent ButtonRequest: re-arm the idle deadline (Fable review #M2).
    _rearm_deadline(deadline)
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

    # MEASUREMENT-ONLY latency trailer, gated on the ironwood-measurement build.
    # `session_region_high_water` is bound ONLY in that build (it exposes the
    # internal region layout to the host), so a PRODUCTION build (default) takes
    # the ImportError path and returns the signature records with NO timing
    # trailer and NO console print. The proto `debug_timings` field stays unset
    # in production, matching its documentation (Fable review #M1).
    try:
        from trezorironwood import session_region_high_water
    except ImportError:
        # PRODUCTION: no post-consent telemetry.
        return ZcashSpendAuthSignatures(transfer_id=transfer_id, records=records)

    # MEASUREMENT build only from here. An ASCII key=value breakdown of the
    # per-phase on-device wall-clock plus the region counters, so a plain host
    # (no DebugLink) reads where the ~35 s first-review and ~103 s sign time go.
    #
    # SWEEP PROTOCOL: run ascending N (2, 4, 8, 16, 32) on ONE boot. session_peak
    # is per-session (resets at session_begin), so it should stay ~flat (~45,760 B)
    # across N — that is the O(1)-RAM claim. in_use_at_begin is the persistent
    # set (Pasta table + orchard OnceBox caches) already allocated when each
    # session began; after session 1 it must stay CONSTANT — any upward drift is
    # a cross-session leak. boot_peak is the boot-monotone max (only grows).
    session_peak, in_use_at_begin, boot_peak = session_region_high_water()
    # The full boot-lifetime signing region size: the denominator for the
    # high-water ratios (mirrors ironwood_allocator.rs; measurement-only).
    region_bytes = 96 * 1024
    # The full bundle action count (payments + change + padding) from the review
    # totals, NOT the payment count, so the trailer figure matches the 32-action
    # guardrail cap (Fable review #S5).
    action_count = totals[8]
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
            region_bytes,
        )
    ).encode()
    if __debug__:
        from trezor import log

        log.debug(__name__, "ironwood latency: %s", debug_timings)
    print("ironwood latency:", debug_timings)  # JTAG/emulator log readout
    return ZcashSpendAuthSignatures(
        transfer_id=transfer_id, records=records, debug_timings=debug_timings
    )
