# flake8: noqa: F403,F405
from common import *  # isort:skip

if utils.USE_APP_LOADING:
    import trezorcrypto_api
    import trezorui_api

# An archived TrezorCryptoEnum::GetPublicKey (operation 1) of m/44'/60'/0'.
GET_PUBLIC_KEY = (
    bytes.fromhex("2c0000803c00008000000080")  # the path
    + bytes(4)
    # variant tag, relative pointer to the path and its length, `compressed`
    + bytes.fromhex("01000000ecffffff0300000001")
    + bytes(83)
)
# An archived TrezorProgressEnum::End (operation 2).
PROGRESS_END = b"\x02" + bytes(31)
MALFORMED = (b"", b"\xff" * 32)


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappRequests(unittest.TestCase):
    def test_crypto_request(self):
        # Copied into a heap object, as run.py does with IPC data.
        data = bytes(bytearray(GET_PUBLIC_KEY))
        address_n, compressed = trezorcrypto_api.deserialize_crypto_message(
            data=data, message_id=1
        )
        self.assertEqual(list(address_n), [H_(44), H_(60), H_(0)])
        self.assertTrue(compressed)
        with self.assertRaises(ValueError):
            trezorcrypto_api.deserialize_crypto_message(data=data, message_id=0)
        for data in MALFORMED:
            with self.assertRaises(ValueError):
                trezorcrypto_api.deserialize_crypto_message(data=data, message_id=1)

    def test_progress_request(self):
        data = bytes(bytearray(PROGRESS_END))
        self.assertIsNone(
            trezorui_api.deserialize_progress_message(data=data, message_id=2)
        )
        for message_id in (0, 1):
            with self.assertRaises(ValueError):
                trezorui_api.deserialize_progress_message(
                    data=data, message_id=message_id
                )
        for data in MALFORMED:
            with self.assertRaises(ValueError):
                trezorui_api.deserialize_progress_message(data=data, message_id=2)

    def test_ui_request(self):
        for data in MALFORMED:
            with self.assertRaises(ValueError):
                trezorui_api.process_ipc_message(data=data)


if __name__ == "__main__":
    unittest.main()
