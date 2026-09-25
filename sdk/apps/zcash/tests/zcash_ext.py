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

"""Host side of the Zcash app: its messages wrapped in ExtAppMessage, and the
ZIP-316 decoding the tests need."""

from __future__ import annotations

import hashlib
import io
import typing as t

from trezorlib import exceptions, protobuf
from trezorlib.messages import ExtAppMessage, ExtAppResponse, Failure

from .generated import messages as zcash_messages

if t.TYPE_CHECKING:
    from trezorlib.client import Session


def call_ext(
    session: "Session",
    instance_id: int,
    msg: protobuf.MessageType,
    expect: type[protobuf.MessageType],
) -> t.Any:
    """Send one of the app's messages and return its response of type `expect`."""
    buf = io.BytesIO()
    protobuf.dump_message(buf, msg)
    return call_raw(
        session,
        instance_id,
        zcash_messages.MessageType[type(msg).__name__],
        buf.getvalue(),
        expect,
    )


def call_raw(
    session: "Session",
    instance_id: int,
    message_id: int,
    data: bytes,
    expect: type[protobuf.MessageType],
) -> t.Any:
    """Send an encoded message, which may be one trezorlib would not encode."""
    request = ExtAppMessage(instance_id=instance_id, message_id=message_id, data=data)
    with session:
        resp = session.client._call(session, request, expect=ExtAppResponse)
    if resp.message_id != zcash_messages.MessageType[expect.__name__]:
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


def orchard_fvk(ufvk: str) -> bytes:
    """The 96-byte Orchard item of an Orchard-only UFVK (ZIP 316)."""
    jumbled = bytearray(_convert_bits_5_to_8(_bech32m_data(ufvk)))
    _f4unjumble(jumbled)
    assert jumbled[:2] == bytes((3, 96))
    return bytes(jumbled[2:98])


_BECH32_CHARSET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l"
_BECH32M_CONST = 0x2BC830A3


def _bech32m_data(value: str) -> list[int]:
    separator = value.rfind("1")
    hrp = value[:separator]
    data = [_BECH32_CHARSET.index(char) for char in value[separator + 1 :]]
    hrp_expanded = [ord(c) >> 5 for c in hrp] + [0] + [ord(c) & 31 for c in hrp]
    if _bech32_polymod(hrp_expanded + data) != _BECH32M_CONST:
        raise ValueError("Invalid Bech32m checksum")
    return data[:-6]


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


def _convert_bits_5_to_8(data: list[int]) -> list[int]:
    accumulator = bit_count = 0
    result = []
    for value in data:
        accumulator = ((accumulator << 5) | value) & 0xFFF
        bit_count += 5
        if bit_count >= 8:
            bit_count -= 8
            result.append((accumulator >> bit_count) & 0xFF)
    if bit_count >= 5 or (accumulator << (8 - bit_count)) & 0xFF:
        raise ValueError("Invalid base-conversion padding")
    return result


def _f4unjumble(message: bytearray) -> None:
    """Inverse F4Jumble (ZIP 316), in place."""
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

    h_round(1)
    g_round(1)
    h_round(0)
    g_round(0)
