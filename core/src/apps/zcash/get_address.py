from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from buffer_types import AnyBytes

    from trezor.messages import ZcashAddress, ZcashGetAddress


def _call_native(
    seed: AnyBytes,
    network: int,
    account: int,
    diversifier_index: AnyBytes,
) -> bytes:
    from trezorironwood import derive_receiver

    return derive_receiver(seed, network, account, diversifier_index)


def _derive_receiver(
    seed: AnyBytes,
    network: int,
    account: int,
    diversifier_index: AnyBytes,
) -> bytes:
    from trezor import utils

    try:
        return _call_native(seed, network, account, diversifier_index)
    finally:
        # Clear completed native stack frames before any await.
        utils.zero_unused_stack()


def _op_timing_bench() -> str | None:
    # MEASUREMENT-ONLY per-operation latency bench. The `ironwood_measurement`
    # module (which carries the bench body and the native bench /
    # region-telemetry bindings) is frozen ONLY when the firmware is built with
    # the `ironwood-measurement` Cargo feature. In a PRODUCTION build (default,
    # feature OFF) there is no such module, this returns None, and the reserved
    # 0xff diversifier index derives an address normally instead of running any
    # bench (Fable review R1).
    try:
        from .ironwood_measurement import op_timing_bench
    except ImportError:
        return None
    return op_timing_bench()


async def get_address(msg: ZcashGetAddress) -> ZcashAddress:
    from trezor import TR, utils, wire
    from trezor.enums import ButtonRequestType
    from trezor.messages import ZcashAddress
    from trezor.ui.layouts import show_address, show_warning
    from trezor.ui.layouts.progress import progress

    from apps.common import coininfo, seed

    from . import ironwood_account, unified_addresses

    if not utils.USE_IRONWOOD:
        raise wire.ProcessError("Ironwood is not supported")

    network = msg.network  # local_cache_attribute
    account = msg.account  # local_cache_attribute
    diversifier_index = msg.diversifier_index  # local_cache_attribute

    coin_name, network_label, coin_type = ironwood_account.validate_network_account(
        network, account
    )
    ironwood_account.validate_diversifier_index(diversifier_index)

    # MEASUREMENT-ONLY (ironwood-measurement builds): the reserved all-0xff
    # diversifier index triggers the per-operation latency bench and returns the
    # timings in a ProcessError instead of deriving an address. In a PRODUCTION
    # build (default) the bench binding is absent, `_op_timing_bench()` returns
    # None, and this index derives normally like any other — so the valid
    # diversifier index 2^88-1 stays usable and no bench runs pre-consent (R1).
    if bytes(diversifier_index) == b"\xff" * 11:
        bench_timings = _op_timing_bench()
        if bench_timings is not None:
            raise wire.ProcessError(bench_timings)

    seed.raise_if_not_initialized()
    session = ironwood_account.snapshot_session()

    if ironwood_account.has_weak_backup():
        await show_warning(
            br_name="ironwood_weak_backup",
            content=TR.zcash__weak_backup_warning,
            br_code=ButtonRequestType.Warning,
        )
        ironwood_account.require_session(session)

    wallet_seed = await seed.get_seed()
    # The native Pallas derivation runs for seconds behind a single blocking
    # call; show the generic progress layout rather than a blank screen, the
    # way every other app does. The address screen below starts its own layout,
    # which stops this one, so the consent screen appears exactly as before.
    progress_layout = progress(indeterminate=True)
    progress_layout.report(0)
    try:
        ironwood_account.require_session(session)
        try:
            receiver = _derive_receiver(
                wallet_seed,
                network,
                account,
                diversifier_index,
            )
            if type(receiver) is not bytes or len(receiver) != 43:
                raise wire.ProcessError("Zcash receiver derivation failed")
        except (ValueError, RuntimeError):
            raise wire.ProcessError("Zcash receiver derivation failed")
    finally:
        del wallet_seed

    address = unified_addresses.encode(
        {unified_addresses.Typecode.ORCHARD: receiver}, coininfo.by_name(coin_name)
    )
    await show_address(
        address=address,
        address_qr=address,
        network=network_label,
        account=ironwood_account.account_label(account),
        path=ironwood_account.account_path(coin_type, account),
        case_sensitive=False,
        br_name="ironwood_receive",
        br_code=ButtonRequestType.Address,
    )
    ironwood_account.require_session(session)
    return ZcashAddress(address=address)
