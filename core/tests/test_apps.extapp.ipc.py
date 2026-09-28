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


def _request_number(initial: int, minimum: int, maximum: int) -> bytes:
    """An archived TrezorUiEnum::RequestNumber (variant 10) titled "t"."""
    return (
        b"tc\x00\x00"  # the title and the content
        + b"\x0a\x00\x00\x00"  # variant tag
        # relative pointers to "t" (-8) and "c" (-15), each of length 1
        + bytes.fromhex("f8ffffff01000000f1ffffff01000000")
        + b"".join(n.to_bytes(4, "little") for n in (initial, minimum, maximum))
        + bytes(88)  # br_code, and padding to the size of the enum
    )


def _show_public_key(pubkey: str) -> bytes:
    """An archived TrezorUiEnum::ShowPublicKey (variant 13) of an ASCII
    `pubkey`, titled "t"."""
    strings = pubkey.encode() + b"tpk"  # the pubkey, the title and br_name
    root = (len(strings) + 3) & ~3

    def string(field: int, offset: int, length: int) -> bytes:
        # A relative pointer from the field at `root + field`, and a length.
        pointer = (offset - root - field) & 0xFFFF_FFFF
        return pointer.to_bytes(4, "little") + length.to_bytes(4, "little")

    return (
        strings
        + bytes(root - len(strings))
        + b"\x0d\x00\x00\x00"  # variant tag
        + string(4, 0, len(pubkey))
        + string(12, len(pubkey), 1)
        + bytes(36)  # no account, path or warning
        + string(56, len(pubkey) + 1, 2)
        + bytes(56)  # br_code, and padding to the size of the enum
    )


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

    def test_request_number_range(self):
        for initial, minimum, maximum in ((5, 1, 10), (1, 1, 1)):
            trezorui_api.process_ipc_message(
                data=_request_number(initial, minimum, maximum)
            )
        for initial, minimum, maximum in ((5, 10, 1), (0, 1, 10), (11, 1, 10)):
            with self.assertRaises(ValueError):
                trezorui_api.process_ipc_message(
                    data=_request_number(initial, minimum, maximum)
                )

    def test_qr_code_capacity(self):
        # A case-sensitive QR code of up to version 9 holds 180 bytes.
        trezorui_api.process_ipc_message(data=_show_public_key("a" * 180))
        with self.assertRaises(ValueError):
            trezorui_api.process_ipc_message(data=_show_public_key("a" * 181))


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappReplies(unittest.TestCase):
    def test_reply_callback_failure_is_raised(self):
        def app_gone(data: bytes) -> None:
            raise RuntimeError

        with self.assertRaises(RuntimeError):
            trezorcrypto_api.send_crypto_result(result=True, ipc_cb=app_gone)
        with self.assertRaises(RuntimeError):
            trezorui_api.send_ui_result(result=trezorui_api.CONFIRMED, ipc_cb=app_gone)


if __name__ == "__main__":
    unittest.main()
