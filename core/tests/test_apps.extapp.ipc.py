# flake8: noqa: F403,F405
from common import *  # isort:skip

if utils.USE_APP_LOADING:
    import trezorui_api


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
class TestExtappUiRequests(unittest.TestCase):
    def test_qr_code_capacity(self):
        # A case-sensitive QR code of up to version 9 holds 180 bytes.
        trezorui_api.process_ipc_message(data=_show_public_key("a" * 180))
        with self.assertRaises(ValueError):
            trezorui_api.process_ipc_message(data=_show_public_key("a" * 181))


if __name__ == "__main__":
    unittest.main()
