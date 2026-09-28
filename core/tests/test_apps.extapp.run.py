# flake8: noqa: F403,F405
from common import *  # isort:skip

import ustruct
from mock import patch
from storage import cache_common
from storage.cache import get_sessionless_cache
from trezor import io, loop
from trezor.crypto import bip39
from trezor.crypto.curve import secp256k1
from trezor.messages import ExtAppMessage
from trezor.wire import DataError, context

from apps.common.keychain import get_keychain
from apps.common.paths import PathSchema

if not utils.USE_THP:
    from storage import cache_codec

if utils.USE_APP_LOADING:
    from apps.extapp import run as extapp_run
    from apps.extapp.run import _get_public_key, _get_xpub, _path_schemas

if utils.USE_APP_LOADING and not utils.BITCOIN_ONLY:
    from apps.extapp.run import _sign_typed_hash


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappPathSchemas(unittest.TestCase):
    def test_one_coin_type(self):
        schemas, slip44_id = _path_schemas(
            ["m/44'/60'/account'", "m/45'/60/account/change/address_index"]
        )
        self.assertEqual(slip44_id, 60)
        self.assertTrue(schemas[0].match([H_(44), H_(60), H_(0)]))
        self.assertTrue(schemas[1].match([H_(45), 60, 0, 0, 0]))

    def test_coin_type_per_pattern(self):
        schemas, slip44_id = _path_schemas(["m/32'/133'/account'", "m/32'/1'/account'"])
        self.assertIsNone(slip44_id)
        self.assertTrue(schemas[0].match([H_(32), H_(133), H_(0)]))
        self.assertFalse(schemas[0].match([H_(32), H_(1), H_(0)]))
        self.assertTrue(schemas[1].match([H_(32), H_(1), H_(0)]))
        self.assertFalse(schemas[1].match([H_(32), H_(133), H_(0)]))

    def test_no_patterns(self):
        with self.assertRaises(DataError):
            _path_schemas([])

    def test_pattern_without_coin_type(self):
        with self.assertRaises(DataError):
            _path_schemas(["m/44'/coin_type'/account'"])

    def test_malformed_pattern(self):
        for pattern in (
            "m",
            "m/44'",
            "m/44'//0'",
            "m/44'/60'/",
            "m/44'/60'/'",
            "m/44'/60'/x",
            "44'/60'/0'",
        ):
            with self.assertRaises(DataError):
                _path_schemas([pattern])


_HASH = bytes(range(32))
_ETHEREUM_PATTERN = "m/44'/60'/account'/change/address_index/**"
_TRON_PATTERN = "m/44'/195'/account'/change/address_index/**"
_ETHEREUM_PATH = [H_(44), H_(60), H_(0), 0, 0]


class _TestCaseWithSeed(TestCaseWithContext):
    def setUp(self):
        seed = bip39.seed(" ".join(["all"] * 12), "")
        if utils.USE_THP:
            context.cache_set(cache_common.APP_COMMON_SEED, seed)
        else:
            cache_codec.start_session()
            cache_codec.get_active_session().set(cache_common.APP_COMMON_SEED, seed)


@unittest.skipUnless(utils.USE_APP_LOADING and not utils.BITCOIN_ONLY, "app loading")
class TestExtappSignTypedHash(_TestCaseWithSeed):
    def sign(self, curve, pattern, coin_type, address_n, chain_id):
        schemas = [PathSchema.parse(pattern, coin_type)]
        return await_result(
            _sign_typed_hash(
                curve, schemas, address_n, _HASH, None, None, chain_id, False
            )
        )

    def assert_signed_by(self, signature, address_n):
        schemas = [PathSchema.parse("m/**", 0)]
        node = await_result(get_keychain("secp256k1", schemas)).derive(address_n)
        self.assertTrue(secp256k1.verify(node.public_key(), signature, _HASH))

    def test_ethereum_app(self):
        for chain_id in (None, 1):
            signature = self.sign(
                "secp256k1", _ETHEREUM_PATTERN, 60, _ETHEREUM_PATH, chain_id
            )
            self.assert_signed_by(signature, _ETHEREUM_PATH)

    def test_ethereum_app_on_the_network_of_the_definitions(self):
        # Ethereum Classic, SLIP-44 61.
        path = [H_(44), H_(61), H_(0), 0, 0]
        signature = self.sign("secp256k1", _ETHEREUM_PATTERN, 60, path, 61)
        self.assert_signed_by(signature, path)

    def test_app_without_ethereum_paths(self):
        with self.assertRaises(DataError):
            self.sign("secp256k1", _TRON_PATTERN, 195, _ETHEREUM_PATH, None)
        with self.assertRaises(ValueError):
            self.sign("secp256k1", _TRON_PATTERN, 195, _ETHEREUM_PATH, 1)

    def test_app_with_another_curve(self):
        for chain_id in (None, 1):
            with self.assertRaises(ValueError):
                self.sign("ed25519", _ETHEREUM_PATTERN, 60, _ETHEREUM_PATH, chain_id)


_XPUB_MAGIC = 0x0488_B21E


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappPublicKeys(_TestCaseWithSeed):
    def test_declared_path(self):
        schemas = [PathSchema.parse(_ETHEREUM_PATTERN, 60)]
        node = await_result(
            get_keychain("secp256k1", [PathSchema.parse("m/**", 0)])
        ).derive(_ETHEREUM_PATH)
        xpub = await_result(
            _get_xpub("secp256k1", schemas, _ETHEREUM_PATH, _XPUB_MAGIC)
        )
        self.assertEqual(xpub, node.serialize_public(_XPUB_MAGIC))
        public_key = await_result(
            _get_public_key("secp256k1", schemas, _ETHEREUM_PATH, True)
        )
        self.assertEqual(public_key, node.public_key())

    def test_undeclared_path(self):
        schemas = [PathSchema.parse(_TRON_PATTERN, 195)]
        with self.assertRaises(DataError):
            await_result(_get_xpub("secp256k1", schemas, _ETHEREUM_PATH, _XPUB_MAGIC))
        with self.assertRaises(DataError):
            await_result(_get_public_key("secp256k1", schemas, _ETHEREUM_PATH, True))


_WIRE_END = 2  # run.py's _SERVICE_WIRE_END, a const() and so not importable


class _Message:
    def __init__(self, service: int, data: bytes) -> None:
        self.fn = service << 16
        self.data = data


class _Slot:
    """Stands in for `app`, `io` and `loop` in run.py: one running app, which
    answers every request with b"reply". `inbox` holds its messages to Core."""

    Timeout = loop.Timeout

    def __init__(self, inbox: list[_Message]) -> None:
        self.inbox = inbox
        # Read here, not in the class body: only builds with IPC have it.
        self.IPC2_EVENT = io.IPC2_EVENT
        self.POLL_READ = io.POLL_READ

    def image_by_handle(self, handle: int) -> "_Slot":
        return self

    def allowed_curves(self) -> list[str]:
        return ["secp256k1"]

    def allowed_paths(self) -> list[str]:
        return [_ETHEREUM_PATTERN]

    def is_running(self) -> bool:
        return True

    def task_id(self) -> int:
        return 2

    def ipc_send(self, task_id: int, fn: int, data: bytes) -> None:
        self.inbox.append(_Message(_WIRE_END, b"reply"))

    async def wait(self, iface: int, timeout_ms: int) -> _Message:
        if not self.inbox:
            raise loop.Timeout
        return self.inbox.pop(0)


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappRun(unittest.TestCase):
    def tearDown(self):
        get_sessionless_cache().delete(cache_common.APP_EXTAPP_IDS)

    def test_drops_message_sent_before_the_request(self):
        # e.g. left in the slot by an earlier instance
        slot = _Slot([_Message(_WIRE_END, b"stale")])
        instance_id = 1
        get_sessionless_cache().set(
            cache_common.APP_EXTAPP_IDS, ustruct.pack("<II", 0, instance_id)
        )
        request = ExtAppMessage(instance_id=instance_id, message_id=0, data=b"")
        with patch(extapp_run, "app", slot), patch(extapp_run, "io", slot):
            with patch(extapp_run, "loop", slot):
                response = await_result(extapp_run.run(request))
        self.assertEqual(response.data, b"reply")


if __name__ == "__main__":
    unittest.main()
