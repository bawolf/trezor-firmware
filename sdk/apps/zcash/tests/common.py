# This file is part of the Trezor project.
#
# Copyright (C) 2012-2019 SatoshiLabs and contributors
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

from __future__ import annotations

import json
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from trezorlib import messages, models, protobuf
from trezorlib.exceptions import TrezorFailure
from trezorlib.tools import H_

from . import zcash_ext
from .zcash_ext import Network

if TYPE_CHECKING:
    from _pytest.mark.structures import MarkDecorator

    from trezorlib.debuglink import DebugSession as Session


HERE = Path(__file__).resolve().parent
COMMON_FIXTURES_DIR = HERE / "fixtures"


def parametrize_using_common_fixtures(*paths: str) -> "MarkDecorator":
    fixtures = []
    for path in paths:
        fixtures.append(json.loads((COMMON_FIXTURES_DIR / path).read_text()))

    tests = []
    for fixture in fixtures:
        for test in fixture["tests"]:
            test_id = test.get("name")
            if not test_id:
                test_id = test.get("description")
                if test_id is not None:
                    test_id = test_id.lower().replace(" ", "_")

            skip_models = test.get("skip_models", [])
            skiplist = []
            # TODO: genericify this
            for skip_model in skip_models:
                if skip_model == "t3t1":
                    skiplist.append(models.T3T1)
                if skip_model == "t3w1":
                    skiplist.append(models.T3W1)
            if skiplist:
                extra_marks = [pytest.mark.models(skip=skiplist)]
            else:
                extra_marks = []

            if test.get("experimental"):
                extra_marks.append(pytest.mark.experimental)

            tests.append(
                pytest.param(
                    test["parameters"],
                    test["result"],
                    marks=[
                        pytest.mark.setup_client(
                            passphrase=fixture["setup"]["passphrase"],
                            mnemonic=fixture["setup"]["mnemonic"],
                        )
                    ]
                    + extra_marks,
                    id=test_id,
                )
            )

    return pytest.mark.parametrize("parameters, result", tests)


# Paths other than m/32'/coin_type'/account' with coin type 133 or 1 and
# account 0..100.
INVALID_PATHS = [
    pytest.param([H_(32), H_(133)], id="short"),
    pytest.param([H_(32), H_(133), H_(0), 0], id="long"),
    pytest.param([H_(44), H_(133), H_(0)], id="purpose_44"),
    pytest.param([H_(32), H_(60), H_(0)], id="coin_type_60"),
    pytest.param([H_(32), 133, H_(0)], id="coin_type_not_hardened"),
    pytest.param([H_(32), H_(133), 0], id="account_not_hardened"),
    pytest.param([H_(32), H_(133), H_(101)], id="account_101"),
]


def parse_network(name: str) -> Network:
    """A fixture's network: "mainnet" or "testnet"."""
    return Network[name.capitalize()]


def assert_refused_before_any_screen(
    session: Session,
    instance_id: int,
    msg: protobuf.MessageType,
    match: str,
) -> None:
    with session.test_ctx as client:
        client.set_expected_responses(
            [messages.Failure(code=messages.FailureType.DataError)]
        )
        with pytest.raises(TrezorFailure, match=match):
            zcash_ext.call_raw(session, instance_id, msg)
