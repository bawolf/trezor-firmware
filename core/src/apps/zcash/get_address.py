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
    # MEASUREMENT-ONLY per-operation latency bench (ironwood-measurement build).
    # Times the two candidate per-action verification costs in isolation, so a
    # hardware run can attribute the ~23 s per real spend to Sinsemilla hashing
    # versus Pallas scalar multiplication. Each op is one native call timed with
    # utime.ticks_ms here on the Python side; the native side returns a folded
    # accumulator so nothing is optimised away. Uses the same computed-generator
    # Sinsemilla and pinned Pasta the signing path links. Changes no signing
    # behavior.
    #
    # The `bench` / `session_region_high_water` native bindings exist ONLY when
    # the firmware was built with the `ironwood-measurement` Cargo feature. In a
    # PRODUCTION build (default, feature OFF) they are absent, the import below
    # raises ImportError, and this returns None so the reserved diversifier index
    # derives normally instead of running any bench (Fable review R1).
    import utime

    try:
        from trezorironwood import bench, session_region_high_water
    except ImportError:
        return None

    # `bench` ignores this argument (it allocates from the boot-lifetime `.buf`
    # region the signing path installs), so pass an EMPTY bytearray. A 96 KiB GC
    # bytearray here would be dead weight — a single contiguous allocation from
    # the now-138.7 KiB heap that can raise MemoryError under fragmentation for a
    # reason unrelated to the bench (Fable review SHOULD-FIX #4).
    region = bytearray()
    slow_iters = 6  # Sinsemilla ops cost seconds each under computed generators
    fast_iters = 200  # scalar mult is milliseconds

    # Warm one-time init (Pasta sqrt table, domain generator derivation) off the
    # clock so it is not billed to the first timed op.
    bench(0, region, 1)

    def _time(selector, iters):
        start = utime.ticks_ms()
        acc = bench(selector, region, iters)
        elapsed = utime.ticks_diff(utime.ticks_ms(), start)
        return elapsed / iters, acc

    nc_ms, nc_acc = _time(1, slow_iters)  # note commitment (hash + blinding)
    sh_ms, sh_acc = _time(2, slow_iters)  # Sinsemilla hash only
    sm_ms, sm_acc = _time(3, fast_iters)  # Pallas variable-base scalar mult
    iv_ms, iv_acc = _time(4, slow_iters)  # commit_ivk (FVK derivation)
    session_peak, _in_use_at_begin, boot_peak = session_region_high_water()

    result = (
        "ironwood op timing slow_iters=%d fast_iters=%d "
        "note_commitment_ms=%.2f sinsemilla_hash_ms=%.2f blinding_mult_ms=%.2f "
        "scalar_mul_ms=%.4f commit_ivk_ms=%.2f "
        "session_peak=%d boot_peak=%d acc=%d,%d,%d,%d"
        % (
            slow_iters,
            fast_iters,
            nc_ms,
            sh_ms,
            nc_ms - sh_ms,
            sm_ms,
            iv_ms,
            session_peak,
            boot_peak,
            nc_acc,
            sh_acc,
            sm_acc,
            iv_acc,
        )
    )
    if __debug__:
        from trezor import log

        log.debug(__name__, "%s", result)
    print("ironwood op timing:", result)
    return result


async def get_address(msg: ZcashGetAddress) -> ZcashAddress:
    from trezor import TR, utils, wire
    from trezor.enums import ButtonRequestType
    from trezor.messages import ZcashAddress
    from trezor.ui.layouts import show_address, show_warning

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
