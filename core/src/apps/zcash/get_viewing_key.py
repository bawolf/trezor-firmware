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
    from trezorzcash import derive_viewing_key

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


def _derive_seed_fingerprint(seed: bytes, output: AnyBuffer) -> None:
    from trezor import utils
    from trezorzcash import seed_fingerprint

    try:
        seed_fingerprint(seed, output)
    finally:
        utils.zero_unused_stack()


async def get_viewing_key(msg: ZcashGetViewingKey) -> ZcashViewingKey:
    from trezor import TR, utils, wire
    from trezor.enums import ButtonRequestType
    from trezor.messages import ZcashViewingKey
    from trezor.ui.layouts import confirm_action, show_warning
    from trezor.ui.layouts.progress import progress

    from apps.common import coininfo, seed

    from . import helpers, unified_addresses

    if not utils.USE_ZCASH_SHIELDED:
        raise wire.ProcessError("Zcash shielded support is not enabled")

    network = msg.network  # local_cache_attribute
    account = msg.account  # local_cache_attribute
    coin_name, network_label, coin_type = helpers.validate_network_account(
        network, account
    )

    seed.raise_if_not_initialized()
    session = helpers.snapshot_session()
    account_label = helpers.account_label(account)
    path = helpers.account_path(coin_type, account)

    await confirm_action(
        "zcash_export_viewing_key",
        TR.zcash__export_viewing_key,
        action=f"Zcash {network_label}\n{account_label}\n{path}",
        description=TR.zcash__viewing_key_warning,
        br_code=ButtonRequestType.SignTx,
        prompt_screen=True,
        # Hold: an exported viewing key reveals the account's whole history
        # and cannot be revoked.
        hold=True,
    )
    helpers.require_session(session)

    # The viewing key is per account; the ZIP-32 seed fingerprint is not. It
    # is the same value for every account index and for both networks, so it
    # links everything this seed ever signs. The screen above promises account
    # scope, so the fingerprint needs its own opt-in and its own warning.
    include_fingerprint = bool(msg.include_seed_fingerprint)
    if include_fingerprint:
        await show_warning(
            br_name="zcash_seed_fingerprint",
            content=TR.zcash__seed_fingerprint_warning,
            br_code=ButtonRequestType.Warning,
        )
        helpers.require_session(session)

    if helpers.has_weak_backup():
        await show_warning(
            br_name="zcash_weak_backup",
            content=TR.zcash__weak_backup_warning,
            br_code=ButtonRequestType.Warning,
        )
        helpers.require_session(session)

    wallet_seed = await seed.get_seed()
    raw_fvk = bytearray(96)
    # The ZIP-32 seed fingerprint is a public identifier of the seed (a
    # one-way hash), derived only when the request asked for it and the user
    # acknowledged the extra warning above.
    fingerprint = bytearray(32) if include_fingerprint else None
    # Show progress during the blocking native derivation. It is a static
    # ring: `ProgressLayout` only moves on `report()`, and nothing can call it
    # while the native call holds the VM.
    progress_layout = progress(indeterminate=True)
    progress_layout.report(0)
    try:
        helpers.require_session(session)
        try:
            _derive_viewing_key(wallet_seed, network, account, raw_fvk)
            key = unified_addresses.encode_fvk(raw_fvk, coininfo.by_name(coin_name))
            if fingerprint is not None:
                _derive_seed_fingerprint(wallet_seed, fingerprint)
        except (KeyError, ValueError, RuntimeError):
            raise wire.ProcessError("Zcash viewing key derivation failed")
    finally:
        utils.memzero(raw_fvk)
        del wallet_seed

    helpers.require_session(session)
    return ZcashViewingKey(
        key=key,
        seed_fingerprint=None if fingerprint is None else bytes(fingerprint),
    )
