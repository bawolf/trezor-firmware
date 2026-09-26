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

from . import zcash_ext
from .common import (
    INVALID_PATHS,
    assert_refused_before_any_screen,
    parametrize_using_common_fixtures,
    parse_network,
)
from .generated import messages as zcash_messages
from .input_flows import InputFlowExportViewingKey
from .zcash_ext import COIN_TYPES


@parametrize_using_common_fixtures("get_viewing_key.json")
def test_get_viewing_key(session: Session, instance_id: int, parameters, result):
    network = parse_network(parameters["network"])
    account = parameters["account"]
    include_seed_fingerprint = parameters["include_seed_fingerprint"]
    with session.test_ctx as client:
        IF = InputFlowExportViewingKey(
            client, network, account, include_seed_fingerprint
        )
        client.set_input_flow(IF.get())
        viewing_key = zcash_ext.export_viewing_key(
            session, instance_id, network, account, include_seed_fingerprint
        )

    assert viewing_key.key == result["key"]
    if result["seed_fingerprint"] is None:
        assert viewing_key.seed_fingerprint is None
    else:
        assert viewing_key.seed_fingerprint == bytes.fromhex(result["seed_fingerprint"])
        assert "recovery seed" in IF.fingerprint_screen
    assert f"m/32'/{COIN_TYPES[network]}'/{account}'" in IF.export_screen


@pytest.mark.parametrize("address_n", INVALID_PATHS)
def test_invalid_path(session: Session, instance_id: int, address_n: list[int]):
    msg = zcash_messages.GetViewingKey(address_n=address_n)
    assert_refused_before_any_screen(session, instance_id, msg, "Forbidden key path")
