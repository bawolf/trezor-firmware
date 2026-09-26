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

from apps.common.paths import HARDENED

if TYPE_CHECKING:
    from buffer_types import AnyBytes

    from trezor.enums import BackupType

    from apps.common.paths import Bip32Path, PathSchema

# Declared in an app's curves to receive Orchard spending keys; run.py allows
# one curve per app.
_CURVE = "zip32-orchard"

_PURPOSE = const(32)
_MAINNET = const(133)  # SLIP-44
_TESTNET = const(1)

# ZIP 32 requires 32..252-byte seeds. A restored 128-bit SLIP-39 backup has a
# 16-byte seed, which is accepted too.
_SLIP39_SEED_LENGTH = const(16)
_MIN_SEED_LENGTH = const(32)
_MAX_SEED_LENGTH = const(252)

# Accounts the user allowed, per session: the app they were allowed to (see
# `app_instance`), then up to `_MAX_APPROVALS` entries of coin type (u8) and
# account (u32 LE). An app may use several accounts on both networks; the
# newest ones are kept.
_APP_INSTANCE_SIZE = const(36)  # 4 + 32
_MAX_APPROVALS = const(8)
_APPROVAL_SIZE = const(5)


async def get_account(
    curve: str,
    schemas: list[PathSchema],
    coin_type: int,
    account: int,
    app_name: str,
    instance: bytes,
) -> tuple[AnyBytes, bytes, bool]:
    """The account's spending key, the seed fingerprint and the weak-backup bit.

    Raises `ActionCancelled` if the user does not allow the app the account.
    """
    from storage.cache_common import APP_EXTAPP_ZIP32_APPROVALS
    from trezor.wire import context

    from apps.common import mnemonic
    from apps.common.seed import get_seed

    path = allowed_account_path(curve, schemas, coin_type, account)
    weak_backup = has_weak_backup(*mnemonic.get())

    approvals = context.cache_get(APP_EXTAPP_ZIP32_APPROVALS)
    if not is_approved(approvals, instance, coin_type, account):
        await _confirm_account(app_name, coin_type, account)
        approvals = with_approval(approvals, instance, coin_type, account)
        context.cache_set(APP_EXTAPP_ZIP32_APPROVALS, approvals)

    seed = await get_seed()
    check_seed_length(seed)
    return derive_spending_key(seed, path), seed_fingerprint(seed), weak_backup


def allowed_account_path(
    curve: str, schemas: list[PathSchema], coin_type: int, account: int
) -> Bip32Path:
    """The path `m/32'/coin_type'/account'`, if this app may have its key.

    The app must declare the `zip32-orchard` curve and the path. The path
    check is a plain schema match: unlike `paths.validate_path`, it never falls
    back to a prompt when safety checks are relaxed.
    """
    if curve != _CURVE:
        raise ValueError  # the app did not declare the zip32-orchard curve
    if coin_type not in (_MAINNET, _TESTNET) or not 0 <= account < HARDENED:
        raise ValueError  # not a ZIP-32 Orchard account
    path = [_PURPOSE | HARDENED, coin_type | HARDENED, account | HARDENED]
    for schema in schemas:
        if schema.match(path):
            return path
    raise ValueError  # the app did not declare this account's path


async def _confirm_account(app_name: str, coin_type: int, account: int) -> None:
    from trezor import TR
    from trezor.enums import ButtonRequestType
    from trezor.ui.layouts import confirm_action

    network = TR.extapp__mainnet if coin_type == _MAINNET else TR.extapp__testnet
    await confirm_action(
        "zip32_orchard_account",
        TR.extapp__spending_key,
        description=TR.extapp__spending_key_template.format(
            app_name, account + 1, network
        ),
        verb=TR.buttons__hold_to_confirm,
        hold=True,
        br_code=ButtonRequestType.Other,
    )


def app_instance(instance_id: int, fingerprint: bytes) -> bytes:
    """The app an approval is for: its instance id (u32 LE) and the fingerprint
    of its verified header."""
    return instance_id.to_bytes(4, "little") + fingerprint


def _approval(coin_type: int, account: int) -> bytes:
    return bytes([coin_type]) + account.to_bytes(4, "little")


def is_approved(
    approvals: bytes | None, instance: bytes, coin_type: int, account: int
) -> bool:
    """Whether the user allowed this app instance the account."""
    if not approvals or approvals[:_APP_INSTANCE_SIZE] != instance:
        return False
    entry = _approval(coin_type, account)
    for offset in range(_APP_INSTANCE_SIZE, len(approvals), _APPROVAL_SIZE):
        if approvals[offset : offset + _APPROVAL_SIZE] == entry:
            return True
    return False


def with_approval(
    approvals: bytes | None, instance: bytes, coin_type: int, account: int
) -> bytes:
    """`approvals` plus the account, keeping the newest `_MAX_APPROVALS`."""
    entries = b""
    if approvals and approvals[:_APP_INSTANCE_SIZE] == instance:
        entries = approvals[_APP_INSTANCE_SIZE:]
    entries += _approval(coin_type, account)
    return instance + entries[-_MAX_APPROVALS * _APPROVAL_SIZE :]


def check_seed_length(seed: AnyBytes) -> None:
    if len(seed) != _SLIP39_SEED_LENGTH and not (
        _MIN_SEED_LENGTH <= len(seed) <= _MAX_SEED_LENGTH
    ):
        raise ValueError  # ZIP 32 defines no key for this seed


def derive_spending_key(seed: AnyBytes, path: Bip32Path) -> memoryview:
    """The Orchard spending key at the hardened `path` below the ZIP-32 master.

    `blake2b.digest()` returns immutable `bytes`, which cannot be wiped; slices
    are views so as not to add further copies.
    """
    # Master: I = BLAKE2b-512("ZcashIP32Orchard", seed); sk = I[:32], c = I[32:].
    i = memoryview(blake2b(seed, outlen=64, personal=b"ZcashIP32Orchard").digest())
    for index in path:
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
    h = blake2b(outlen=32, personal=b"Zcash_HD_Seed_FP")
    h.update(bytes([len(seed)]))
    h.update(seed)
    return h.digest()


def has_weak_backup(secret: bytes | None, backup_type: BackupType) -> bool:
    """Whether ZIP 315 asks wallets to warn about this backup: it holds fewer
    than 256 bits (a BIP-39 mnemonic of fewer than 24 words, or a SLIP-39
    secret shorter than 32 bytes).

    Any other kind of backup is refused rather than guessed.
    """
    from trezor.enums import BackupType

    from apps.common import backup_types

    if secret is None:
        raise ValueError  # no backup

    if backup_type == BackupType.Bip39:
        return secret.count(b" ") + 1 < 24

    if backup_types.is_slip39_backup_type(backup_type):
        return len(secret) < 32

    raise ValueError  # an unknown kind of backup
