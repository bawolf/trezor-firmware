# flake8: noqa: F403,F405
from common import *  # isort:skip

from trezor.crypto import bip39
from trezor.enums import BackupType
from trezor.wire import DataError

from apps.common.paths import PathSchema

if utils.USE_APP_LOADING:
    from apps.extapp.zip32_orchard import (
        allowed_account_path,
        derive_spending_key,
        has_weak_backup,
        is_approved,
        seed_fingerprint,
        with_approval,
    )


def schemas(*patterns: str) -> list[PathSchema]:
    return [PathSchema.parse(pattern, 133) for pattern in patterns]


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestZip32OrchardDerivation(unittest.TestCase):
    def test_zip32_orchard_vectors(self):
        # source: zcash-test-vectors orchard_zip32.py (as vendored in
        # orchard 0.15.5 src/test_vectors/zip32.rs)
        seed = bytes(range(32))
        vectors = [
            ([], "7eee3c1017870990a3dd6891b82f80be8976c1e7dc20d60817a5e88e8b2cd4b8"),
            (
                [H_(1)],
                "98d703fcb40504c95b3b6ed10ecd50082cff97dfd1dd9aa0913c78f977c962af",
            ),
            (
                [H_(1), H_(2)],
                "99afd8894baad58784d0ec08f5148ee2c2a17b2b294b08ef9e0a0cf14bcc0920",
            ),
            (
                [H_(1), H_(2), H_(3)],
                "96439ea348a4b2ce4ec7beb4543c70274c8f76495d60c5fa5f018b68f3c32367",
            ),
        ]
        for path, sk in vectors:
            self.assertEqual(bytes(derive_spending_key(seed, path)), bytes.fromhex(sk))

    def test_account_key(self):
        # "abandon" x11 + "about", mainnet account 0; equals orchard 0.15.5
        # SpendingKey::from_zip32_seed(seed, 133, 0).
        seed = bip39.seed(" ".join(["abandon"] * 11 + ["about"]), "")
        self.assertEqual(
            bytes(derive_spending_key(seed, [H_(32), H_(133), H_(0)])),
            bytes.fromhex(
                "38a11cff85ebbd46615b70e95575139b45564162abbe2ef27c680e47e691dcd3"
            ),
        )

    def test_non_hardened_index(self):
        with self.assertRaises(DataError):
            derive_spending_key(bytes(32), [H_(32), H_(133), 0])

    def test_seed_fingerprint_vector(self):
        # source: zip32 0.2.1 src/fingerprint.rs `test_seed_fingerprint`
        self.assertEqual(
            seed_fingerprint(bytes(range(32))),
            bytes.fromhex(
                "deff604c246710f7176dead02aa746f2fd8d5389f7072556dcb555fdbe5e3ae3"
            ),
        )

    def test_seed_fingerprint_slip39_128(self):
        # The 16-byte restored SLIP-39 seed: the same construction, length byte
        # 16 (value from CPython hashlib.blake2b; zip32 refuses this length).
        self.assertEqual(
            seed_fingerprint(bytes([0x11] * 16)),
            bytes.fromhex(
                "53ff5ed62e710d54f0b7fdd4a5c37f75d1c7083e613e57e0d165b57309e914f0"
            ),
        )

    def test_seed_length(self):
        for length in (0, 15, 17, 31, 253):
            with self.assertRaises(DataError):
                seed_fingerprint(bytes(length))
            with self.assertRaises(DataError):
                derive_spending_key(bytes(length), [])
        for length in (16, 32, 64, 252):
            seed_fingerprint(bytes(length))
            derive_spending_key(bytes(length), [])


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestZip32OrchardPolicy(unittest.TestCase):
    CURVE = "zip32-orchard"
    SCHEMAS = schemas("m/32'/133'/[0-2147483647]'")

    def test_allowed(self):
        self.assertEqual(
            allowed_account_path(self.CURVE, self.SCHEMAS, 133, 0),
            [H_(32), H_(133), H_(0)],
        )
        self.assertEqual(
            allowed_account_path(self.CURVE, self.SCHEMAS, 133, 0x7FFF_FFFF),
            [H_(32), H_(133), H_(0x7FFF_FFFF)],
        )
        # One app may declare both networks.
        both = [
            PathSchema.parse("m/32'/133'/[0-9]'", 133),
            PathSchema.parse("m/32'/1'/[0-9]'", 1),
        ]
        self.assertEqual(
            allowed_account_path(self.CURVE, both, 1, 5), [H_(32), H_(1), H_(5)]
        )
        self.assertEqual(
            allowed_account_path(self.CURVE, both, 133, 5),
            [H_(32), H_(133), H_(5)],
        )

    def test_refused_without_entitlement(self):
        for curve in ("secp256k1", "ed25519", ""):
            with self.assertRaises(DataError):
                allowed_account_path(curve, self.SCHEMAS, 133, 0)

    def test_refused_outside_declared_paths(self):
        cases = [
            (schemas("m/32'/133'/[0-9]'"), 133, 10),
            (schemas("m/32'/1'/[0-2147483647]'"), 133, 0),
            (self.SCHEMAS, 1, 0),
            (schemas("m/44'/133'/account'/*"), 133, 0),
            ([], 133, 0),
        ]
        for declared, coin_type, account in cases:
            with self.assertRaises(DataError):
                allowed_account_path(self.CURVE, declared, coin_type, account)

    def test_refused_account_out_of_range(self):
        anything = schemas("m/**")
        for account in (0x8000_0000, 0xFFFF_FFFF, -1):
            with self.assertRaises(DataError):
                allowed_account_path(self.CURVE, anything, 133, account)

    def test_refused_other_coin_types(self):
        anything = schemas("m/**")
        for coin_type in (0, 2, 60, 132, 134, H_(133), H_(1)):
            with self.assertRaises(DataError):
                allowed_account_path(self.CURVE, anything, coin_type, 0)


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestZip32OrchardApprovals(unittest.TestCase):
    def test_approval_is_per_instance_and_account(self):
        approvals = with_approval(None, 7, 133, 0)
        self.assertTrue(is_approved(approvals, 7, 133, 0))
        self.assertFalse(is_approved(approvals, 7, 133, 1))
        self.assertFalse(is_approved(approvals, 7, 1, 0))
        self.assertFalse(is_approved(approvals, 8, 133, 0))
        self.assertFalse(is_approved(None, 7, 133, 0))

        approvals = with_approval(approvals, 7, 1, 0)
        self.assertTrue(is_approved(approvals, 7, 133, 0))
        self.assertTrue(is_approved(approvals, 7, 1, 0))

        # A new app instance starts from nothing.
        approvals = with_approval(approvals, 8, 133, 1)
        self.assertFalse(is_approved(approvals, 7, 133, 0))
        self.assertFalse(is_approved(approvals, 8, 133, 0))
        self.assertTrue(is_approved(approvals, 8, 133, 1))

    def test_keeps_the_newest_eight(self):
        approvals = None
        for account in range(10):
            approvals = with_approval(approvals, 7, 133, account)
        self.assertEqual(len(approvals), 4 + 8 * 5)
        for account in range(10):
            self.assertEqual(is_approved(approvals, 7, 133, account), account >= 2)


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestZip32OrchardWeakBackup(unittest.TestCase):
    def test_bip39(self):
        for words, weak in ((12, True), (18, True), (24, False)):
            secret = " ".join(["abandon"] * words).encode()
            self.assertEqual(has_weak_backup(secret, BackupType.Bip39), weak)
        for words in (1, 15, 21, 25):
            secret = " ".join(["abandon"] * words).encode()
            with self.assertRaises(DataError):
                has_weak_backup(secret, BackupType.Bip39)

    def test_slip39(self):
        for backup_type in (
            BackupType.Slip39_Basic,
            BackupType.Slip39_Advanced,
            BackupType.Slip39_Single_Extendable,
            BackupType.Slip39_Basic_Extendable,
            BackupType.Slip39_Advanced_Extendable,
        ):
            self.assertTrue(has_weak_backup(bytes(16), backup_type))
            self.assertFalse(has_weak_backup(bytes(32), backup_type))
            with self.assertRaises(DataError):
                has_weak_backup(bytes(24), backup_type)

    def test_missing_secret(self):
        with self.assertRaises(DataError):
            has_weak_backup(None, BackupType.Bip39)


if __name__ == "__main__":
    unittest.main()
