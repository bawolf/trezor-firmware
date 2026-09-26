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

import io
import json

import pytest

from trezorlib import messages, protobuf
from trezorlib.debuglink import DebugSession as Session
from trezorlib.exceptions import Cancelled, TrezorFailure

from . import zcash_ext
from .common import (
    COMMON_FIXTURES_DIR,
    INVALID_PATHS,
    assert_refused_before_any_screen,
    parametrize_using_common_fixtures,
    parse_network,
)
from .generated import messages as zcash_messages
from .input_flows import InputFlowSignPczt
from .zcash_ext import DIVERSIFIER_INDEX_BYTES, Network

MAX_PCZT_BYTES = 65_536
TRANSFER_ID_BYTES = 16
CHUNK_BYTES = 1024
SIGNATURE_BYTES = 64
# The manifest's `ipc-buffer-size`.
APP_INBOX_BYTES = 2048
HEIGHT = 10_000_000


def vector(name: str) -> tuple[dict, dict]:
    fixture = json.loads((COMMON_FIXTURES_DIR / "sign_pczt.json").read_text())
    test = next(test for test in fixture["tests"] if test["name"] == name)
    return test["parameters"], test["result"]


def sign_pczt(
    session: Session, instance_id: int, parameters: dict
) -> list[zcash_messages.SpendAuthSignature]:
    return zcash_ext.sign_pczt(
        session,
        instance_id,
        bytes.fromhex(parameters["pczt"]),
        parse_network(parameters["network"]),
        parameters["account"],
        parameters["host_reference_height"],
    )


@parametrize_using_common_fixtures(
    "sign_pczt.json",
    "sign_pczt.memos.json",
    "sign_pczt.transparent.json",
    "sign_pczt.user_address.json",
)
def test_sign_pczt(session: Session, instance_id: int, parameters, result):
    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result)
        client.set_input_flow(IF.get())
        signatures = sign_pczt(session, instance_id, parameters)

    assert [s.action_index for s in signatures] == result["real_spend_actions"]
    assert all(len(s.signature) == SIGNATURE_BYTES for s in signatures)
    assert sorted(IF.addresses) == sorted(p["address"] for p in result["payments"])


@parametrize_using_common_fixtures("sign_pczt_error.json")
def test_sign_pczt_error(session: Session, instance_id: int, parameters, result):
    with session.test_ctx as client:
        client.set_input_flow(InputFlowSignPczt(client, parameters, result).get())
        with pytest.raises(TrezorFailure, match=result["error"]):
            sign_pczt(session, instance_id, parameters)


def test_sign_pczt_cancel_at_output(session: Session, instance_id: int):
    parameters, result = vector("8_actions")
    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result, cancel="output")
        client.set_input_flow(IF.get())
        with pytest.raises(Cancelled):
            sign_pczt(session, instance_id, parameters)


def test_sign_pczt_cancel_at_total(session: Session, instance_id: int):
    parameters, result = vector("2_actions")
    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result, cancel="total")
        client.set_input_flow(IF.get())
        with pytest.raises(Cancelled):
            sign_pczt(session, instance_id, parameters)

    # Nothing is left pending, and the account is not asked for again.
    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result, first_use=False)
        client.set_input_flow(IF.get())
        signatures = sign_pczt(session, instance_id, parameters)
    assert [s.action_index for s in signatures] == result["real_spend_actions"]


def test_sign_pczt_twice(session: Session, instance_id: int):
    parameters, result = vector("8_actions")
    for first_use in (True, False):
        with session.test_ctx as client:
            IF = InputFlowSignPczt(client, parameters, result, first_use)
            client.set_input_flow(IF.get())
            signatures = sign_pczt(session, instance_id, parameters)
        assert [s.action_index for s in signatures] == result["real_spend_actions"]


def start_sign_pczt(
    session: Session, instance_id: int, parameters: dict
) -> zcash_messages.PcztRequest:
    """Send SignPczt; return the device's request for the first chunk."""
    msg = zcash_messages.SignPczt(
        address_n=zcash_ext.address_n(
            parse_network(parameters["network"]), parameters["account"]
        ),
        pczt_length=len(bytes.fromhex(parameters["pczt"])),
        host_reference_height=parameters["host_reference_height"],
    )
    resp = zcash_ext.call_raw(session, instance_id, msg)
    assert resp.message_id == zcash_ext.message_id(zcash_messages.PcztRequest)
    assert not resp.finished
    return protobuf.load_message(io.BytesIO(resp.data), zcash_messages.PcztRequest)


@pytest.mark.parametrize(
    "field, value",
    [
        pytest.param("offset", 1, id="offset"),
        pytest.param("transfer_id", bytes(TRANSFER_ID_BYTES), id="zero_transfer_id"),
        pytest.param("data", b"short", id="short_data"),
        pytest.param("data", None, id="no_data"),
    ],
)
def test_sign_pczt_wrong_chunk(
    session: Session, instance_id: int, field: str, value: object
):
    parameters, result = vector("2_actions")
    pczt = bytes.fromhex(parameters["pczt"])
    with session.test_ctx as client:
        client.set_input_flow(InputFlowSignPczt(client, parameters, result).get())
        request = start_sign_pczt(session, instance_id, parameters)
        ack = zcash_messages.PcztAck(
            transfer_id=request.transfer_id,
            offset=request.offset,
            data=pczt[: request.length],
        )
        setattr(ack, field, value)
        with pytest.raises(TrezorFailure, match="Invalid PCZT chunk"):
            zcash_ext.call_raw(session, instance_id, ack)


def test_sign_pczt_chunk_over_app_inbox(session: Session, instance_id: int):
    """Core cannot pass a chunk larger than the app's inbox, and stops the app
    rather than leave it waiting."""
    parameters, result = vector("2_actions")
    with session.test_ctx as client:
        client.set_input_flow(InputFlowSignPczt(client, parameters, result).get())
        request = start_sign_pczt(session, instance_id, parameters)
        ack = zcash_messages.PcztAck(
            transfer_id=request.transfer_id,
            offset=request.offset,
            data=bytes(2 * APP_INBOX_BYTES),
        )
        with pytest.raises(TrezorFailure, match="Failed to send IPC message"):
            zcash_ext.call_raw(session, instance_id, ack)

    with pytest.raises(TrezorFailure, match="Task not running"):
        zcash_ext.get_address(
            session, instance_id, Network.Mainnet, 0, bytes(DIVERSIFIER_INDEX_BYTES)
        )


def test_sign_pczt_host_cancel(session: Session, instance_id: int):
    """A Cancel in place of a chunk ends the signing."""
    parameters, result = vector("2_actions")
    with session.test_ctx as client:
        client.set_input_flow(InputFlowSignPczt(client, parameters, result).get())
        start_sign_pczt(session, instance_id, parameters)
        with pytest.raises(Cancelled):
            zcash_ext.call_raw(session, instance_id, zcash_messages.Cancel())

    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result, first_use=False)
        client.set_input_flow(IF.get())
        signatures = sign_pczt(session, instance_id, parameters)
    assert [s.action_index for s in signatures] == result["real_spend_actions"]


def cancel_after_first_chunk_request(
    session: Session, instance_id: int, parameters: dict, result: dict
) -> None:
    """Start a signing and abandon it with Trezor's Cancel."""
    with session.test_ctx as client:
        client.set_input_flow(InputFlowSignPczt(client, parameters, result).get())
        start_sign_pczt(session, instance_id, parameters)
        with pytest.raises(Cancelled):
            session.call(messages.Cancel())


def test_sign_pczt_trezor_cancel(session: Session, instance_id: int):
    """Trezor's Cancel in place of a chunk abandons the signing without
    stopping the app."""
    parameters, result = vector("2_actions")
    cancel_after_first_chunk_request(session, instance_id, parameters, result)

    # Refused before any screen, whether or not it is answered with the
    # abandoned request's failure (see the next test).
    empty = zcash_messages.SignPczt(
        address_n=zcash_ext.address_n(Network.Mainnet, 0),
        pczt_length=0,
        host_reference_height=HEIGHT,
    )
    with session.test_ctx as client:
        client.set_expected_responses([messages.Failure()])
        with pytest.raises(TrezorFailure):
            zcash_ext.call_raw(session, instance_id, empty)

    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result, first_use=False)
        client.set_input_flow(IF.get())
        signatures = sign_pczt(session, instance_id, parameters)
    assert [s.action_index for s in signatures] == result["real_spend_actions"]


@pytest.mark.xfail(
    raises=TrezorFailure,
    strict=True,
    reason="the app answers the request after an abandoned one with the "
    "abandoned request's failure; fixing it needs request ids on the IPC wire "
    "or an abort message to the app",
)
def test_sign_pczt_after_trezor_cancel(session: Session, instance_id: int):
    """The signing after an abandoned one should succeed. Any failure counts as
    the known one here; test_sign_pczt_trezor_cancel checks that the app keeps
    running."""
    parameters, result = vector("2_actions")
    cancel_after_first_chunk_request(session, instance_id, parameters, result)

    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result, first_use=False)
        client.set_input_flow(IF.get())
        sign_pczt(session, instance_id, parameters)


def test_sign_pczt_chunk_for_another_instance(session: Session, instance_id: int):
    """Core refuses a chunk sent to another app instance, and stops the app
    rather than leave it waiting."""
    parameters, result = vector("2_actions")
    with session.test_ctx as client:
        client.set_input_flow(InputFlowSignPczt(client, parameters, result).get())
        request = start_sign_pczt(session, instance_id, parameters)
        ack = zcash_messages.PcztAck(
            transfer_id=request.transfer_id,
            offset=request.offset,
            data=bytes.fromhex(parameters["pczt"])[: request.length],
        )
        with pytest.raises(TrezorFailure, match="Invalid instance ID"):
            zcash_ext.call_raw(session, (instance_id + 1) % 2**32, ack)

    with pytest.raises(TrezorFailure, match="Task not running"):
        zcash_ext.get_address(
            session, instance_id, Network.Mainnet, 0, bytes(DIVERSIFIER_INDEX_BYTES)
        )


@pytest.mark.parametrize("address_n", INVALID_PATHS)
def test_sign_pczt_invalid_path(
    session: Session, instance_id: int, address_n: list[int]
):
    msg = zcash_messages.SignPczt(
        address_n=address_n, pczt_length=100, host_reference_height=HEIGHT
    )
    assert_refused_before_any_screen(session, instance_id, msg, "Forbidden key path")


@pytest.mark.parametrize(
    "pczt_length, height, error",
    [
        pytest.param(0, HEIGHT, "Invalid PCZT length", id="empty"),
        pytest.param(
            MAX_PCZT_BYTES + 1, HEIGHT, "Invalid PCZT length", id="over_max_length"
        ),
        pytest.param(100, 0, "Zcash PCZT rejected", id="height_before_nu6_3"),
    ],
)
def test_sign_pczt_invalid_request(
    session: Session, instance_id: int, pczt_length: int, height: int, error: str
):
    msg = zcash_messages.SignPczt(
        address_n=zcash_ext.address_n(Network.Mainnet, 0),
        pczt_length=pczt_length,
        host_reference_height=height,
    )
    assert_refused_before_any_screen(session, instance_id, msg, error)


def test_diagnostics(session: Session, instance_id: int):
    """A debug build reports its heap peak and its longest IPC silence, which
    Core limits to 1 s; each request starts new counters."""
    parameters, result = vector("2_actions")
    seen = []
    with session.test_ctx as client:
        IF = InputFlowSignPczt(client, parameters, result)
        client.set_input_flow(IF.get())
        zcash_ext.sign_pczt(
            session,
            instance_id,
            bytes.fromhex(parameters["pczt"]),
            parse_network(parameters["network"]),
            parameters["account"],
            parameters["host_reference_height"],
            seen.append,
        )

    pczt_length = len(bytes.fromhex(parameters["pczt"]))
    assert len(seen) == -(-pczt_length // CHUNK_BYTES)
    for earlier, later in zip(seen, seen[1:]):
        assert earlier.ipc_sent < later.ipc_sent
        assert earlier.heap_peak <= later.heap_peak

    counters = zcash_ext.get_diagnostics(session, instance_id)
    assert 0 < counters.heap_used < counters.heap_peak <= counters.heap_size
    assert counters.max_ipc_silence_ms < 1000
    assert counters.heap_peak >= seen[-1].heap_peak
    again = zcash_ext.get_diagnostics(session, instance_id)
    # Since the previous request the app sent only its response.
    assert again.ipc_sent == 1
    assert again.heap_peak < counters.heap_peak
