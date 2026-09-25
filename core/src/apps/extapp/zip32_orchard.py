"""ZIP-32 Orchard account keys for extapps.

An app that declares the pseudo-curve `zip32-orchard` in its header may ask
for the spending key of a ZIP-32 Orchard account `m/32'/coin_type'/account'`
that its declared paths allow, once the user has allowed it. Core derives the
key from the wallet seed with BLAKE2b only; the app does all Orchard (Pallas)
work itself.

ZIP 32: https://zips.z.cash/zip-0032 ("Orchard master key generation",
"Orchard child key derivation", "Seed Fingerprints").
"""

from micropython import const
from typing import TYPE_CHECKING

from trezor.crypto.hashlib import blake2b
from trezor.wire.errors import DataError

from apps.common.paths import HARDENED

if TYPE_CHECKING:
    from buffer_types import AnyBytes

    from trezor.enums import BackupType

    from apps.common.paths import Bip32Path, PathSchema

# Pseudo-curve an app declares in its header `curves` to receive Orchard
# spending keys. run.py allows one curve per app, so such an app cannot also
# use the BIP-32 operations.
_ENTITLEMENT = "zip32-orchard"

_PURPOSE = const(32)
_MAINNET = const(133)  # SLIP-44
_TESTNET = const(1)

# ZIP 32 requires 32..252-byte seeds. A restored 128-bit SLIP-39 backup has a
# 16-byte seed, which is accepted too.
_SLIP39_SEED_LENGTH = const(16)
_MIN_SEED_LENGTH = const(32)
_MAX_SEED_LENGTH = const(252)

# Accounts the user allowed, per session: the app instance id (u32 LE), then
# up to `_MAX_APPROVALS` entries of coin type (u8) and account (u32 LE).
_MAX_APPROVALS = const(8)
_APPROVAL_SIZE = const(5)


async def get_account(
    curve: str,
    schemas: list[PathSchema],
    coin_type: int,
    account: int,
    app_name: str,
    instance_id: int,
) -> tuple[AnyBytes, bytes, bool]:
    """The account's spending key, the seed fingerprint and the weak-backup bit.

    Raises `ActionCancelled` if the user does not allow the app the account.
    """
    from storage.cache_common import APP_EXTAPP_ZIP32_APPROVALS
    from trezor.wire import context

    from apps.common import mnemonic
    from apps.common.seed import get_seed

    path = allowed_account_path(curve, schemas, coin_type, account)

    approvals = context.cache_get(APP_EXTAPP_ZIP32_APPROVALS)
    if not is_approved(approvals, instance_id, coin_type, account):
        await _confirm_account(app_name, coin_type, account)
        approvals = with_approval(approvals, instance_id, coin_type, account)
        context.cache_set(APP_EXTAPP_ZIP32_APPROVALS, approvals)

    seed = await get_seed()
    return (
        derive_spending_key(seed, path),
        seed_fingerprint(seed),
        has_weak_backup(*mnemonic.get()),
    )


def allowed_account_path(
    curve: str, schemas: list[PathSchema], coin_type: int, account: int
) -> Bip32Path:
    """The path `m/32'/coin_type'/account'`, if this app may have its key.

    The app must hold the entitlement and declare the path. The path check is
    a plain schema match: unlike `paths.validate_path`, it never falls back to
    a prompt when safety checks are relaxed.
    """
    if curve != _ENTITLEMENT:
        raise DataError("App may not receive ZIP-32 Orchard keys")
    if coin_type not in (_MAINNET, _TESTNET) or not 0 <= account < HARDENED:
        raise DataError("Invalid ZIP-32 Orchard account")
    path = [_PURPOSE | HARDENED, coin_type | HARDENED, account | HARDENED]
    for schema in schemas:
        if schema.match(path):
            return path
    raise DataError("Path not allowed for this app")


async def _confirm_account(app_name: str, coin_type: int, account: int) -> None:
    from trezor.enums import ButtonRequestType
    from trezor.ui.layouts import confirm_action

    network = "Mainnet" if coin_type == _MAINNET else "Testnet"
    await confirm_action(
        "zip32_orchard_account",
        "Zcash account",
        description=(
            f"Allow {app_name} to use your Zcash account #{account + 1} "
            f"({network})? It can see your balance and create transactions "
            "for you to confirm."
        ),
        verb="Allow",
        br_code=ButtonRequestType.Other,
    )


def _approval(coin_type: int, account: int) -> bytes:
    return bytes([coin_type]) + account.to_bytes(4, "little")


def is_approved(
    approvals: bytes | None, instance_id: int, coin_type: int, account: int
) -> bool:
    """Whether the user allowed this app instance the account."""
    if not approvals or approvals[:4] != instance_id.to_bytes(4, "little"):
        return False
    entry = _approval(coin_type, account)
    for offset in range(4, len(approvals), _APPROVAL_SIZE):
        if approvals[offset : offset + _APPROVAL_SIZE] == entry:
            return True
    return False


def with_approval(
    approvals: bytes | None, instance_id: int, coin_type: int, account: int
) -> bytes:
    """`approvals` plus the account, keeping the newest `_MAX_APPROVALS`."""
    prefix = instance_id.to_bytes(4, "little")
    entries = b""
    if approvals and approvals[:4] == prefix:
        entries = approvals[4:]
    entries += _approval(coin_type, account)
    return prefix + entries[-_MAX_APPROVALS * _APPROVAL_SIZE :]


def _check_seed_length(seed: AnyBytes) -> None:
    if len(seed) != _SLIP39_SEED_LENGTH and not (
        _MIN_SEED_LENGTH <= len(seed) <= _MAX_SEED_LENGTH
    ):
        raise DataError("Unsupported seed length")


def derive_spending_key(seed: AnyBytes, path: Bip32Path) -> memoryview:
    """The Orchard spending key at the hardened `path` below the ZIP-32 master.

    `blake2b.digest()` returns immutable `bytes`, which cannot be wiped; slices
    are views so as not to add further copies.
    """
    _check_seed_length(seed)
    # Master: I = BLAKE2b-512("ZcashIP32Orchard", seed); sk = I[:32], c = I[32:].
    i = memoryview(blake2b(seed, outlen=64, personal=b"ZcashIP32Orchard").digest())
    for index in path:
        if not index & HARDENED:
            raise DataError("Orchard derivation is hardened only")
        # Child: I = PRF^expand(c_par, [0x81] || sk_par || I2LEOSP_32(index)),
        # where PRF^expand(k, t) = BLAKE2b-512("Zcash_ExpandSeed", k || t).
        h = blake2b(outlen=64, personal=b"Zcash_ExpandSeed")
        h.update(i[32:])
        h.update(b"\x81")
        h.update(i[:32])
        h.update(index.to_bytes(4, "little"))
        i = memoryview(h.digest())
    return i[:32]


def seed_fingerprint(seed: AnyBytes) -> bytes:
    """BLAKE2b-256("Zcash_HD_Seed_FP", I2LEOSP_8(len(seed)) || seed).

    ZIP 32 defines this only for seeds of 32 to 252 bytes. The 16-byte seed of a
    restored 128-bit SLIP-39 backup uses the same construction with length byte 16.
    """
    _check_seed_length(seed)
    h = blake2b(outlen=32, personal=b"Zcash_HD_Seed_FP")
    h.update(bytes([len(seed)]))
    h.update(seed)
    return h.digest()


def has_weak_backup(secret: bytes | None, backup_type: BackupType) -> bool:
    """Whether ZIP 315 asks wallets to warn about this backup.

    Weak: a 12- or 18-word BIP-39 mnemonic, or a 128-bit SLIP-39 secret.
    Any other kind of backup is refused rather than guessed.
    """
    from trezor.enums import BackupType

    from apps.common import backup_types

    if secret is None:
        raise DataError("Unsupported backup")

    if backup_type == BackupType.Bip39:
        word_count = secret.count(b" ") + 1
        if word_count not in (12, 18, 24):
            raise DataError("Unsupported backup")
        return word_count != 24

    if backup_types.is_slip39_backup_type(backup_type):
        if len(secret) not in (16, 32):
            raise DataError("Unsupported backup")
        return len(secret) == 16

    raise DataError("Unsupported backup")
