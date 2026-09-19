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
used to slice or allocate. Returned viewing keys are fully decoded and checked
for the one Orchard-only product shape. There is no retry, no resume, and no
partial result.

Any *host-detected* violation cancels the device workflow and then raises, so
the device is not left mid-workflow holding pending consent. A device-sent
`Failure` is already terminal and is deliberately not cancelled again.
"""

from __future__ import annotations

import hashlib
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
    "POOL_IRONWOOD",
    "RECORD_BYTES",
    "SpendAuthSignature",
    "TRANSFER_ID_BYTES",
    "get_address",
    "get_viewing_key",
    "sign_pczt",
]

# Transfer limits, shared with the device side of the contract.
MAX_PCZT_BYTES = 65_536
CHUNK_BYTES = 1_024
TRANSFER_ID_BYTES = 16

# Signature records: pool u8 | action_index u8 | signature[64], ascending index
# order, one per real spend, at most one per admitted action.
POOL_IRONWOOD = 0x03
RECORD_BYTES = 66
# Must track the firmware cap (core/embed/ironwood/src/wire.rs MAX_ACTIONS),
# raised 8 -> 32 for 16/32-action signing.
MAX_ACTIONS = 32


class SpendAuthSignature(t.NamedTuple):
    """One spend authorization signature returned by the device.

    Apply it to the host's copy of the PCZT with the `pczt` crate's
    `Signer::apply_orchard_spend_auth_signature`, which verifies the signature
    against the indexed action's `rk` and the host-computed sighash before
    storing it. This library carries no PCZT parser, so it does not apply
    records itself.
    """

    action_index: int
    signature: bytes


# ZIP-32 account index bound. The device derives m/32'/coin_type'/account'
# itself; the host cannot supply an arbitrary derivation path.
MAX_ACCOUNT = 2**31 - 1

_UINT32_MAX = 2**32 - 1

# MEASUREMENT-ONLY: last per-phase timing trailer read from a signing-latency
# instrumentation firmware (bytes), or None against a release build.
last_debug_timings: bytes | None = None

_BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
_BECH32_CHARSET_INDEX = {char: index for index, char in enumerate(_BECH32_CHARSET)}
_BECH32M_CONST = 0x2BC830A3
_ORCHARD_FVK_BYTES = 96
_UNIFIED_FVK_BYTES = 114

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
    if not _has_canonical_orchard_ufvk_envelope(key, network):
        _cancel_and_fail(session, "Invalid Zcash viewing key")
    return key


@workflow()
def sign_pczt(
    session: "Session",
    pczt: bytes,
    network: messages.ZcashNetwork,
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
    transfer_id, signatures = _upload(session, request, pczt)
    # MEASUREMENT-ONLY: the signing-latency instrumentation firmware appends an
    # ASCII per-phase timing trailer to the response. Stash it so a plain host
    # runner (no DebugLink) can read the derive/feed/sign/high-water breakdown.
    # This field is absent from release builds, so this is always a no-op there.
    global last_debug_timings
    last_debug_timings = getattr(signatures, "debug_timings", None)
    return _parse_records(session, transfer_id, signatures)


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
    kind: t.Literal["address"],
) -> str:
    """Reject empty or cross-network encodings before returning host-visible data."""
    prefixes = {
        "address": {
            messages.ZcashNetwork.Mainnet: "u1",
            messages.ZcashNetwork.Testnet: "utest1",
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


def _has_canonical_orchard_ufvk_envelope(
    value: object, network: messages.ZcashNetwork
) -> bool:
    """Validate the canonical ZIP-316 envelope and Orchard-only item shape.

    The consuming wallet must still parse the 96-byte item as an Orchard FVK;
    this transport check deliberately does not implement Orchard field math.
    """
    hrp = {
        messages.ZcashNetwork.Mainnet: "uview",
        messages.ZcashNetwork.Testnet: "uviewtest",
    }[network]
    expected_length = 195 if network is messages.ZcashNetwork.Mainnet else 199
    if (
        type(value) is not str
        or len(value) != expected_length
        or value != value.lower()
    ):
        return False

    try:
        encoded_hrp, data = _bech32m_decode(value)
        if encoded_hrp != hrp:
            return False
        jumbled = bytearray(_convert_bits(data, 5, 8, pad=False))
        if len(jumbled) != _UNIFIED_FVK_BYTES:
            return False
        _f4jumble(jumbled, inverse=True)
    except (KeyError, ValueError):
        return False

    padding = hrp.encode() + bytes(16 - len(hrp))
    if jumbled[:2] != bytes((3, _ORCHARD_FVK_BYTES)) or jumbled[98:] != padding:
        return False

    # A successful decode is not enough: only the unique canonical spelling is
    # accepted, so future decoder changes cannot silently normalize a response.
    _f4jumble(jumbled, inverse=False)
    canonical_data = _convert_bits(jumbled, 8, 5, pad=True)
    return _bech32m_encode(hrp, canonical_data) == value


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
) -> tuple[bytes, messages.ZcashSpendAuthSignatures]:
    """Serve device-pulled chunks until the entire PCZT has been uploaded.

    Returns the latched transfer ID and the signature response.
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
            # The upload is complete, so the device owes us the signatures.
            return transfer_id, _expect(
                session, response, messages.ZcashSpendAuthSignatures
            )

        request = _expect(session, response, messages.ZcashPcztRequest)


def _parse_records(
    session: "Session",
    transfer_id: bytes,
    response: messages.ZcashSpendAuthSignatures,
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
