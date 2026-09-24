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
    from trezorzcash import derive_receiver

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

    from . import helpers, unified_addresses

    if not utils.USE_ZCASH_SHIELDED:
        raise wire.ProcessError("Zcash shielded support is not enabled")

    network = msg.network  # local_cache_attribute
    account = msg.account  # local_cache_attribute
    diversifier_index = msg.diversifier_index  # local_cache_attribute

    coin_name, network_label, coin_type = helpers.validate_network_account(
        network, account
    )
    helpers.validate_diversifier_index(diversifier_index)

    seed.raise_if_not_initialized()
    session = helpers.snapshot_session()

    if helpers.has_weak_backup():
        await show_warning(
            br_name="zcash_weak_backup",
            content=TR.zcash__weak_backup_warning,
            br_code=ButtonRequestType.Warning,
        )
        helpers.require_session(session)

    wallet_seed = await seed.get_seed()
    # Show progress during the blocking native derivation. It is a static
    # ring: nothing can move it while the native call holds the VM.
    progress_layout = progress(indeterminate=True)
    progress_layout.report(0)
    try:
        helpers.require_session(session)
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
        account=helpers.account_label(account),
        path=helpers.account_path(coin_type, account),
        case_sensitive=False,
        br_name="zcash_receive",
        br_code=ButtonRequestType.Address,
        chunkify=bool(msg.chunkify),
    )
    helpers.require_session(session)
    return ZcashAddress(address=address)
