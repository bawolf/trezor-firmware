"""Streamed Ironwood PCZT signing.

The device pulls the PCZT in 1,024-byte chunks and feeds them to the native
streaming session (`trezorironwood.session_*`), which never retains more than
one action. Payment outputs are confirmed as they arrive, like Bitcoin's
signer; those confirmations are not consent. Consent is the totals screen
after the whole PCZT verified, after which the device returns one signature
record per real spend (docs/common/zcash-ironwood-signing.md §3-§4).
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

# Device-owned policy limits (ironwood::Limits). The fee cap is 0.01 ZEC;
# the expiry window bounds how far past the host's reference height a
# transaction may stay valid.
MAXIMUM_FEE = const(1_000_000)
EXPIRY_WINDOW = const(100)

# CHUNK_TIMEOUT_MS drops a host that stalls a single chunk read. There is no
# bespoke walk-away/idle timeout: an unattended or abandoned sign is torn down by
# Trezor's standard autolock, the same mechanism Bitcoin sign_tx relies on. When
# the idle timer fires, `lock_manager.lock_device` calls `workflow.close_others()`,
# which closes this paused workflow; the resulting GeneratorExit unwinds through
# `sign_pczt`'s `finally`, and `_cancel_native()` (`session_cancel`) drops the
# native request, wiping its session secrets. ZcashSignPczt is not in
# `workflow.ALLOW_WHILE_LOCKED`, so `autolock_interrupts_workflow` stays True and
# the interrupt applies.
#
# Parity with Bitcoin sign_tx is two-sided. (1) Teardown: an abandoned sign is
# closed by autolock and its secrets are wiped in the `finally` above. (2)
# Survival of a legitimate long sign: Bitcoin keeps the idle timer alive across
# its silent verification phase via `progress.report()` (which reaches
# `workflow.idle_timer.touch()`); we do exactly the same thing, reporting the
# streaming progress layout after each verified step in `_stream_and_sign` (see
# there), so autolock does not kill an actively-progressing sign. Unlike Bitcoin
# we deliberately do NOT set `autolock_interrupts_workflow = False`, because a
# truly abandoned pre-consent sign must still be torn down.
CHUNK_TIMEOUT_MS = const(5_000)

# Curated, non-secret native ValueError messages allowed to reach the host
# verbatim. Any other ValueError (MicroPython unpack, parse_u32, format_amount,
# address encode, ...) is collapsed to MALFORMED so incidental text never leaks
# to the host. Must match `micropython/ironwood.rs::failure`.
_MALFORMED = "Malformed PCZT"
_TOO_MANY_ACTIONS = "Too many transaction actions (max 32)"
_AMOUNT_OUT_OF_RANGE = "Zcash amount out of range"

# session_feed step kinds.
_STEP_OUTPUT = const(1)
_STEP_REVIEW = const(2)

# Memo kinds of an output step. Emitted by `session_feed` in
# core/embed/rust/src/micropython/ironwood.rs, which picks the same 0/1/2;
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
        # Deliberately unconditional, where Bitcoin makes it host-selectable
        # (`bool(tx.chunkify)` from `SignTx.chunkify`, apps/bitcoin/sign_tx/
        # approvers.py). A unified address is 106 characters: unbroken on a
        # consent screen it is not realistically comparable against the one in
        # the wallet, so there is no version of this screen worth offering
        # without grouping. `ZcashSignPczt` therefore carries no `chunkify`
        # field, and the asymmetry with `ZcashGetAddress` — where the flag is
        # opt-in, because that screen shipped unchunked and must not change
        # under an existing host — is on purpose.
        chunkify=True,
        source_account=account_label,
        source_account_path=path,
    )


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
    from trezor import TR, utils, wire
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
    from trezor import TR, utils, wire
    from trezor.crypto import random
    from trezor.messages import ZcashPcztAck, ZcashPcztRequest, ZcashSpendAuthSignatures
    from trezor.ui.layouts.progress import progress
    from trezorironwood import (
        session_approve,
        session_begin,
        session_feed,
        session_sign,
    )

    from . import ironwood_account

    # Bitcoin shows its progress screen for the whole of sign_tx — while it
    # waits for host data and while it verifies silently — and never a blank
    # one. Same here, and it has to be up *before* `session_begin`, because
    # that call prewarms the Pasta/Sinsemilla generators and is itself seconds
    # of blocking native work. The bar is driven by PCZT bytes verified: the
    # action count is not known until the final review step arrives, but
    # `pczt_length` is known up front, is monotonic, and the per-action
    # verification cost is what consumes those bytes.
    progress_layout = progress(TR.progress__loading_transaction)
    progress_layout.report(0)

    # `session_begin` is a seed-touching native call: it runs the whole ZIP-32
    # path and derives the account FVK, which computes the spend-authorizing
    # scalar as a temporary, and `zip32`'s `HardenedOnlyKey` is not `Zeroize`.
    # Wipe the completed native frames before the first host round trip, the
    # way `session_sign`, `get_address` and `get_viewing_key` do. Without this
    # the first wipe is after the first `session_feed` -- a `random.bytes`, a
    # host call of up to CHUNK_TIMEOUT_MS, a protobuf decode and one action's
    # verification later.
    try:
        handle = session_begin(
            wallet_seed,
            network,
            account,
            host_reference_height,
            MAXIMUM_FEE,
            EXPIRY_WINDOW,
            pczt_length,
        )
    finally:
        utils.zero_unused_stack()
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
            # Report BEFORE the blocking call, never after, and that ordering is
            # the whole point rather than a detail. Finishing a confirmation runs
            # `Layout.stop()`, which fades the backlight out because no layout is
            # running; the very next thing this loop does is verify the next
            # action, ~3 s of blocking native work. Reporting first is what
            # brings the screen back (`report` -> `start` -> `repaint` ->
            # `backlight_fade(NORMAL)`), so the action after every payment and
            # every memo is covered too — that gap was the live-session symptom.
            # It also removes a bar that used to animate for a few frames and be
            # replaced by the consent screen immediately.
            #
            # `offset + fed` is the bytes actually verified so far, so the bar
            # reads "this much is done, now working on the next action".
            #
            # This is also what keeps the idle timer alive: it is
            # `ProgressLayout.report` (trezor/ui/__init__.py) that calls
            # `workflow.idle_timer.touch()`, which is why `_stream_and_sign` no
            # longer touches the timer itself. A report still precedes every
            # feed and `length >= 1`, so this runs at least once per chunk —
            # strictly more often than the per-chunk touch it replaces. The
            # guarantee in the CHUNK_TIMEOUT_MS note above is unchanged: an
            # actively progressing sign never autolocks, and a sign parked at a
            # ButtonRequest — where no report fires — still does.
            progress_layout.report(1000 * (offset + fed) // pczt_length)
            # One chunk may complete several outputs; each returns separately
            # and the remainder is fed again after its confirmation.
            consumed, kind, payload = session_feed(handle, data[fed:])
            utils.zero_unused_stack()
            fed += consumed
            if kind == _STEP_OUTPUT:
                _action_index, receiver, value, memo_kind, memo = payload
                await _confirm_output(
                    receiver, value, payments, coin_name, account_label, path
                )
                if memo_kind != _MEMO_NONE:
                    ironwood_account.require_session(session)
                    await _confirm_memo(memo_kind, memo, payments)
                payments += 1
                ironwood_account.require_session(session)
            elif kind == _STEP_REVIEW:
                totals = payload
            elif consumed == 0:
                raise wire.ProcessError("Zcash PCZT rejected")
        offset += length
        del data

    if totals is None:
        raise wire.ProcessError("Zcash PCZT rejected")
    await _confirm_totals(totals, coin_name, network_label, account_label, path)
    ironwood_account.require_session(session)
    session_approve(handle)
    # Post-consent signing is another multi-second blocking native call, one
    # RedPallas signature per real spend. Bitcoin switches its progress screen
    # from "Loading transaction..." to "Signing transaction..." at exactly this
    # point (progress.init_signing / report_init); so do we.
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
