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
    # way every other app does. A *static* "Please wait" ring, not an animated
    # spinner -- see the note in get_viewing_key.py for why nothing can move it
    # while the native code holds the VM. The address screen below starts its
    # own layout, which stops this one, so the consent screen appears exactly
    # as before.
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
        # A unified address is 106 characters; unbroken it is hard to compare
        # against the one in the wallet. Same opt-in as Bitcoin's GetAddress
        # (`chunkify=bool(msg.chunkify)`), and off by default for the same
        # reason: the host decides, and an old host that does not know the
        # field gets exactly the screen it got before.
        chunkify=bool(msg.chunkify),
    )
    ironwood_account.require_session(session)
    return ZcashAddress(address=address)
