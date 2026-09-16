"""Bounded synthetic PCZT workflow for supported test builds."""
from micropython import const

import ironwood_test
import usb
from trezor import loop, utils, workflow
from trezor.crypto import random
from trezor.messages import (
    IronwoodPcztAck,
    IronwoodPcztRequest,
    IronwoodSignedPczt,
    IronwoodSignedPcztAck,
    Success,
)
from trezor.wire import ActionCancelled, DataError, ProcessError, context

from apps.zcash.ironwood_review import review_and_sign
from apps.zcash.ironwood_account import get_seed

MAX_PCZT_BYTES = const(65536)
CHUNK_BYTES = const(1024)


async def _cancel_after_deadline(owner):
    await loop.sleep(180000)
    loop.close(owner)


async def _call(message, expected_type):
    reply = await loop.race(
        context.with_context(context.get_context(), context.call(message, expected_type)),
        loop.sleep(5000),
    )
    if not expected_type.is_type_of(reply):
        raise ActionCancelled("PCZT transfer timed out")
    return reply


def _supported_runtime():
    return (
        utils.IRONWOOD_NATIVE_CALLER
        and not utils.EMULATOR
        and utils.INTERNAL_MODEL == "T3T1"
        and utils.UI_LAYOUT == "DELIZIA"
        and not utils.USE_THP
    )


async def sign_pczt(msg):
    if not _supported_runtime():
        raise ProcessError("Unsupported synthetic test build")
    ctx = context.get_context()
    if ctx.iface is not usb.iface_wire:
        raise ProcessError("Primary wire interface required")

    if loop.this_task is None or not any(task.is_running() for task in workflow.tasks):
        raise ProcessError("Registered workflow required")

    # Do this while executing inside the registered workflow, before any child task.
    workflow.close_others()
    # Closing another session unwinds its context wrapper. Restore this session
    # with the existing helper, in the same registered task, before any I/O.
    return await context.with_context(ctx, _transfer(msg))


async def _transfer(msg):
    owner = loop.this_task
    if owner is None:
        raise ProcessError("Registered workflow required")
    memory_started = False
    deadline = _cancel_after_deadline(owner)
    try:
        # Keep the UI inside this registered task. Racing the whole workflow would
        # make its layouts' close_others() close their own waiting parent.
        loop.schedule(deadline)
        total = msg.total_length
        if not 1 <= total <= MAX_PCZT_BYTES:
            raise DataError("Invalid PCZT length")
        if utils.IRONWOOD_NATIVE_CALLER:
            ironwood_test.memory_start()
            memory_started = True
        transfer_id = random.bytes(16)
        pczt = bytearray(total)
        offset = 0
        while offset < total:
            length = min(CHUNK_BYTES, total - offset)
            reply = await _call(
                IronwoodPcztRequest(transfer_id=transfer_id, offset=offset, length=length),
                IronwoodPcztAck,
            )
            if (
                reply.transfer_id != transfer_id
                or reply.offset != offset
                or len(reply.data) != length
            ):
                raise DataError("Invalid PCZT chunk")
            pczt[offset : offset + length] = reply.data
            offset += length
            del reply

        session_id = context.get_context().cache.export_session_id()
        seed = await get_seed()
        review = ironwood_test.begin(seed, pczt)
        del seed, pczt
        signed = await review_and_sign(review, session_id)
        del review
        total = len(signed)
        if not 1 <= total <= MAX_PCZT_BYTES:
            raise DataError("Invalid signed PCZT length")
        offset = 0
        while offset < total:
            end = min(offset + CHUNK_BYTES, total)
            reply = await _call(
                IronwoodSignedPczt(
                    transfer_id=transfer_id, total_length=total, offset=offset,
                    data=signed[offset:end],
                ),
                IronwoodSignedPcztAck,
            )
            if reply.transfer_id != transfer_id or reply.next_offset != end:
                raise DataError("Invalid signed PCZT acknowledgement")
            offset = end
            del reply
        return Success(message="Synthetic PCZT signed")
    finally:
        try:
            # The deadline task may be closing us right now; it then exits itself.
            if deadline is not loop.this_task:
                loop.close(deadline)
        finally:
            ironwood_test.cancel()
            if memory_started:
                ironwood_test.memory_finish()
