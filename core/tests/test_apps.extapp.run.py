# flake8: noqa: F403,F405
from common import *  # isort:skip

from storage import cache_common
from trezor.crypto import bip39
from trezor.crypto.curve import secp256k1
from trezor.wire import DataError, context

from apps.common.keychain import get_keychain
from apps.common.paths import PathSchema

if not utils.USE_THP:
    from storage import cache_codec

if utils.USE_APP_LOADING:
    from apps.extapp.run import _path_schemas

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


@unittest.skipUnless(utils.USE_APP_LOADING and not utils.BITCOIN_ONLY, "app loading")
class TestExtappSignTypedHash(TestCaseWithContext):
    def setUp(self):
        seed = bip39.seed(" ".join(["all"] * 12), "")
        if utils.USE_THP:
            context.cache_set(cache_common.APP_COMMON_SEED, seed)
        else:
            cache_codec.start_session()
            cache_codec.get_active_session().set(cache_common.APP_COMMON_SEED, seed)

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


if __name__ == "__main__":
    unittest.main()
