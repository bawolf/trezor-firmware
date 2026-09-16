"""Display a wallet-derived regtest address; isolated test build, never for funds."""

import ironwood_test
import usb
from trezor import loop, utils, workflow
from trezor.messages import IronwoodAddress
from trezor.ui.layouts import show_address
from trezor.wire import DataError, ProcessError, context

from .unified_addresses import Typecode, _encode
from .ironwood_account import get_seed, require_session


async def receive_test(msg):
    """Registered legacy-wire entry; never accept a seed or display-skip option."""
    if not (
        utils.IRONWOOD_NATIVE_CALLER
        and not utils.EMULATOR
        and utils.INTERNAL_MODEL == "T3T1"
        and utils.UI_LAYOUT == "DELIZIA"
        and not utils.USE_THP
    ):
        raise ProcessError("Unsupported synthetic test build")
    ctx = context.get_context()
    if ctx.iface is not usb.iface_wire:
        raise ProcessError("Primary wire interface required")
    if loop.this_task is None or not any(task.is_running() for task in workflow.tasks):
        raise ProcessError("Registered workflow required")
    workflow.close_others()
    # Closing another workflow may unwind its context. Restore this one using
    # the existing wrapper, not a separate task or session manager.
    try:
        ironwood_test.cancel()
        address = await context.with_context(ctx, receive_address(msg.diversifier_index))
        return IronwoodAddress(address=address)
    finally:
        ironwood_test.cancel()


async def receive_address(index: bytes) -> str:
    """Caller must own the registered, exclusive wire/session workflow."""
    if not (
        utils.IRONWOOD_NATIVE_CALLER
        and not utils.EMULATOR
        and utils.INTERNAL_MODEL == "T3T1"
        and utils.UI_LAYOUT == "DELIZIA"
        and not utils.USE_THP
    ):
        raise ProcessError("Unsupported synthetic test build")

    try:
        ironwood_test.cancel()
        if type(index) is not bytes or len(index) != 11:
            raise DataError("Expected an 11-byte receiver index")
        session_id = context.get_context().cache.export_session_id()
        seed = await get_seed()
        receiver = ironwood_test.receive(seed, index)
        del seed
        address = _encode({Typecode.ORCHARD: receiver}, "uregtest")
        await show_address(
            address,
            address_qr=address,
            title="Synthetic receive",
            network="REGTEST - test wallet - no funds",
            account="Test account 9",
            path="m_Orchard/32'/1'/9'",
            case_sensitive=False,
            br_name="ironwood_test_receive",
        )
        require_session(session_id)
        return address
    finally:
        ironwood_test.cancel()
