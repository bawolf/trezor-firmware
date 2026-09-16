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

"""Zcash Ironwood host API.

The wire identifiers backing these calls are PROVISIONAL and local-only; they
are not assigned or reserved upstream, and they may change once maintainers
coordinate an allocation.

The host is untrusted by the device, and the device is untrusted by the host.
Every transfer field is validated against the declared transfer before it is
used to slice or allocate. Returned address and viewing-key strings are checked
for the selected network prefix; canonical decoding belongs to the consuming
wallet. There is no retry, no resume, and no partial result.

Any *host-detected* violation cancels the device workflow and then raises, so
the device is not left mid-workflow holding pending consent. A device-sent
`Failure` is already terminal and is deliberately not cancelled again.
"""

from __future__ import annotations

import typing as t

from . import exceptions, messages, protobuf
from .tools import workflow

if t.TYPE_CHECKING:
    from .client import Session

_MT = t.TypeVar("_MT", bound=protobuf.MessageType)

__all__ = [
    "CHUNK_BYTES",
    "MAX_ACCOUNT",
    "MAX_PCZT_BYTES",
    "TRANSFER_ID_BYTES",
    "get_address",
    "get_viewing_key",
    "sign_pczt",
]

# Transfer limits, shared with the device side of the contract.
MAX_PCZT_BYTES = 65_536
CHUNK_BYTES = 1_024
TRANSFER_ID_BYTES = 16

# ZIP-32 account index bound. The device derives m/32'/coin_type'/account'
# itself; the host cannot supply an arbitrary derivation path.
MAX_ACCOUNT = 2**31 - 1

_UINT32_MAX = 2**32 - 1

# Capability gating is deliberately absent. `Features.Capability` value 30 is
# only the lowest free candidate at this baseline, not an upstream assignment,
# so no `@workflow(capability=...)` is declared until coordination fixes an ID.


def _check_network(network: messages.ZcashNetwork) -> None:
    # Production policy is MAINNET/TESTNET only. There is deliberately no
    # Regtest enum value; regtest remains confined to test fixtures and harnesses.
    #
    # Membership alone is value equality on an IntEnum, which would let `True`
    # select Testnet and let `1.0` pass here only to fail inside the codec,
    # after the transport was already entered. So pin the type first.
    if type(network) is not messages.ZcashNetwork or network not in (
        messages.ZcashNetwork.Mainnet,
        messages.ZcashNetwork.Testnet,
    ):
        raise ValueError("Invalid network")


def _check_account(account: int) -> None:
    # `type(x) is int` rather than isinstance: bool is an int subclass, and the
    # desktop codec does not enforce the uint32 upper bound for us.
    if type(account) is not int or not 0 <= account <= MAX_ACCOUNT:
        raise ValueError("Invalid account")


def _check_uint32(value: int, name: str) -> None:
    if type(value) is not int or not 0 <= value <= _UINT32_MAX:
        raise ValueError(f"Invalid {name}")


# ====== Client functions ====== #


@workflow()
def get_address(
    session: "Session",
    network: messages.ZcashNetwork,
    account: int,
    diversifier_index: bytes,
) -> str:
    """Return a wallet-owned Zcash Unified Address.

    The device always confirms the complete canonical address on screen; there
    is no unconfirmed path. The returned UA contains the receiver shared by
    Orchard and Ironwood.
    """
    _check_network(network)
    _check_account(account)
    if type(diversifier_index) is not bytes or len(diversifier_index) != 11:
        raise ValueError("Invalid diversifier index")

    response = _call(
        session,
        messages.ZcashGetAddress(
            network=network,
            account=account,
            diversifier_index=diversifier_index,
        ),
    )
    address = _expect(session, response, messages.ZcashAddress).address
    return _check_encoded_text(session, address, network, "address")


@workflow()
def get_viewing_key(
    session: "Session",
    network: messages.ZcashNetwork,
    account: int,
) -> str:
    """Return the Orchard-only Unified Full Viewing Key for `account`.

    The device requires an explicit privacy confirmation for every export. The
    UFVK can reveal and link wallet activity and can derive the account's
    viewing material and addresses, but it cannot confer spend authority.
    """
    _check_network(network)
    _check_account(account)

    response = _call(
        session, messages.ZcashGetViewingKey(network=network, account=account)
    )
    key = _expect(session, response, messages.ZcashViewingKey).key
    return _check_encoded_text(session, key, network, "viewing key")


@workflow()
def sign_pczt(
    session: "Session",
    pczt: bytes,
    network: messages.ZcashNetwork,
    account: int,
    host_reference_height: int,
) -> bytes:
    """Upload, review, authorize, sign, and return the exact admitted PCZT.

    `host_reference_height` is a host assertion that the device displays as
    unverified; it is not a header, checkpoint, or proof of chain state.

    The returned PCZT may be a canonical reserialization rather than being
    byte-for-byte equal to `pczt`. Before finalizing it, the caller must parse
    it and verify that it is the expected signed update of the submitted PCZT;
    this transfer layer does not interpret PCZT semantics.
    """
    _check_network(network)
    _check_account(account)
    _check_uint32(host_reference_height, "host reference height")
    if type(pczt) is not bytes:
        raise TypeError("PCZT must be bytes")
    if not 1 <= len(pczt) <= MAX_PCZT_BYTES:
        raise ValueError("Invalid PCZT length")

    response = _call(
        session,
        messages.ZcashSignPczt(
            network=network,
            account=account,
            pczt_length=len(pczt),
            host_reference_height=host_reference_height,
        ),
    )
    request = _expect(session, response, messages.ZcashPcztRequest)
    transfer_id, first_chunk = _upload(session, request, pczt)
    return _download(session, transfer_id, first_chunk)


def _cancel(session: "Session") -> None:
    """Cancel the device workflow and consume the device's answer.

    `Session.cancel()` is write-only. Using it here would leave the device's
    `Failure(ActionCancelled)` queued, so the *next* call on this session would
    read the previous request's response -- a cross-request confusion that could
    hand a caller the address or viewing key of an account it never asked for.
    Draining the reply, as `TrezorClient._callback_pin` does, keeps the session
    synchronized. If the drain itself fails the session can no longer be
    trusted, so it is invalidated rather than silently reused.
    """
    try:
        response = session.call_raw(messages.Cancel())
        if not (
            isinstance(response, messages.Failure)
            and response.code == messages.FailureType.ActionCancelled
        ):
            session.is_invalid = True
    except Exception:
        session.is_invalid = True


def _call(session: "Session", message: protobuf.MessageType) -> protobuf.MessageType:
    """Call the device, cancelling if its protobuf response cannot be decoded."""
    try:
        return session.call(message)
    except (
        ValueError,
        TypeError,
        KeyError,
        OSError,
        exceptions.ProtocolError,
    ) as error:
        _cancel(session)
        raise exceptions.ProtocolError("Malformed Zcash response") from error


def _check_encoded_text(
    session: "Session",
    value: object,
    network: messages.ZcashNetwork,
    kind: t.Literal["address", "viewing key"],
) -> str:
    """Reject empty or cross-network encodings before returning host-visible data."""
    prefixes = {
        "address": {
            messages.ZcashNetwork.Mainnet: "u1",
            messages.ZcashNetwork.Testnet: "utest1",
        },
        "viewing key": {
            messages.ZcashNetwork.Mainnet: "uview1",
            messages.ZcashNetwork.Testnet: "uviewtest1",
        },
    }
    prefix = prefixes[kind][network]
    if (
        type(value) is not str
        or len(value) <= len(prefix)
        or not value.startswith(prefix)
    ):
        _cancel_and_fail(session, f"Invalid Zcash {kind}")
    return value


def _cancel_and_fail(session: "Session", reason: str) -> t.NoReturn:
    """Abort the device workflow, then report the violation.

    `ProtocolError`, not `ValueError`: a caller must be able to tell "the device
    misbehaved" from "I passed a bad argument", because only the latter is worth
    correcting and retrying.
    """
    _cancel(session)
    raise exceptions.ProtocolError(reason)


def _expect(
    session: "Session",
    response: protobuf.MessageType,
    expected: type[_MT],
) -> _MT:
    """Require a message class, cancelling the device workflow if it differs.

    `session.call(expect=...)` raises without cancelling, which is fine for the
    single round trips above but would strand a half-finished transfer here.
    """
    if not isinstance(response, expected):
        _cancel(session)
        raise exceptions.UnexpectedMessageError(expected, response)
    return response


def _upload(
    session: "Session",
    request: messages.ZcashPcztRequest,
    pczt: bytes,
) -> tuple[bytes, messages.ZcashSignedPczt]:
    """Serve device-pulled chunks until the entire PCZT has been uploaded.

    Returns the latched transfer ID and the first signed-response chunk.
    """
    total = len(pczt)
    transfer_id = request.transfer_id
    if type(transfer_id) is not bytes or len(transfer_id) != TRANSFER_ID_BYTES:
        _cancel_and_fail(session, "Invalid transfer ID")

    # The loop re-checks the latched identity on every pass. On the first pass
    # those checks are trivially true, because the values were latched from this
    # same message; they earn their keep from the second chunk onwards.
    offset = 0
    while True:
        if request.transfer_id != transfer_id:
            _cancel_and_fail(session, "Changed transfer ID")
        if request.offset != offset:
            _cancel_and_fail(session, "Unexpected PCZT chunk offset")
        if request.length != min(CHUNK_BYTES, total - offset):
            _cancel_and_fail(session, "Unexpected PCZT chunk length")

        # Validated above, so this slice can be neither short nor out of range.
        end = offset + request.length
        response = _call(
            session,
            messages.ZcashPcztAck(
                transfer_id=transfer_id,
                offset=offset,
                data=pczt[offset:end],
            ),
        )
        offset = end

        if offset == total:
            # The upload is complete, so the device owes us the signed result.
            return transfer_id, _expect(session, response, messages.ZcashSignedPczt)

        request = _expect(session, response, messages.ZcashPcztRequest)


def _download(
    session: "Session",
    transfer_id: bytes,
    first_chunk: messages.ZcashSignedPczt,
) -> bytes:
    """Collect signed chunks, returning bytes only once the result is complete."""
    chunk = first_chunk
    # Contract order: prove identity, then bound the length, then allocate.
    if chunk.transfer_id != transfer_id:
        _cancel_and_fail(session, "Changed transfer ID")
    total = chunk.pczt_length
    if not 1 <= total <= MAX_PCZT_BYTES:
        _cancel_and_fail(session, "Invalid signed PCZT length")

    signed = bytearray(total)
    # As in `_upload`, the identity checks below are trivially true on the first
    # pass and load-bearing on every later one.
    offset = 0
    while True:
        if chunk.transfer_id != transfer_id:
            _cancel_and_fail(session, "Changed transfer ID")
        if chunk.pczt_length != total:
            _cancel_and_fail(session, "Changed signed PCZT length")
        if chunk.offset != offset:
            _cancel_and_fail(session, "Unexpected signed PCZT chunk offset")
        if len(chunk.data) != min(CHUNK_BYTES, total - offset):
            _cancel_and_fail(session, "Unexpected signed PCZT chunk length")

        end = offset + len(chunk.data)
        signed[offset:end] = chunk.data
        offset = end

        response = _call(
            session,
            messages.ZcashSignedPcztAck(
                transfer_id=transfer_id,
                next_offset=offset,
            ),
        )

        if offset == total:
            # Only a complete transfer may terminate, and only with Success.
            _expect(session, response, messages.Success)
            return bytes(signed)

        chunk = _expect(session, response, messages.ZcashSignedPczt)
