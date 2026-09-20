"""Shared wallet and account policy for Zcash/Ironwood workflows."""

from typing import TYPE_CHECKING

from trezor import utils, wire
from trezor.wire import context

from apps.common import backup_types, mnemonic

if TYPE_CHECKING:
    from buffer_types import AnyBytes

    SessionIdentity = bytes | tuple[AnyBytes, AnyBytes]


MALFORMED_REQUEST = "Malformed Zcash request"
POLICY_VIOLATION = "Zcash request violates device policy"
UNSUPPORTED_BACKUP = "Zcash wallet backup is unsupported"


def validate_network_account(network: int, account: int) -> tuple[str, str, int]:
    """Return the device-owned coin name, UI label, and ZIP-32 coin type."""
    from trezor.enums import ZcashNetwork

    if type(network) is not int or type(account) is not int:
        raise wire.DataError(MALFORMED_REQUEST)
    if account < 0 or account > 0x7FFF_FFFF:
        raise wire.ProcessError(POLICY_VIOLATION)

    if network == ZcashNetwork.Mainnet:
        return "Zcash", "Mainnet", 133
    if network == ZcashNetwork.Testnet:
        return "Zcash Testnet", "Testnet", 1
    raise wire.ProcessError(POLICY_VIOLATION)


def validate_diversifier_index(diversifier_index: AnyBytes) -> None:
    # The protobuf decoder owns the buffer type; this policy owns its exact width.
    if len(diversifier_index) != 11:
        raise wire.DataError(MALFORMED_REQUEST)


def account_path(coin_type: int, account: int) -> str:
    from apps.common import paths

    hardened = paths.HARDENED
    return paths.address_n_to_str(
        [32 | hardened, coin_type | hardened, account | hardened]
    )


def account_label(account: int) -> str:
    return f"ZEC #{account + 1}"


def snapshot_session() -> SessionIdentity:
    if utils.USE_THP:
        identity = context.try_get_ctx_ids()
        if identity is None:
            raise wire.InvalidSession()
        return identity
    return context.get_context().cache.export_session_id()


def require_session(identity: SessionIdentity) -> None:
    if snapshot_session() != identity:
        raise wire.InvalidSession()


def has_weak_backup() -> bool:
    """Recognize existing backups that ZIP-315 says wallets should warn about.

    The result is device-wide and constant until the mnemonic changes, so it is
    memoized in the sessionless cache (cleared on wipe/recovery, NOT on lock, so
    the cache — and thus the single secret read — is per boot, not per unlock).
    This keeps the mnemonic secret from being copied onto the GC heap on every
    receive, export and sign; only the first call per boot touches it (Fable
    review R2/V1). Residual (deferred, #S2): that one read still allocates an
    unzeroized GC `bytes` of the whole secret; eliminating it means persisting a
    public backup-strength flag at `store_mnemonic_secret` time, a cross-app
    change out of scope here.
    """
    from storage.cache_common import APP_ZCASH_WEAK_BACKUP

    cached = context.cache_get_int(APP_ZCASH_WEAK_BACKUP)
    if cached is not None:
        return cached == 1

    weak = _compute_weak_backup()
    context.cache_set_int(APP_ZCASH_WEAK_BACKUP, 1 if weak else 0)
    return weak


def _compute_weak_backup() -> bool:
    from trezor.enums import BackupType

    secret = mnemonic.get_secret()
    if secret is None:
        raise wire.ProcessError(UNSUPPORTED_BACKUP)

    backup_type = mnemonic.get_type()
    if backup_type == BackupType.Bip39:
        # Avoid split(), which would leave a list of secret word copies on the heap.
        word_count = 1
        for byte in secret:
            if byte == 0x20:
                word_count += 1
        if word_count not in (12, 18, 24):
            raise wire.ProcessError(UNSUPPORTED_BACKUP)
        return word_count != 24

    if backup_types.is_slip39_backup_type(backup_type):
        if len(secret) not in (16, 32):
            raise wire.ProcessError(UNSUPPORTED_BACKUP)
        return len(secret) == 16

    raise wire.ProcessError(UNSUPPORTED_BACKUP)
