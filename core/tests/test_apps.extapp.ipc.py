# flake8: noqa: F403,F405
from common import *  # isort:skip

if utils.USE_APP_LOADING:
    import trezorui_api


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


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappUiRequests(unittest.TestCase):
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


if __name__ == "__main__":
    unittest.main()
