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
network, the ZIP-32 account and the diversifier index. What is checked here is the part a host
cannot: what the screen says, and that the returned address is the one the
user was shown.
"""

import typing as t

import pytest

from trezorlib import exceptions, messages
from trezorlib.debuglink import DebugSession as Session

from . import zcash_ext
from .common import ADDRESSES, MNEMONIC, address_pieces, no_screens, screen_text
from .generated.messages import MessageType, ZcashGetAddress, ZcashNetwork

B = messages.ButtonRequestType

pytestmark = [pytest.mark.setup_client(mnemonic=MNEMONIC)]


class AddressFlow(t.NamedTuple):
    address: str
    requests: list[tuple[int, str]]
    screen: str
    subtitle: str


def _get_address_reading_the_screen(
    session: Session,
    instance_id: int,
    network: ZcashNetwork = ZcashNetwork.Mainnet,
    account: int = 0,
    index: int = 0,
    chunkify: bool = False,
) -> AddressFlow:
    """Drive ZcashGetAddress, recording the ButtonRequests, the screen and its
    subtitle."""
    shown = []
    subtitles = []
    requests = []

    def accept(session: Session):
        br = yield
        requests.append((br.code, br.name))
        session.debug.press_yes()
        br = yield
        requests.append((br.code, br.name))
        subtitles.append(session.debug.read_layout().subtitle())
        shown.append(screen_text(session))
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        address = zcash_ext.get_address(
            session,
            instance_id,
            network,
            account,
            index.to_bytes(11, "little"),
            chunkify=chunkify,
        )
    return AddressFlow(address, requests, shown[0], subtitles[0])


@pytest.mark.parametrize("network, account, index", sorted(ADDRESSES))
def test_receive_address(
    session: Session, instance_id: int, network: ZcashNetwork, account: int, index: int
) -> None:
    flow = _get_address_reading_the_screen(
        session, instance_id, network, account, index
    )
    assert flow.address == ADDRESSES[network, account, index]
    # The 12-word test wallet gets the ZIP-315 weak-backup warning first. The
    # SDK's address screen has a fixed ButtonRequest name.
    assert flow.requests == [
        (B.Warning, "zcash_weak_backup"),
        (B.Address, "show_address"),
    ]
    assert flow.address.startswith("".join(address_pieces(flow.screen, flow.address)))
    assert flow.subtitle == (
        "Mainnet" if network == ZcashNetwork.Mainnet else "Testnet"
    )


def test_receive_address_chunkify(session: Session, instance_id: int) -> None:
    """`chunkify` groups the 106-character UA on screen, like Bitcoin's.

    It is presentation only: the address the device returns and the
    ButtonRequests it sends are the same either way.
    """
    plain = _get_address_reading_the_screen(session, instance_id, chunkify=False)
    chunked = _get_address_reading_the_screen(session, instance_id, chunkify=True)

    assert plain.address == chunked.address == ADDRESSES[ZcashNetwork.Mainnet, 0, 0]
    assert len(plain.address) == 106
    assert plain.requests == chunked.requests

    # The debug layout reports lines, not the spaces inside them, so what
    # chunking shows up as is width: a full chunked line is a whole number of
    # four-character groups and is narrower than a full unchunked one.
    plain_pieces = address_pieces(plain.screen, plain.address)
    chunked_pieces = address_pieces(chunked.screen, chunked.address)
    assert plain_pieces and chunked_pieces
    assert plain.address.startswith("".join(plain_pieces))
    assert chunked.address.startswith("".join(chunked_pieces))
    widest_chunked = max(len(piece) for piece in chunked_pieces)
    assert widest_chunked % 4 == 0
    assert widest_chunked < max(len(piece) for piece in plain_pieces)


@pytest.mark.parametrize(
    "fields, message",
    [
        pytest.param(
            dict(account=0, diversifier_index=bytes(11)),
            "Malformed Zcash request",
            id="no-network",
        ),
        pytest.param(
            dict(network=ZcashNetwork.Mainnet, diversifier_index=bytes(11)),
            "Malformed Zcash request",
            id="no-account",
        ),
        pytest.param(
            dict(network=ZcashNetwork.Mainnet, account=0),
            "Malformed Zcash request",
            id="no-index",
        ),
        pytest.param(
            dict(network=ZcashNetwork.Mainnet, account=0, diversifier_index=bytes(10)),
            "Malformed Zcash request",
            id="short-index",
        ),
        pytest.param(
            dict(
                network=ZcashNetwork.Mainnet, account=2**31, diversifier_index=bytes(11)
            ),
            "Zcash request violates device policy",
            id="account-2^31",
        ),
    ],
)
def test_refuses_bad_requests_before_any_screen(
    session: Session, instance_id: int, fields: dict, message: str
) -> None:
    with session.test_ctx as client:
        client.set_input_flow(no_screens(session))
        with pytest.raises(exceptions.TrezorFailure, match=message):
            zcash_ext.call_ext(
                session,
                instance_id,
                ZcashGetAddress(**fields),
                zcash_ext.zcash_messages.ZcashAddress,
            )


def test_refuses_an_unknown_network_before_any_screen(
    session: Session, instance_id: int
) -> None:
    # network = 2, account = 0, diversifier_index = 11 zero bytes; trezorlib
    # refuses to encode the unknown enum value itself.
    data = b"\x08\x02" + b"\x10\x00" + b"\x1a\x0b" + bytes(11)
    with session.test_ctx as client:
        client.set_input_flow(no_screens(session))
        with pytest.raises(
            exceptions.TrezorFailure, match="Zcash request violates device policy"
        ):
            zcash_ext.call_raw(
                session,
                instance_id,
                MessageType.ZcashGetAddress,
                data,
                zcash_ext.zcash_messages.ZcashAddress,
            )
