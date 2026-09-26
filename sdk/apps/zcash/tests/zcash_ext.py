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

"""Host side of the Zcash app: its messages wrapped in ExtAppMessage, the
streamed PCZT signer, and the ZIP-316 encodings the tests need.

`sign_pczt` mirrors trezorlib's `zcash.sign_pczt` of the firmware series
(bawolf/trezor-firmware@7864a22444, `python/src/trezorlib/zcash.py`), over the
app platform's `ExtAppMessage`/`ExtAppResponse`: a `ZcashPcztRequest` arrives
as an unfinished `ExtAppResponse` and is answered with another
`ExtAppMessage`."""

from __future__ import annotations

import hashlib
import io
import typing as t

from trezorlib import exceptions, protobuf
from trezorlib.messages import ExtAppMessage, ExtAppResponse, Failure, FailureType

from .generated import messages as zcash_messages

if t.TYPE_CHECKING:
    from trezorlib.client import Session

_MT = t.TypeVar("_MT", bound=protobuf.MessageType)

# Transfer limits, shared with the app.
MAX_PCZT_BYTES = 65_536
CHUNK_BYTES = 1_024
TRANSFER_ID_BYTES = 16

# Signature records: pool u8 | action_index u8 | signature[64], ascending index
# order, one per real spend, at most one per admitted action.
POOL_IRONWOOD = 0x03
RECORD_BYTES = 66
MAX_ACTIONS = 32

# ZIP-32 account index bound. The device derives m/32'/coin_type'/account'
# itself; the host cannot supply an arbitrary derivation path.
MAX_ACCOUNT = 2**31 - 1


class SpendAuthSignature(t.NamedTuple):
    """One spend authorization signature returned by the device.

    Apply it to the host's copy of the PCZT with the `pczt` crate's
    `Signer::apply_orchard_spend_auth_signature`, which verifies the signature
    against the indexed action's `rk` and the host-computed sighash.
    """

    action_index: int
    signature: bytes


def _message_id(message_type: type[protobuf.MessageType]) -> int:
    return zcash_messages.MessageType[message_type.__name__]


def _encode(msg: protobuf.MessageType) -> bytes:
    buf = io.BytesIO()
    protobuf.dump_message(buf, msg)
    return buf.getvalue()


def _exchange(
    session: "Session", instance_id: int, message_id: int, data: bytes
) -> ExtAppResponse:
    """One ExtAppMessage round trip; a Failure raises."""
    request = ExtAppMessage(instance_id=instance_id, message_id=message_id, data=data)
    return session.call(request, expect=ExtAppResponse)


def call_ext(
    session: "Session",
    instance_id: int,
    msg: protobuf.MessageType,
    expect: type[_MT],
) -> _MT:
    """Send one of the app's messages and return its response of type `expect`."""
    return call_raw(session, instance_id, _message_id(type(msg)), _encode(msg), expect)


def call_raw(
    session: "Session",
    instance_id: int,
    message_id: int,
    data: bytes,
    expect: type[_MT],
) -> _MT:
    """Send an encoded message, which may be one trezorlib would not encode."""
    resp = _exchange(session, instance_id, message_id, data)
    if resp.message_id != _message_id(expect):
        raise exceptions.TrezorFailure(
            failure=Failure(message="Unexpected response type")
        )
    return protobuf.load_message(io.BytesIO(resp.data), expect)


def get_address(
    session: "Session",
    instance_id: int,
    network: zcash_messages.ZcashNetwork,
    account: int,
    diversifier_index: bytes,
    chunkify: bool = False,
) -> str:
    return call_ext(
        session,
        instance_id,
        zcash_messages.ZcashGetAddress(
            network=network,
            account=account,
            diversifier_index=diversifier_index,
            chunkify=chunkify,
        ),
        zcash_messages.ZcashAddress,
    ).address


def export_viewing_key(
    session: "Session",
    instance_id: int,
    network: zcash_messages.ZcashNetwork,
    account: int,
    include_seed_fingerprint: bool = False,
) -> zcash_messages.ZcashViewingKey:
    return call_ext(
        session,
        instance_id,
        zcash_messages.ZcashGetViewingKey(
            network=network,
            account=account,
            include_seed_fingerprint=include_seed_fingerprint,
        ),
        zcash_messages.ZcashViewingKey,
    )


def sign_pczt(
    session: "Session",
    instance_id: int,
    pczt: bytes,
    network: zcash_messages.ZcashNetwork,
    account: int,
    host_reference_height: int,
) -> list[SpendAuthSignature]:
    """Upload and review a PCZT; return the device's spend authorization signatures.

    `host_reference_height` is a host assertion used by device policy only; it
    is not a header, checkpoint, or proof of chain state.

    The device streams the PCZT and never returns it. The result is one
    `SpendAuthSignature` per real Ironwood spend in ascending action order; the
    caller applies them to its own copy of `pczt` (see `SpendAuthSignature`),
    which is also where each signature is verified against the transaction.
    """
    if network not in (
        zcash_messages.ZcashNetwork.Mainnet,
        zcash_messages.ZcashNetwork.Testnet,
    ):
        raise ValueError("Invalid Zcash network")
    if type(account) is not int or not 0 <= account <= MAX_ACCOUNT:
        raise ValueError("Invalid account")
    if type(host_reference_height) is not int or not (
        0 <= host_reference_height < 2**32
    ):
        raise ValueError("Invalid host reference height")
    if type(pczt) is not bytes:
        raise TypeError("PCZT must be bytes")
    if not 1 <= len(pczt) <= MAX_PCZT_BYTES:
        raise ValueError("Invalid PCZT length")

    request = _stream_call(
        session,
        instance_id,
        zcash_messages.ZcashSignPczt(
            network=network,
            account=account,
            pczt_length=len(pczt),
            host_reference_height=host_reference_height,
        ),
        zcash_messages.ZcashPcztRequest,
    )
    transfer_id, signatures = _upload(session, instance_id, request, pczt)
    return _parse_records(transfer_id, signatures)


def get_diagnostics(
    session: "Session", instance_id: int
) -> zcash_messages.ZcashDiagnostics:
    """The app's heap and IPC counters since the previous call, which starts
    new ones. Debug builds of the app only: a release build stops on it."""
    return call_ext(
        session,
        instance_id,
        zcash_messages.ZcashGetDiagnostics(),
        zcash_messages.ZcashDiagnostics,
    )


def cancel(session: "Session", instance_id: int) -> None:
    """Abandon the signing request in progress and consume the device's answer.

    The app platform does not pass trezorlib's `Cancel` to an app that waits
    for its next chunk, so the app has its own `ZcashCancel`. The device answers
    it with `Failure(ActionCancelled)`; anything else leaves the session in an
    unknown state, so it is invalidated rather than silently reused.
    """
    try:
        response = session.call_raw(
            ExtAppMessage(
                instance_id=instance_id,
                message_id=_message_id(zcash_messages.ZcashCancel),
                data=b"",
            )
        )
        if not (
            isinstance(response, Failure)
            and response.code == FailureType.ActionCancelled
        ):
            session.is_invalid = True
    except Exception:
        session.is_invalid = True


def _cancel_and_fail(session: "Session", instance_id: int, reason: str) -> t.NoReturn:
    """Abort the device workflow, then report the violation.

    `ProtocolError`, not `ValueError`: a caller must be able to tell "the device
    misbehaved" from "I passed a bad argument".
    """
    cancel(session, instance_id)
    raise exceptions.ProtocolError(reason)


def _stream_call(
    session: "Session",
    instance_id: int,
    msg: protobuf.MessageType,
    expect: type[_MT],
) -> _MT:
    """Send `msg` and decode the response as `expect`, cancelling the device
    workflow if it is anything else. Only the final response of the workflow
    is `finished`; a `ZcashPcztRequest` waits for an answer."""
    resp = _exchange(session, instance_id, _message_id(type(msg)), _encode(msg))
    finished = expect is zcash_messages.ZcashSpendAuthSignatures
    if resp.message_id != _message_id(expect) or resp.finished is not finished:
        if not resp.finished:
            cancel(session, instance_id)
        raise exceptions.ProtocolError(f"Expected {expect.__name__}")
    try:
        return protobuf.load_message(io.BytesIO(resp.data), expect)
    except Exception as error:
        if not finished:
            cancel(session, instance_id)
        raise exceptions.ProtocolError("Malformed Zcash response") from error


def _upload(
    session: "Session",
    instance_id: int,
    request: zcash_messages.ZcashPcztRequest,
    pczt: bytes,
) -> tuple[bytes, zcash_messages.ZcashSpendAuthSignatures]:
    """Serve device-pulled chunks until the entire PCZT has been uploaded.

    Returns the latched transfer ID and the signature response.
    """
    total = len(pczt)
    transfer_id = request.transfer_id
    if type(transfer_id) is not bytes or len(transfer_id) != TRANSFER_ID_BYTES:
        _cancel_and_fail(session, instance_id, "Invalid transfer ID")

    # The loop re-checks the latched identity on every pass. On the first pass
    # those checks are trivially true, because the values were latched from this
    # same message; they earn their keep from the second chunk onwards.
    offset = 0
    while True:
        if request.transfer_id != transfer_id:
            _cancel_and_fail(session, instance_id, "Changed transfer ID")
        if request.offset != offset:
            _cancel_and_fail(session, instance_id, "Unexpected PCZT chunk offset")
        if request.length != min(CHUNK_BYTES, total - offset):
            _cancel_and_fail(session, instance_id, "Unexpected PCZT chunk length")

        # Validated above, so this slice can be neither short nor out of range.
        end = offset + request.length
        ack = zcash_messages.ZcashPcztAck(
            transfer_id=transfer_id, offset=offset, data=pczt[offset:end]
        )
        offset = end

        if offset == total:
            # The upload is complete, so the device owes us the signatures.
            signatures = _stream_call(
                session, instance_id, ack, zcash_messages.ZcashSpendAuthSignatures
            )
            return transfer_id, signatures

        request = _stream_call(
            session, instance_id, ack, zcash_messages.ZcashPcztRequest
        )


def _parse_records(
    transfer_id: bytes,
    response: zcash_messages.ZcashSpendAuthSignatures,
) -> list[SpendAuthSignature]:
    """Split the record blob, rejecting anything but the one admissible shape.

    The device workflow has already finished, so a violation here raises
    without cancelling; the caller must not use any of the records.
    """
    if response.transfer_id != transfer_id:
        raise exceptions.ProtocolError("Changed transfer ID")
    records = response.records
    if (
        type(records) is not bytes
        or len(records) == 0
        or len(records) % RECORD_BYTES
        or len(records) // RECORD_BYTES > MAX_ACTIONS
    ):
        raise exceptions.ProtocolError("Invalid Zcash signature records")

    signatures: list[SpendAuthSignature] = []
    previous_index = -1
    for start in range(0, len(records), RECORD_BYTES):
        pool, action_index = records[start], records[start + 1]
        signature = records[start + 2 : start + RECORD_BYTES]
        if (
            pool != POOL_IRONWOOD
            or action_index <= previous_index
            or action_index >= MAX_ACTIONS
        ):
            raise exceptions.ProtocolError("Invalid Zcash signature records")
        previous_index = action_index
        signatures.append(SpendAuthSignature(action_index, signature))
    return signatures


def orchard_fvk(ufvk: str) -> bytes:
    """The 96-byte Orchard item of an Orchard-only UFVK (ZIP 316)."""
    _hrp, data = _bech32m_decode(ufvk)
    jumbled = bytearray(_convert_bits(data, 5, 8, pad=False))
    _f4jumble(jumbled, inverse=True)
    assert jumbled[:2] == bytes((3, 96))
    return bytes(jumbled[2:98])


def unified_address(receivers: dict[int, bytes], hrp: str) -> str:
    """A ZIP-316 unified address for `receivers` (typecode -> receiver bytes),
    built on the host for vectors that need one the wallet did not make."""
    items = b"".join(
        bytes((typecode, len(receiver))) + receiver
        for typecode, receiver in sorted(receivers.items())
    )
    message = bytearray(items + hrp.encode() + bytes(16 - len(hrp)))
    _f4jumble(message, inverse=False)
    return _bech32m_encode(hrp, _convert_bits(message, 8, 5, pad=True))


# The Bech32m, base-conversion and F4Jumble helpers of the series' trezorlib.
_BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
_BECH32_CHARSET_INDEX = {char: index for index, char in enumerate(_BECH32_CHARSET)}
_BECH32M_CONST = 0x2BC830A3


def _bech32m_decode(value: str) -> tuple[str, list[int]]:
    separator = value.rfind("1")
    if separator <= 0 or separator + 7 > len(value):
        raise ValueError("Invalid Bech32m separator")
    hrp = value[:separator]
    data = [_BECH32_CHARSET_INDEX[char] for char in value[separator + 1 :]]
    if _bech32_polymod(_bech32_hrp_expand(hrp) + data) != _BECH32M_CONST:
        raise ValueError("Invalid Bech32m checksum")
    return hrp, data[:-6]


def _bech32m_encode(hrp: str, data: list[int]) -> str:
    values = _bech32_hrp_expand(hrp) + data + [0] * 6
    checksum = _bech32_polymod(values) ^ _BECH32M_CONST
    checksum_values = [(checksum >> (5 * (5 - index))) & 31 for index in range(6)]
    return hrp + "1" + "".join(_BECH32_CHARSET[item] for item in data + checksum_values)


def _bech32_hrp_expand(hrp: str) -> list[int]:
    return [ord(char) >> 5 for char in hrp] + [0] + [ord(char) & 31 for char in hrp]


def _bech32_polymod(values: t.Iterable[int]) -> int:
    generators = (0x3B6A57B2, 0x26508E6D, 0x1EA119FA, 0x3D4233DD, 0x2A1462B3)
    checksum = 1
    for value in values:
        top = checksum >> 25
        checksum = ((checksum & 0x1FFFFFF) << 5) ^ value
        for index, generator in enumerate(generators):
            if (top >> index) & 1:
                checksum ^= generator
    return checksum


def _convert_bits(
    data: t.Iterable[int], from_bits: int, to_bits: int, *, pad: bool
) -> list[int]:
    accumulator = 0
    bit_count = 0
    result = []
    output_mask = (1 << to_bits) - 1
    accumulator_mask = (1 << (from_bits + to_bits - 1)) - 1
    for value in data:
        if value < 0 or value >> from_bits:
            raise ValueError("Invalid base-conversion input")
        accumulator = ((accumulator << from_bits) | value) & accumulator_mask
        bit_count += from_bits
        while bit_count >= to_bits:
            bit_count -= to_bits
            result.append((accumulator >> bit_count) & output_mask)

    if pad:
        if bit_count:
            result.append((accumulator << (to_bits - bit_count)) & output_mask)
    elif bit_count >= from_bits or (
        (accumulator << (to_bits - bit_count)) & output_mask
    ):
        raise ValueError("Invalid base-conversion padding")
    return result


def _f4jumble(message: bytearray, *, inverse: bool) -> None:
    """F4Jumble (ZIP 316), or its inverse, in place."""
    left_length = min(64, len(message) // 2)
    left = memoryview(message)[:left_length]
    right = memoryview(message)[left_length:]

    def xor(target: memoryview, mask: bytes) -> None:
        for index in range(len(target)):
            target[index] ^= mask[index]

    def g_round(round_index: int) -> None:
        for block_index in range((len(right) + 63) // 64):
            personalization = (
                b"UA_F4Jumble_G"
                + bytes((round_index,))
                + block_index.to_bytes(2, "little")
            )
            mask = hashlib.blake2b(left, person=personalization).digest()
            xor(right[block_index * 64 : (block_index + 1) * 64], mask)

    def h_round(round_index: int) -> None:
        personalization = b"UA_F4Jumble_H" + bytes((round_index, 0, 0))
        mask = hashlib.blake2b(
            right, digest_size=len(left), person=personalization
        ).digest()
        xor(left, mask)

    if inverse:
        h_round(1)
        g_round(1)
        h_round(0)
        g_round(0)
    else:
        g_round(0)
        h_round(0)
        g_round(1)
        h_round(1)
