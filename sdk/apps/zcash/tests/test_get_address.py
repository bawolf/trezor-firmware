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

import pytest

from trezorlib.debuglink import DebugSession as Session
from trezorlib.debuglink import LayoutType
from trezorlib.exceptions import Cancelled

from . import zcash_ext
from .common import (
    INVALID_PATHS,
    assert_refused_before_any_screen,
    parametrize_using_common_fixtures,
    parse_network,
)
from .generated import messages as zcash_messages
from .input_flows import (
    InputFlowDeclineAccount,
    InputFlowShowAddress,
    is_chunked,
    is_shown_whole,
)
from .zcash_ext import DIVERSIFIER_INDEX_BYTES, Network


def get_address(
    session: Session,
    instance_id: int,
    network: Network = Network.Mainnet,
    account: int = 0,
    diversifier_index: int = 0,
    chunkify: bool = False,
    first_use: bool = True,
) -> tuple[str, InputFlowShowAddress]:
    with session.test_ctx as client:
        IF = InputFlowShowAddress(client, network, account, first_use)
        client.set_input_flow(IF.get())
        address = zcash_ext.get_address(
            session,
            instance_id,
            network,
            account,
            diversifier_index.to_bytes(DIVERSIFIER_INDEX_BYTES, "little"),
            chunkify=chunkify,
        )
    return address, IF


@parametrize_using_common_fixtures("get_address.json", "get_address.other_seed.json")
def test_get_address(session: Session, instance_id: int, parameters, result):
    network = parse_network(parameters["network"])
    address, IF = get_address(
        session,
        instance_id,
        network,
        parameters["account"],
        parameters["diversifier_index"],
    )
    assert address == result["address"]
    assert is_shown_whole(IF.screen, address)
    assert not is_chunked(IF.screen, address)
    # Eckhart shows the network as the subtitle; Delizia has no subtitle.
    if session.debug.layout_type is LayoutType.Eckhart:
        assert IF.subtitle == network.name


@parametrize_using_common_fixtures("get_address.json")
def test_get_address_chunkify(session: Session, instance_id: int, parameters, result):
    address, IF = get_address(
        session,
        instance_id,
        parse_network(parameters["network"]),
        parameters["account"],
        parameters["diversifier_index"],
        chunkify=True,
    )
    assert address == result["address"]
    assert is_shown_whole(IF.screen, address)
    assert is_chunked(IF.screen, address)


def test_account_request_asked_once(session: Session, instance_id: int):
    """Core asks once per network and account of an app instance."""
    for network, account, first_use in [
        (Network.Mainnet, 0, True),
        (Network.Mainnet, 0, False),
        (Network.Mainnet, 1, True),
        (Network.Testnet, 0, True),
        (Network.Mainnet, 1, False),
    ]:
        get_address(session, instance_id, network, account, first_use=first_use)


def test_decline_account_request(session: Session, instance_id: int):
    with session.test_ctx as client:
        client.set_input_flow(InputFlowDeclineAccount(client).get())
        with pytest.raises(Cancelled):
            zcash_ext.get_address(
                session, instance_id, Network.Mainnet, 0, bytes(DIVERSIFIER_INDEX_BYTES)
            )

    # Declining is not remembered: the next request asks again.
    get_address(session, instance_id)


@pytest.mark.parametrize("address_n", INVALID_PATHS)
def test_invalid_path(session: Session, instance_id: int, address_n: list[int]):
    msg = zcash_messages.GetAddress(
        address_n=address_n, diversifier_index=bytes(DIVERSIFIER_INDEX_BYTES)
    )
    assert_refused_before_any_screen(session, instance_id, msg, "Forbidden key path")


def test_invalid_diversifier_index(session: Session, instance_id: int):
    msg = zcash_messages.GetAddress(
        address_n=zcash_ext.address_n(Network.Mainnet, 0),
        diversifier_index=bytes(DIVERSIFIER_INDEX_BYTES - 1),
    )
    assert_refused_before_any_screen(
        session, instance_id, msg, "Invalid diversifier index"
    )
