# flake8: noqa: F403,F405
from common import *  # isort:skip

from mock import patch
from trezor import utils, wire
from trezor.enums import BackupType, ZcashNetwork
from trezor.wire import context

from apps.common import mnemonic
from apps.zcash import ironwood_account


class _Cache:
    def __init__(self, identity: bytes) -> None:
        self.identity = identity

    def export_session_id(self) -> bytes:
        return self.identity


class _Context:
    def __init__(self, identity: bytes) -> None:
        self.cache = _Cache(identity)


@unittest.skipUnless(not utils.BITCOIN_ONLY, "altcoin")
class TestIronwoodAccount(unittest.TestCase):
    def test_network_account_and_path_policy(self):
        self.assertEqual(
            ironwood_account.validate_network_account(ZcashNetwork.Mainnet, 0),
            ("Zcash", "Mainnet", 133),
        )
        self.assertEqual(
            ironwood_account.validate_network_account(
                ZcashNetwork.Testnet, 0x7FFF_FFFF
            ),
            ("Zcash Testnet", "Testnet", 1),
        )
        self.assertEqual(ironwood_account.account_path(133, 7), "m/32'/133'/7'")
        self.assertEqual(ironwood_account.account_label(7), "ZEC #8")

        for network, account in ((True, 0), (ZcashNetwork.Mainnet, False)):
            with self.assertRaises(wire.DataError) as raised:
                ironwood_account.validate_network_account(network, account)
            self.assertEqual(raised.value.message, "Malformed Zcash request")

        for network, account in ((2, 0), (ZcashNetwork.Mainnet, -1), (0, 1 << 31)):
            with self.assertRaises(wire.ProcessError) as raised:
                ironwood_account.validate_network_account(network, account)
            self.assertEqual(
                raised.value.message,
                "Zcash request violates device policy",
            )

    def test_diversifier_policy(self):
        ironwood_account.validate_diversifier_index(bytes(11))
        for invalid in (bytes(10), bytes(12)):
            with self.assertRaises(wire.DataError) as raised:
                ironwood_account.validate_diversifier_index(invalid)
            self.assertEqual(raised.value.message, "Malformed Zcash request")

        ironwood_account.validate_diversifier_index(memoryview(bytes(11)))

    def test_backup_strength_policy(self):
        from storage import cache as storage_cache

        cases = (
            (BackupType.Bip39, b"word " * 11 + b"word", True),
            (BackupType.Bip39, b"word " * 17 + b"word", True),
            (BackupType.Bip39, b"word " * 23 + b"word", False),
            (BackupType.Slip39_Basic, bytes(16), True),
            (BackupType.Slip39_Advanced_Extendable, bytes(32), False),
        )
        for backup_type, secret, expected in cases:
            # The predicate is memoized in the sessionless cache; on a real
            # device that cache is cleared whenever the mnemonic can change.
            storage_cache.get_sessionless_cache().clear()
            with patch(mnemonic, "get_type", lambda: backup_type):
                with patch(mnemonic, "get_secret", lambda: secret):
                    self.assertEqual(ironwood_account.has_weak_backup(), expected)

        storage_cache.get_sessionless_cache().clear()
        with patch(mnemonic, "get_type", lambda: BackupType.Bip39):
            with patch(mnemonic, "get_secret", lambda: b"invalid"):
                with self.assertRaises(wire.ProcessError) as raised:
                    ironwood_account.has_weak_backup()
                self.assertEqual(
                    raised.value.message, "Zcash wallet backup is unsupported"
                )

    def test_weak_backup_predicate_is_memoized(self):
        # The mnemonic secret must not be re-copied onto the GC heap
        # on every receive/export/sign. The predicate is computed once and cached
        # in the sessionless cache; only clearing that cache re-reads the secret.
        from storage import cache as storage_cache

        storage_cache.get_sessionless_cache().clear()
        reads = []

        def counting_secret():
            reads.append(1)
            return b"word " * 11 + b"word"

        with patch(mnemonic, "get_type", lambda: BackupType.Bip39):
            with patch(mnemonic, "get_secret", counting_secret):
                self.assertTrue(ironwood_account.has_weak_backup())
                self.assertTrue(ironwood_account.has_weak_backup())
                self.assertTrue(ironwood_account.has_weak_backup())
        # Secret read exactly once despite three queries.
        self.assertEqual(len(reads), 1)

        # Clearing the cache (as wipe/recovery does) forces a fresh computation.
        storage_cache.get_sessionless_cache().clear()
        with patch(mnemonic, "get_type", lambda: BackupType.Bip39):
            with patch(mnemonic, "get_secret", counting_secret):
                self.assertTrue(ironwood_account.has_weak_backup())
        self.assertEqual(len(reads), 2)

    def test_legacy_session_identity(self):
        ctx = _Context(b"session-a")
        with patch(utils, "USE_THP", False):
            with patch(context, "get_context", lambda: ctx):
                snapshot = ironwood_account.snapshot_session()
                ironwood_account.require_session(snapshot)
                ctx.cache.identity = b"session-b"
                with self.assertRaises(wire.InvalidSession):
                    ironwood_account.require_session(snapshot)

    def test_thp_session_identity(self):
        identity = [(b"channel", b"session")]
        with patch(utils, "USE_THP", True):
            with patch(context, "try_get_ctx_ids", lambda: identity[0]):
                snapshot = ironwood_account.snapshot_session()
                ironwood_account.require_session(snapshot)
                identity[0] = (b"other-channel", b"session")
                with self.assertRaises(wire.InvalidSession):
                    ironwood_account.require_session(snapshot)

            with patch(context, "try_get_ctx_ids", lambda: None):
                with self.assertRaises(wire.InvalidSession):
                    ironwood_account.snapshot_session()


if __name__ == "__main__":
    unittest.main()
