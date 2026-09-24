# This file is part of the Trezor project.
#
# Copyright (C) SatoshiLabs and contributors
#
# This library is free software: you can redistribute it and/or modify
# it under the terms of the GNU Lesser General Public License version 3
# as published by the Free Software Foundation.
#
# This library is distributed in the hope that it will be useful,
# but WITHOUT ANY WARRANTY; without even the implied warranty of
# MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
# GNU Lesser General Public License for more details.
#
# You should have received a copy of the License along with this library.
# If not, see <https://www.gnu.org/licenses/lgpl-3.0.html>.

"""ZcashGetAddress on the emulator.

The device derives its own Orchard receiver; the host selects only the
network and the ZIP-32 account. What is checked here is the part a host
cannot: what the screen says, and that the returned address is the one the
user was shown.
"""

import pytest

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session

from .common import MNEMONIC, address_pieces, screen_text

B = messages.ButtonRequestType

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.zcash,
    pytest.mark.capabilities(messages.Capability.Zcash_Shielded),
    pytest.mark.models("t3b1", "t3t1", "t3w1"),
    pytest.mark.setup_client(mnemonic=MNEMONIC),
]


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
        shown.append(screen_text(session))
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

    # The 12-word test wallet gets the ZIP-315 weak-backup warning first.
    expected_brs = [
        (B.Warning, "zcash_weak_backup"),
        (B.Address, "zcash_receive"),
    ]
    assert plain_brs == expected_brs
    assert chunked_brs == expected_brs

    # Both screens carry the address, in order, from the start, and nothing
    # else that looks like it. The debug layout reports lines, not the spaces
    # inside them, so what chunking shows up as is width: a full chunked line
    # is a whole number of four-character groups and is narrower than a full
    # unchunked one. Paying width is why the chunked first page holds a prefix
    # rather than all 106 characters -- the trade the host opts into.
    plain_pieces = address_pieces(plain_screen, plain_addr)
    chunked_pieces = address_pieces(chunked_screen, chunked_addr)
    assert plain_pieces and chunked_pieces
    assert plain_addr.startswith("".join(plain_pieces))
    assert chunked_addr.startswith("".join(chunked_pieces))
    widest_chunked = max(len(piece) for piece in chunked_pieces)
    assert widest_chunked % 4 == 0
    assert widest_chunked < max(len(piece) for piece in plain_pieces)
    assert chunked_screen != plain_screen
