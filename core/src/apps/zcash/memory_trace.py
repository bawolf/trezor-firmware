"""Read fixed GC counters from the completed native synthetic workflow."""
from micropython import const

import ironwood_test
import usb
from trezor import loop, utils, workflow
from trezor.messages import IronwoodMemoryTrace
from trezor.wire import ProcessError, context

TRACE_BYTES = const(80)


async def memory_trace(msg):
    if not (
        utils.IRONWOOD_NATIVE_CALLER
        and not utils.EMULATOR
        and utils.INTERNAL_MODEL == "T3T1"
        and utils.UI_LAYOUT == "DELIZIA"
        and not utils.USE_THP
    ):
        raise ProcessError("Unsupported synthetic test build")
    if context.get_context().iface is not usb.iface_wire:
        raise ProcessError("Primary wire interface required")
    if loop.this_task is None:
        raise ProcessError("Registered workflow required")
    # Do not cancel another workflow to obtain diagnostics.
    if any(not task.is_running() for task in workflow.tasks):
        raise ProcessError("Another workflow is active")
    data = ironwood_test.memory_trace()
    if len(data) != TRACE_BYTES:
        raise ProcessError("Invalid memory trace length")
    return IronwoodMemoryTrace(data=data)
