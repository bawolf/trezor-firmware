# flake8: noqa: F403,F405
from common import *  # isort:skip

from trezor.wire import DataError

if utils.USE_APP_LOADING:
    from apps.extapp.run import _path_schemas


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


if __name__ == "__main__":
    unittest.main()
