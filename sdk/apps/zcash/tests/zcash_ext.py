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

"""Host side of the Zcash app's messages over ExtAppMessage."""

from __future__ import annotations

import io
import typing as t
from enum import IntEnum

from trezorlib import exceptions, protobuf
from trezorlib.messages import ExtAppMessage, ExtAppResponse, Failure
from trezorlib.tools import H_

from .generated import messages as zcash_messages

if t.TYPE_CHECKING:
    from trezorlib.client import Session

MT = t.TypeVar("MT", bound=protobuf.MessageType)

DIVERSIFIER_INDEX_BYTES = 11


class Network(IntEnum):
    Mainnet = 0
    Testnet = 1


COIN_TYPES = {Network.Mainnet: 133, Network.Testnet: 1}


def address_n(network: Network, account: int) -> list[int]:
    """The ZIP-32 account path m/32'/coin_type'/account'."""
    return [H_(32), H_(COIN_TYPES[network]), H_(account)]


def message_id(msg: type[protobuf.MessageType] | protobuf.MessageType) -> int:
    """The app's message ID of a message class or instance."""
    cls = msg if isinstance(msg, type) else type(msg)
    return zcash_messages.MessageType[cls.__name__]


def call_raw(
    session: Session, instance_id: int, msg: protobuf.MessageType
) -> ExtAppResponse:
    """Send one of the app's messages; a Failure raises."""
    buf = io.BytesIO()
    protobuf.dump_message(buf, msg)
    return session.call(
        ExtAppMessage(
            instance_id=instance_id, message_id=message_id(msg), data=buf.getvalue()
        ),
        expect=ExtAppResponse,
    )


def call_ext(
    session: Session,
    instance_id: int,
    *,
    msg: protobuf.MessageType,
    expect: type[MT],
    finished: bool = True,
) -> MT:
    """Send one of the app's messages and return its response of type `expect`,
    which ends the app's request unless `finished` is False."""
    resp = call_raw(session, instance_id, msg)
    if resp.message_id != message_id(expect):
        raise exceptions.TrezorFailure(
            failure=Failure(message="Unexpected response type")
        )
    assert resp.finished == finished
    return protobuf.load_message(io.BytesIO(resp.data), expect)


def get_address(
    session: Session,
    instance_id: int,
    network: Network,
    account: int,
    diversifier_index: bytes,
    chunkify: bool = False,
) -> str:
    return call_ext(
        session,
        instance_id,
        msg=zcash_messages.GetAddress(
            address_n=address_n(network, account),
            diversifier_index=diversifier_index,
            chunkify=chunkify,
        ),
        expect=zcash_messages.Address,
    ).address


def export_viewing_key(
    session: Session,
    instance_id: int,
    network: Network,
    account: int,
    include_seed_fingerprint: bool = False,
) -> zcash_messages.ViewingKey:
    return call_ext(
        session,
        instance_id,
        msg=zcash_messages.GetViewingKey(
            address_n=address_n(network, account),
            include_seed_fingerprint=include_seed_fingerprint,
        ),
        expect=zcash_messages.ViewingKey,
    )


def sign_pczt(
    session: Session,
    instance_id: int,
    pczt: bytes,
    network: Network,
    account: int,
    host_reference_height: int,
    on_diagnostics: t.Callable[[zcash_messages.Diagnostics], None] | None = None,
) -> list[zcash_messages.SpendAuthSignature]:
    """Send the PCZT in the chunks the device requests, and return its spend
    authorization signatures, one per real spend in ascending action order.

    A debug build of the app sends its counters with every chunk request, to
    `on_diagnostics`."""
    request = call_ext(
        session,
        instance_id,
        msg=zcash_messages.SignPczt(
            address_n=address_n(network, account),
            pczt_length=len(pczt),
            host_reference_height=host_reference_height,
        ),
        expect=zcash_messages.PcztRequest,
        finished=False,
    )
    transfer_id = request.transfer_id
    while True:
        if on_diagnostics is not None and request.diagnostics is not None:
            on_diagnostics(request.diagnostics)
        end = request.offset + request.length
        ack = zcash_messages.PcztAck(
            transfer_id=transfer_id,
            offset=request.offset,
            data=pczt[request.offset : end],
        )
        if end == len(pczt):
            break
        request = call_ext(
            session,
            instance_id,
            msg=ack,
            expect=zcash_messages.PcztRequest,
            finished=False,
        )
        assert (request.transfer_id, request.offset) == (transfer_id, end)

    response = call_ext(
        session, instance_id, msg=ack, expect=zcash_messages.SpendAuthSignatures
    )
    assert response.transfer_id == transfer_id
    return response.signatures


def get_diagnostics(session: Session, instance_id: int) -> zcash_messages.Diagnostics:
    """The app's heap and IPC counters since the previous call, which starts
    new ones. Debug builds of the app only."""
    return call_ext(
        session,
        instance_id,
        msg=zcash_messages.GetDiagnostics(),
        expect=zcash_messages.Diagnostics,
    )
