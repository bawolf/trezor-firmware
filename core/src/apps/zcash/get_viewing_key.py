from typing import TYPE_CHECKING

if TYPE_CHECKING:
    from buffer_types import AnyBuffer

    from trezor.messages import ZcashGetViewingKey, ZcashViewingKey


def _call_native(
    seed: bytes,
    network: int,
    account: int,
    output: AnyBuffer,
) -> None:
    from trezorironwood import derive_viewing_key

    derive_viewing_key(seed, network, account, output)


def _derive_viewing_key(
    seed: bytes,
    network: int,
    account: int,
    output: AnyBuffer,
) -> None:
    from trezor import utils

    try:
        _call_native(seed, network, account, output)
    finally:
        # Clear completed native stack frames before any subsequent await.
        utils.zero_unused_stack()


async def get_viewing_key(msg: ZcashGetViewingKey) -> ZcashViewingKey:
    from trezor import TR, utils, wire
    from trezor.enums import ButtonRequestType
    from trezor.messages import ZcashViewingKey
    from trezor.ui.layouts import confirm_action, show_warning

    from apps.common import coininfo, seed

    from . import ironwood_account, unified_addresses

    if not utils.USE_IRONWOOD:
        raise wire.ProcessError("Ironwood is not supported")

    network = msg.network  # local_cache_attribute
    account = msg.account  # local_cache_attribute
    coin_name, network_label, coin_type = ironwood_account.validate_network_account(
        network, account
    )

    seed.raise_if_not_initialized()
    session = ironwood_account.snapshot_session()
    account_label = ironwood_account.account_label(account)
    path = ironwood_account.account_path(coin_type, account)

    await confirm_action(
        "ironwood_export_viewing_key",
        TR.zcash__export_viewing_key,
        action=f"Zcash {network_label}\n{account_label}\n{path}",
        description=TR.zcash__viewing_key_warning,
        br_code=ButtonRequestType.SignTx,
        prompt_screen=True,
    )
    ironwood_account.require_session(session)

    if ironwood_account.has_weak_backup():
        await show_warning(
            br_name="ironwood_weak_backup",
            content=TR.zcash__weak_backup_warning,
            br_code=ButtonRequestType.Warning,
        )
        ironwood_account.require_session(session)

    wallet_seed = await seed.get_seed()
    raw_fvk = bytearray(96)
    try:
        ironwood_account.require_session(session)
        try:
            _derive_viewing_key(wallet_seed, network, account, raw_fvk)
            key = unified_addresses.encode_fvk(raw_fvk, coininfo.by_name(coin_name))
        except (KeyError, ValueError, RuntimeError):
            raise wire.ProcessError("Zcash viewing key derivation failed")
    finally:
        utils.memzero(raw_fvk)
        del wallet_seed

    ironwood_account.require_session(session)
    return ZcashViewingKey(key=key)
