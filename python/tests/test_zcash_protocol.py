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

"""Schema, wire-allocation, and codec tests for the provisional Zcash protocol.

These guard the properties that must not drift before upstream coordination:
the identifiers stay isolated and labelled provisional, the donor's private
identifiers stay unimplemented, and the production wire carries no regtest network.
"""

from __future__ import annotations

import re
import warnings
from functools import lru_cache
from io import BytesIO
from pathlib import Path

import pytest

from trezorlib import messages, protobuf

REPO_ROOT = Path(__file__).resolve().parents[2]
MESSAGES_PROTO = REPO_ROOT / "common" / "protob" / "messages.proto"
ZCASH_PROTO = REPO_ROOT / "common" / "protob" / "messages-zcash.proto"
requires_repo_schema = pytest.mark.skipif(
    not (MESSAGES_PROTO.exists() and ZCASH_PROTO.exists()),
    reason="protobuf sources are not included in the trezorlib sdist",
)

# The provisional local-only allocation from the contract, section 2.2.
PROVISIONAL = {
    "ZcashGetAddress": (32100, "in"),
    "ZcashAddress": (32101, "out"),
    "ZcashGetViewingKey": (32102, "in"),
    "ZcashViewingKey": (32103, "out"),
    "ZcashSignPczt": (32104, "in"),
    "ZcashPcztRequest": (32105, "out"),
    "ZcashPcztAck": (32106, "in"),
    "ZcashSpendAuthSignatures": (32109, "out"),
}

# Freed by the collapse to a single response shape: they carried the removed
# whole-PCZT download pair (signed chunk + ack). Nothing may reclaim them
# without coordination.
FREED_WIRE_IDS = (32107, 32108)

# Numbers the contract names only as candidates for a future coordinated
# allocation. Nothing may claim them yet, so the tests assert they are FREE.
CANDIDATE_WIRE_IDS = range(2300, 2309)
CANDIDATE_CAPABILITY = 30

# The donor's private identifiers, which must remain entirely unimplemented.
DONOR_WIRE_IDS = range(32000, 32009)

NETWORKS = [messages.ZcashNetwork.Mainnet, messages.ZcashNetwork.Testnet]
# A 32-byte value with the seed fingerprint's shape (PUBLIC TEST VALUE).
SEED_FINGERPRINT = bytes(range(32))
VIEWING_KEYS = {
    messages.ZcashNetwork.Mainnet: "uview17j0q0nnczz63ducvkhe409f4r8sa2gx88unakv64k95dpe4r2hvn3lhe2gdfn00vsl830682a7tdhzwuhtsw2dp7usgxzdgqxujgu4pv50xrhuakfuk294xjcuhrs5ag0esenlp4wsawqmuqaaspykcplgk0vrds7fm0hrp3up2mmzgh7rdfhycgu2xp8",
    messages.ZcashNetwork.Testnet: "uviewtest1frzzf669pvkxdjsgwf6y43tvuulek5l3fujvjsfrddrqs07mvpmaa2tua4jhdw4n3ekkqdxq9zgl53r8axe6l3sdzddlwuv3fz6tkyzv4xfkpfmkuevv2q46sapk5d3lhp7m5te04k7ulpv9j3sa08w7akay2xlpj68ly3355l0pgcydz3kvu5c335ggc",
}


@lru_cache(maxsize=1)
def _wire_entries() -> dict[str, tuple[int, set[str]]]:
    """Parse the Zcash block of the MessageType enum out of messages.proto."""
    entries = {}
    pattern = re.compile(
        r"^\s*MessageType_(Zcash\w+)\s*=\s*(\d+)\s*\[(.*)\];\s*$", re.MULTILINE
    )
    for name, value, options in pattern.findall(MESSAGES_PROTO.read_text()):
        directions = set()
        if "(wire_in) = true" in options:
            directions.add("in")
        if "(wire_out) = true" in options:
            directions.add("out")
        entries[name] = (int(value), directions)
    return entries


# ====== Allocation ====== #


@pytest.mark.parametrize("name, expected", sorted(PROVISIONAL.items()))
@requires_repo_schema
def test_provisional_wire_id(name: str, expected: tuple[int, str]) -> None:
    value, direction = expected
    assert messages.MessageType[name] == value
    assert getattr(messages, name).MESSAGE_WIRE_TYPE == value
    assert _wire_entries()[name] == (value, {direction})


def test_no_wire_id_collisions() -> None:
    values = [int(m) for m in messages.MessageType.__members__.values()]
    assert len(values) == len(set(values))


def test_zcash_messages_are_exactly_the_contract_set() -> None:
    declared = {m.name for m in messages.MessageType if m.name.startswith("Zcash")}
    assert declared == set(PROVISIONAL)


@requires_repo_schema
def test_freed_download_ids_stay_unclaimed() -> None:
    """The whole-PCZT download pair is gone; its IDs are freed, not reused."""
    taken = {int(m) for m in messages.MessageType}
    assert taken.isdisjoint(FREED_WIRE_IDS)
    proto = MESSAGES_PROTO.read_text()
    for value in FREED_WIRE_IDS:
        assert f"= {value} " not in proto
    # protoc enforces the freeing: a `reserved` statement makes any reuse of
    # the numbers a compile error, the enum's own idiom (cf. `reserved 219;`).
    assert re.search(r"^\s*reserved 32107, 32108;", proto, re.M)
    # No download-shaped message survives, by name or by field.
    schema = ZCASH_PROTO.read_text()
    assert not re.search(r"^message Zcash\w*Signed\w*", schema, re.M)
    assert "next_offset" not in schema
    assert not any(
        n.startswith("Zcash") and "Signed" in n for n in messages.MessageType.__members__
    )


def test_signing_has_exactly_one_response_shape() -> None:
    """The device answers a completed upload with records, never a PCZT."""
    fields = {f.name for f in messages.ZcashSpendAuthSignatures.FIELDS.values()}
    assert {"transfer_id", "records"} <= fields
    assert "pczt_length" not in fields and "data" not in fields


def test_candidate_allocation_is_still_free() -> None:
    """The coordinated candidates are unclaimed, and we do not claim them."""
    taken = {int(m) for m in messages.MessageType}
    assert taken.isdisjoint(CANDIDATE_WIRE_IDS)
    assert CANDIDATE_CAPABILITY not in {int(c) for c in messages.Capability}
    assert not hasattr(messages.Capability, "Zcash")


@requires_repo_schema
def test_donor_identifiers_are_unimplemented() -> None:
    taken = {int(m) for m in messages.MessageType}
    assert taken.isdisjoint(DONOR_WIRE_IDS)
    proto = MESSAGES_PROTO.read_text()
    for value in DONOR_WIRE_IDS:
        # The trailing space is load-bearing: it stops "= 32000" from matching
        # "= 320001". Every wire-registered entry carries an options bracket, so
        # a space always follows the number.
        assert f"= {value} " not in proto
    # No donor message name may gain a registration. ("Ironwood" appears in a
    # comment naming the design note, so match the identifier form instead.)
    assert "MessageType_Ironwood" not in proto


def test_wire_ids_fit_the_firmware_codec() -> None:
    """Wire IDs must stay below the 15-bit codec ceiling.

    `common/protob/pb2py:618` rejects any wire ID above 0x7FFF, and
    `core/embed/rust/src/protobuf/defs.rs:225` masks to 15 bits with 0x7FFF as
    the "no wire ID" sentinel. That check runs only in the firmware blob build,
    so a bad ID would otherwise not surface until a full Core build.
    """
    for name, (value, _) in PROVISIONAL.items():
        assert value < 0x7FFF, f"{name} = {value} exceeds the 15-bit wire ID space"
    # The candidate coordinated block must satisfy the same ceiling.
    assert max(CANDIDATE_WIRE_IDS) < 0x7FFF


def test_no_diagnostic_or_memory_trace_messages() -> None:
    """The donor's memory-trace endpoints must stay unimplemented."""
    for name in messages.MessageType.__members__:
        assert "MemoryTrace" not in name


# ====== Provisional labelling ====== #


def _squash(text: str) -> str:
    """Collapse comment wrapping so phrase checks survive reflowing."""
    return " ".join(text.replace("//", " ").replace("*", " ").lower().split())


@requires_repo_schema
def test_schema_declares_itself_provisional() -> None:
    for path in (MESSAGES_PROTO, ZCASH_PROTO):
        text = path.read_text()
        assert "PROVISIONAL" in text, path
        assert "not assigned or reserved upstream" in _squash(text), path


@requires_repo_schema
def test_no_identifier_is_claimed_as_upstream() -> None:
    """Every mention of upstream assignment must be a denial, not a claim."""
    block = _squash(MESSAGES_PROTO.read_text().split("// Zcash", 1)[1])
    for claim in ("assigned", "reserved"):
        # Prose only: protoc's `reserved <numbers>;` statement is the enum's
        # own freed-ID mechanism (test_freed_download_ids_stay_unclaimed pins
        # it), not a statement about upstream allocation.
        for match in re.finditer(rf"\b{claim}\b(?!\s+\d)", block):
            # Wide enough to survive comment reflow, narrow enough not to reach
            # back into the previous sentence and find an unrelated negation.
            preceding = block[max(0, match.start() - 40) : match.start()]
            context = block[max(0, match.start() - 60) : match.end() + 20]
            assert "not " in preceding, f"unnegated {claim!r} near: {context}"


# ====== Network policy ====== #


def test_network_enum_is_mainnet_and_testnet_only() -> None:
    assert {m.name: int(m) for m in messages.ZcashNetwork} == {
        "Mainnet": 0,
        "Testnet": 1,
    }


@requires_repo_schema
def test_no_regtest_value_in_production_schema() -> None:
    """Regtest must exist nowhere on the wire, only in test-only fixtures."""
    assert not hasattr(messages.ZcashNetwork, "Regtest")
    declared = re.findall(r"^\s*ZcashNetwork_(\w+)\s*=", ZCASH_PROTO.read_text(), re.M)
    assert declared == ["Mainnet", "Testnet"]
    # messages.proto must not mention regtest in the Zcash block at all. The
    # schema file documents the exclusion but defines no production value.
    zcash_block = MESSAGES_PROTO.read_text().split("// Zcash", 1)[1]
    assert "regtest" not in zcash_block.lower()


# ====== Codec ====== #


def _roundtrip(msg: protobuf.MessageType) -> protobuf.MessageType:
    buf = BytesIO()
    protobuf.dump_message(buf, msg)
    buf.seek(0)
    return protobuf.load_message(buf, type(msg))


@pytest.mark.parametrize("network", NETWORKS)
def test_roundtrip_all_messages(network: messages.ZcashNetwork) -> None:
    transfer_id = bytes(range(16))
    for msg in (
        messages.ZcashGetAddress(
            network=network, account=0, diversifier_index=bytes(11)
        ),
        messages.ZcashAddress(address="u1example"),
        messages.ZcashGetViewingKey(network=network, account=7),
        messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=VIEWING_KEYS[network]),
        messages.ZcashSignPczt(
            network=network,
            account=7,
            pczt_length=2429,
            host_reference_height=10_000_000,
        ),
        messages.ZcashPcztRequest(transfer_id=transfer_id, offset=0, length=1024),
        messages.ZcashPcztAck(transfer_id=transfer_id, offset=0, data=b"\x00" * 1024),
        messages.ZcashSpendAuthSignatures(
            transfer_id=transfer_id, records=b"\x03\x00" + b"\x01" * 64
        ),
    ):
        assert _roundtrip(msg) == msg


@pytest.mark.parametrize(
    "network, encoded_size",
    # The key plus the 32-byte fingerprint field (2 bytes of framing).
    [(messages.ZcashNetwork.Mainnet, 232), (messages.ZcashNetwork.Testnet, 236)],
)
def test_viewing_key_response_has_fixed_small_wire_size(
    network: messages.ZcashNetwork, encoded_size: int
) -> None:
    buf = BytesIO()
    protobuf.dump_message(buf, messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=VIEWING_KEYS[network]))
    assert len(buf.getvalue()) == encoded_size


@pytest.mark.parametrize(
    "cls, kwargs",
    [
        # An omitted scalar must not silently mean network, account, height,
        # length, or offset zero.
        (messages.ZcashGetAddress, {"account": 0, "diversifier_index": bytes(11)}),
        (
            messages.ZcashGetAddress,
            {"network": messages.ZcashNetwork.Mainnet, "diversifier_index": bytes(11)},
        ),
        (messages.ZcashGetViewingKey, {"network": messages.ZcashNetwork.Mainnet}),
        (messages.ZcashViewingKey, {}),
        (
            messages.ZcashSignPczt,
            {
                "network": messages.ZcashNetwork.Mainnet,
                "account": 0,
                "pczt_length": 1,
            },
        ),
        (messages.ZcashPcztRequest, {"transfer_id": bytes(16), "offset": 0}),
        (messages.ZcashSpendAuthSignatures, {"transfer_id": bytes(16)}),
    ],
    ids=lambda v: v.__name__ if isinstance(v, type) else "",
)
def test_missing_required_field_is_rejected(
    cls: type[protobuf.MessageType], kwargs: dict[str, object]
) -> None:
    with warnings.catch_warnings():
        # Constructing an incomplete message only warns; encoding must raise.
        warnings.simplefilter("ignore", DeprecationWarning)
        msg = cls(**kwargs)
    with pytest.raises(ValueError):
        protobuf.dump_message(BytesIO(), msg)
