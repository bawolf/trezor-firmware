# flake8: noqa: F403,F405
from common import *  # isort:skip

from storage import cache_common
from trezor.crypto import bip39
from trezor.wire import DataError, context

from apps.common.keychain import get_keychain
from apps.common.paths import PathSchema

if not utils.USE_THP:
    from storage import cache_codec

if utils.USE_APP_LOADING:
    from apps.extapp.run import _get_public_key, _get_xpub

_ETHEREUM_PATTERN = "m/44'/60'/account'/change/address_index/**"
_TRON_PATTERN = "m/44'/195'/account'/change/address_index/**"
_ETHEREUM_PATH = [H_(44), H_(60), H_(0), 0, 0]
_XPUB_MAGIC = 0x0488_B21E


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappPublicKeys(TestCaseWithContext):
    def setUp(self):
        seed = bip39.seed(" ".join(["all"] * 12), "")
        if utils.USE_THP:
            context.cache_set(cache_common.APP_COMMON_SEED, seed)
        else:
            cache_codec.start_session()
            cache_codec.get_active_session().set(cache_common.APP_COMMON_SEED, seed)

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


if __name__ == "__main__":
    unittest.main()
