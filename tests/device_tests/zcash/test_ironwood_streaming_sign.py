"""Zcash receive flows on the emulator (T3B1 / T3T1 / T3W1).

The device derives its own Orchard receiver and full viewing key; the host
selects only the network and the ZIP-32 account. What the tests here check is
the part a host cannot: what the screen says, and that the returned address is
the one the user was shown.
"""

import pytest

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session
from trezorlib.debuglink import LayoutType

B = messages.ButtonRequestType

MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.zcash,
    pytest.mark.ironwood,
    pytest.mark.models("t3b1", "t3t1", "t3w1"),
    pytest.mark.setup_client(mnemonic=MNEMONIC),
]


def _address_screen_text(session: Session) -> str:
    """The receive-address screen, paging where the model needs it."""
    debug = session.debug
    layout = debug.read_layout()
    shown = layout.text_content()
    if debug.layout_type is LayoutType.Caesar:
        for _ in range(layout.page_count() - 1):
            debug.press_right()
            shown += " " + debug.read_layout().text_content()
    return shown


def _address_pieces(screen: str, address: str) -> list[str]:
    """The runs of address on screen, dropping labels like "Mainnet"."""
    return [word for word in screen.split() if word and word in address]


def _get_address_reading_the_screen(session: Session, chunkify: bool):
    """Drive ZcashGetAddress, recording the ButtonRequests and the screen."""
    shown = []
    requests = []

    def accept(session: Session):
        br = yield
        requests.append((br.code, br.name))
        session.debug.press_yes()
        br = yield
        requests.append((br.code, br.name))
        shown.append(_address_screen_text(session))
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        address = zcash.get_address(
            session, messages.ZcashNetwork.Mainnet, 0, bytes(11), chunkify=chunkify
        )
    return address, requests, shown[0]


def test_receive_address_chunkify(session: Session) -> None:
    """`chunkify` groups the 106-character UA on screen, like Bitcoin's.

    It is presentation only: the address the device returns and the
    ButtonRequests it sends are the same either way, so it changes nothing a
    host verifies. Off by default, so a host that has never heard of the field
    gets exactly the screen it got before.
    """
    plain_addr, plain_brs, plain_screen = _get_address_reading_the_screen(
        session, chunkify=False
    )
    chunked_addr, chunked_brs, chunked_screen = _get_address_reading_the_screen(
        session, chunkify=True
    )

    assert plain_addr == chunked_addr
    assert plain_addr.startswith("u1")
    assert len(plain_addr) == 106

    expected_brs = [
        (B.Warning, "ironwood_weak_backup"),
        (B.Address, "ironwood_receive"),
    ]
    assert plain_brs == expected_brs
    assert chunked_brs == expected_brs

    # Both screens carry the address, in order, from the start, and nothing
    # else that looks like it. The debug layout reports lines, not the spaces
    # inside them, so what chunking shows up as is width: a full chunked line
    # is a whole number of four-character groups and is narrower than a full
    # unchunked one. Paying width is why the chunked first page holds a prefix
    # rather than all 106 characters -- the trade the host opts into.
    plain_pieces = _address_pieces(plain_screen, plain_addr)
    chunked_pieces = _address_pieces(chunked_screen, chunked_addr)
    assert plain_pieces and chunked_pieces
    assert plain_addr.startswith("".join(plain_pieces))
    assert chunked_addr.startswith("".join(chunked_pieces))
    widest_chunked = max(len(piece) for piece in chunked_pieces)
    assert widest_chunked % 4 == 0
    assert widest_chunked < max(len(piece) for piece in plain_pieces)
    assert chunked_screen != plain_screen
