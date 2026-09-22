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

"""Host-side transcript and hostile-input tests for `trezorlib.zcash`.

The device is untrusted here. Every test drives the real `trezorlib` call
machinery over a scripted transport, asserts the exact sequence of messages the
host put on the wire, and -- for every hostile script -- asserts that no
partial result escapes and that the workflow is cancelled.

The PCZT fixtures are real donor bytes; see `fixtures/zcash/MANIFEST.json` for
provenance and limitations. The host never parses a PCZT, so these exercise the
transfer protocol and the signature-record response only, never signing.
"""

from __future__ import annotations

import json
import typing as t
from hashlib import sha256
from io import BytesIO
from pathlib import Path

import pytest

from trezorlib import exceptions, messages, protobuf, zcash
from trezorlib.client import AppManifest, Session, TrezorClient
from trezorlib.protobuf import MessageType

FIXTURE_DIR = Path(__file__).parent / "fixtures" / "zcash"
FIXTURE_MANIFEST = json.loads((FIXTURE_DIR / "MANIFEST.json").read_text())

MAINNET = messages.ZcashNetwork.Mainnet
TESTNET = messages.ZcashNetwork.Testnet
NETWORKS = [MAINNET, TESTNET]

# Account 0 and a nonzero account. Deliberately not 9: the donor hardcoded
# account 9, and no production path may inherit that.
ACCOUNTS = [0, 3]

TRANSFER_ID = bytes(range(16))
OTHER_TRANSFER_ID = bytes(range(16, 32))
REFERENCE_HEIGHT = FIXTURE_MANIFEST["corpus_height"]
DIVERSIFIER_INDEX = bytes(11)

# A 32-byte value with the fingerprint's shape (PUBLIC TEST VALUE).
SEED_FINGERPRINT = bytes(range(32))
VIEWING_KEYS = {
    MAINNET: "uview17j0q0nnczz63ducvkhe409f4r8sa2gx88unakv64k95dpe4r2hvn3lhe2gdfn00vsl830682a7tdhzwuhtsw2dp7usgxzdgqxujgu4pv50xrhuakfuk294xjcuhrs5ag0esenlp4wsawqmuqaaspykcplgk0vrds7fm0hrp3up2mmzgh7rdfhycgu2xp8",
    TESTNET: "uviewtest1frzzf669pvkxdjsgwf6y43tvuulek5l3fujvjsfrddrqs07mvpmaa2tua4jhdw4n3ekkqdxq9zgl53r8axe6l3sdzddlwuv3fz6tkyzv4xfkpfmkuevv2q46sapk5d3lhp7m5te04k7ulpv9j3sa08w7akay2xlpj68ly3355l0pgcydz3kvu5c335ggc",
}
# Generated independently with published orchard 0.15.3 FullViewingKey and
# zcash_address 0.13.0 Ufvk APIs from the public seed bytes 00..1f.

CHUNK = zcash.CHUNK_BYTES
RECORD = zcash.RECORD_BYTES


# ====== Scripted transport ====== #


class _FakeTransport:
    def __enter__(self) -> "_FakeTransport":
        return self

    def __exit__(self, *args: object) -> None:
        pass

    def __str__(self) -> str:
        return "scripted"


class ScriptedClient(TrezorClient):
    """A client whose transport replays a fixed list of device messages.

    Everything above the transport is the real implementation, so `expect=`
    narrowing, `Failure` translation and `ButtonRequest` handling behave exactly
    as they do against a device.
    """

    def __init__(self, responses: t.Sequence[MessageType | BaseException]) -> None:
        super().__init__(
            app=AppManifest(app_name="trezorlib-zcash-tests"),
            transport=t.cast(t.Any, _FakeTransport()),
            model=None,
            mapping=None,
            pairing=t.cast(t.Any, None),
        )
        self.responses: list[MessageType | BaseException] = list(responses)
        self.sent: list[MessageType] = []

    def _write(self, session: t.Any, msg: MessageType) -> None:
        self.sent.append(msg)

    def _read(self, session: t.Any, timeout: float | None = None) -> MessageType:
        if not self.responses:
            raise AssertionError("host sent more messages than the script provides")
        response = self.responses.pop(0)
        if isinstance(response, BaseException):
            raise response
        return response

    def _get_any_session(self) -> t.Any:  # pragma: no cover - unused
        raise NotImplementedError

    def _get_session(self, **kwargs: t.Any) -> t.Any:  # pragma: no cover - unused
        raise NotImplementedError


def scripted(*responses: MessageType | BaseException) -> Session:
    return Session(ScriptedClient(responses), id=b"scripted")


def sent(session: Session) -> list[MessageType]:
    return t.cast(ScriptedClient, session.client).sent


def remaining(session: Session) -> list[MessageType]:
    return t.cast(list[MessageType], t.cast(ScriptedClient, session.client).responses)


def assert_cancelled(session: Session) -> None:
    """A host-detected violation must abort the device workflow, exactly once."""
    cancels = [m for m in sent(session) if isinstance(m, messages.Cancel)]
    assert len(cancels) == 1
    assert isinstance(sent(session)[-1], messages.Cancel)


def assert_no_cancel(session: Session) -> None:
    assert not any(isinstance(m, messages.Cancel) for m in sent(session))


def assert_violation(script: t.Sequence[MessageType], pczt: bytes) -> Session:
    """A hostile device script must abort the workflow and yield no result."""
    session = scripted(*script)
    with pytest.raises(exceptions.ProtocolError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)
    return session


# ====== Fixtures ====== #


def _load(name: str) -> bytes:
    record = next(f for f in FIXTURE_MANIFEST["fixtures"] if f["file"] == name)
    blob = (FIXTURE_DIR / name).read_bytes()
    assert len(blob) == record["byte_length"]
    assert sha256(blob).hexdigest() == record["sha256"]
    return blob


PCZT_BY_ACTIONS = {f["action_count"]: f["file"] for f in FIXTURE_MANIFEST["fixtures"]}


def test_fixture_integrity_matches_attested_manifest() -> None:
    """Detect fixture drift against the manifest committed beside the corpus."""
    assert sorted(PCZT_BY_ACTIONS) == [1, 2, 8]
    for record in FIXTURE_MANIFEST["fixtures"]:
        blob = _load(record["file"])
        expected_chunks = -(-len(blob) // CHUNK)
        assert record["chunks_at_1024"] == expected_chunks
    # The two-action fixture really has two actions. An earlier revision of the
    # (local, untracked) protocol contract note labelled it "one action"; the
    # donor corpus manifest disproves that, so the manifest is authoritative.
    two = next(f for f in FIXTURE_MANIFEST["fixtures"] if f["action_count"] == 2)
    assert two["byte_length"] == 2429
    assert two["real_input_count"] == 2


# ====== Transcript builders ====== #


def chunks(total: int) -> list[tuple[int, int]]:
    return [(off, min(CHUNK, total - off)) for off in range(0, total, CHUNK)]


def upload_requests(
    total: int, transfer_id: bytes = TRANSFER_ID
) -> list[messages.ZcashPcztRequest]:
    return [
        messages.ZcashPcztRequest(transfer_id=transfer_id, offset=off, length=length)
        for off, length in chunks(total)
    ]


def records_for(indices: t.Sequence[int], pool: int = zcash.POOL_IRONWOOD) -> bytes:
    """Device-format signature records, one per action index, in the given order."""
    return b"".join(
        bytes((pool, index)) + bytes(((index * 37 + 11) % 256,)) * 64
        for index in indices
    )


def signatures(
    records: bytes, transfer_id: bytes = TRANSFER_ID
) -> messages.ZcashSpendAuthSignatures:
    return messages.ZcashSpendAuthSignatures(transfer_id=transfer_id, records=records)


def expected_signatures(records: bytes) -> list[zcash.SpendAuthSignature]:
    return [
        zcash.SpendAuthSignature(
            records[start + 1], records[start + 2 : start + RECORD]
        )
        for start in range(0, len(records), RECORD)
    ]


def sign_script(
    pczt: bytes, records: bytes, transfer_id: bytes = TRANSFER_ID
) -> list[MessageType]:
    """The full device side of a successful signing workflow.

    The record response is terminal (`@end`): nothing follows it on the wire.
    """
    return [*upload_requests(len(pczt), transfer_id), signatures(records, transfer_id)]


def expected_host_messages(
    pczt: bytes,
    network: messages.ZcashNetwork = MAINNET,
    account: int = 0,
    transfer_id: bytes = TRANSFER_ID,
) -> list[MessageType]:
    out: list[MessageType] = [
        messages.ZcashSignPczt(
            network=network,
            account=account,
            pczt_length=len(pczt),
            host_reference_height=REFERENCE_HEIGHT,
        )
    ]
    out += [
        messages.ZcashPcztAck(
            transfer_id=transfer_id, offset=off, data=pczt[off : off + length]
        )
        for off, length in chunks(len(pczt))
    ]
    return out


# ====== get_address ====== #


@pytest.mark.parametrize("network", NETWORKS)
@pytest.mark.parametrize("account", ACCOUNTS)
def test_get_address(network: messages.ZcashNetwork, account: int) -> None:
    address = "u1example" if network is MAINNET else "utest1example"
    session = scripted(messages.ZcashAddress(address=address))
    assert zcash.get_address(session, network, account, DIVERSIFIER_INDEX) == address
    assert sent(session) == [
        messages.ZcashGetAddress(
            network=network, account=account, diversifier_index=DIVERSIFIER_INDEX
        )
    ]
    assert not remaining(session)


def test_get_address_wrong_response_type_cancels() -> None:
    session = scripted(
        messages.ZcashViewingKey(
            seed_fingerprint=SEED_FINGERPRINT, key="uview1example"
        ),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.get_address(session, MAINNET, 0, DIVERSIFIER_INDEX)
    assert_cancelled(session)
    assert not session.is_invalid


@pytest.mark.parametrize(
    "network, address",
    [
        (MAINNET, ""),
        (MAINNET, "utest1cross-network"),
        (TESTNET, "u1cross-network"),
        (TESTNET, b"utest1wrong-type"),
    ],
)
def test_get_address_rejects_invalid_or_cross_network_response(
    network: messages.ZcashNetwork, address: object
) -> None:
    session = scripted(
        messages.ZcashAddress(address=t.cast(str, address)),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash address"):
        zcash.get_address(session, network, 0, DIVERSIFIER_INDEX)
    assert_cancelled(session)
    assert not session.is_invalid


def test_get_address_has_no_unconfirmed_path() -> None:
    """There is deliberately no show_display switch to turn confirmation off."""
    assert not any(
        f.name == "show_display" for f in messages.ZcashGetAddress.FIELDS.values()
    )


@pytest.mark.parametrize(
    "diversifier", [b"", bytes(10), bytes(12), bytes(32), bytearray(11)]
)
def test_get_address_rejects_bad_diversifier(diversifier: object) -> None:
    session = scripted()
    with pytest.raises(ValueError):
        zcash.get_address(session, MAINNET, 0, t.cast(bytes, diversifier))
    assert sent(session) == []


# ====== get_viewing_key ====== #


@pytest.mark.parametrize("network", NETWORKS)
@pytest.mark.parametrize("account", ACCOUNTS)
def test_get_viewing_key(network: messages.ZcashNetwork, account: int) -> None:
    key = VIEWING_KEYS[network]
    session = scripted(messages.ZcashViewingKey(key=key))
    assert zcash.get_viewing_key(session, network, account) == key
    assert sent(session) == [
        messages.ZcashGetViewingKey(
            network=network, account=account, include_seed_fingerprint=False
        )
    ]
    assert not remaining(session)


@pytest.mark.parametrize("network", NETWORKS)
def test_export_viewing_key_returns_key_and_seed_fingerprint(
    network: messages.ZcashNetwork,
) -> None:
    key = VIEWING_KEYS[network]
    session = scripted(
        messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=key)
    )
    export = zcash.export_viewing_key(
        session, network, 3, include_seed_fingerprint=True
    )
    assert export == zcash.ViewingKeyExport(key, SEED_FINGERPRINT)
    assert export.seed_fingerprint == SEED_FINGERPRINT
    assert sent(session) == [
        messages.ZcashGetViewingKey(
            network=network, account=3, include_seed_fingerprint=True
        )
    ]
    assert not remaining(session)


@pytest.mark.parametrize("network", NETWORKS)
def test_export_viewing_key_does_not_ask_for_the_fingerprint_by_default(
    network: messages.ZcashNetwork,
) -> None:
    """The seed fingerprint identifies the seed, so it is opt-in (M2).

    A host that does not ask gets a response without field 2 and a `None`
    fingerprint, and the device shows only the account-scoped warning.
    """
    key = VIEWING_KEYS[network]
    session = scripted(messages.ZcashViewingKey(key=key))
    export = zcash.export_viewing_key(session, network, 3)
    assert export == zcash.ViewingKeyExport(key, None)
    request = sent(session)[0]
    assert isinstance(request, messages.ZcashGetViewingKey)
    assert request.include_seed_fingerprint is False
    assert not remaining(session)


def test_export_viewing_key_loads_a_response_without_the_fingerprint_field() -> None:
    """M1: firmware older than the field omits it; the response must still load.

    The wire bytes below are the whole `ZcashViewingKey` an image without
    `seed_fingerprint` emits: field 1 only. With the field `required` this
    raised `ValueError("Did not receive value for field seed_fingerprint")`
    and no such device could be talked to at all.
    """
    key = VIEWING_KEYS[MAINNET]
    buf = BytesIO()
    protobuf.dump_message(
        buf, messages.ZcashViewingKey(key=key, seed_fingerprint=SEED_FINGERPRINT)
    )
    new_style = buf.getvalue()
    # field 2, wire type 2 (0x12), length 32 (0x20), then the fingerprint.
    assert new_style.endswith(b"\x12\x20" + SEED_FINGERPRINT)
    encoded = new_style[: -(2 + len(SEED_FINGERPRINT))]
    loaded = protobuf.load_message(BytesIO(encoded), messages.ZcashViewingKey)
    assert loaded == messages.ZcashViewingKey(key=key)
    assert loaded.seed_fingerprint is None

    session = scripted(loaded)
    assert zcash.export_viewing_key(session, MAINNET, 0) == zcash.ViewingKeyExport(
        key, None
    )
    assert zcash.get_viewing_key(session := scripted(loaded), MAINNET, 0) == key
    assert not remaining(session)


def test_export_viewing_key_rejects_a_missing_fingerprint_it_asked_for() -> None:
    """Asking and not being answered is a device violation, not a `None`."""
    session = scripted(
        messages.ZcashViewingKey(key=VIEWING_KEYS[MAINNET]),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="seed fingerprint"):
        zcash.export_viewing_key(session, MAINNET, 0, include_seed_fingerprint=True)
    assert_cancelled(session)


@pytest.mark.parametrize("include", [0, 1, None, "yes"])
def test_export_viewing_key_rejects_a_non_bool_request_flag(include: object) -> None:
    session = scripted()
    with pytest.raises(ValueError):
        zcash.export_viewing_key(
            session, MAINNET, 0, include_seed_fingerprint=t.cast(bool, include)
        )
    assert sent(session) == []


@pytest.mark.parametrize("fingerprint", [b"", b"\x00" * 31, b"\x00" * 33])
def test_export_viewing_key_rejects_wrong_fingerprint_length(
    fingerprint: bytes,
) -> None:
    """A fingerprint that is not exactly 32 bytes is a device violation."""
    session = scripted(
        messages.ZcashViewingKey(
            seed_fingerprint=fingerprint, key=VIEWING_KEYS[MAINNET]
        ),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="seed fingerprint"):
        zcash.export_viewing_key(session, MAINNET, 0, include_seed_fingerprint=True)
    assert_cancelled(session)


def test_export_viewing_key_checks_an_unasked_fingerprint_too() -> None:
    """Not asking does not license a malformed one."""
    session = scripted(
        messages.ZcashViewingKey(
            seed_fingerprint=b"\x00" * 31, key=VIEWING_KEYS[MAINNET]
        ),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="seed fingerprint"):
        zcash.export_viewing_key(session, MAINNET, 0)
    assert_cancelled(session)


def test_viewing_key_export_has_no_full_selector() -> None:
    """v1 has exactly one product shape: the Orchard-only UFVK.

    `include_seed_fingerprint` is not a second shape: it adds a public seed
    identifier to the same key, behind its own warning.
    """
    assert set(f.name for f in messages.ZcashGetViewingKey.FIELDS.values()) == {
        "network",
        "account",
        "include_seed_fingerprint",
    }
    assert set(f.name for f in messages.ZcashViewingKey.FIELDS.values()) == {
        "key",
        "seed_fingerprint",
    }
    # Both new fields are optional: an older device omits the response field,
    # and an older host omits the request field (default off).
    assert not messages.ZcashGetViewingKey.FIELDS[3].required
    assert messages.ZcashGetViewingKey.FIELDS[3].default is False
    assert not messages.ZcashViewingKey.FIELDS[2].required


@pytest.mark.parametrize(
    "failure, expected",
    [
        (messages.FailureType.ActionCancelled, exceptions.Cancelled),
        (messages.FailureType.InvalidSession, exceptions.InvalidSessionError),
        (messages.FailureType.NotInitialized, exceptions.TrezorFailure),
        (messages.FailureType.ProcessError, exceptions.TrezorFailure),
    ],
)
def test_viewing_key_rejection_returns_no_key_material(
    failure: int, expected: type[Exception]
) -> None:
    session = scripted(messages.Failure(code=failure, message="nope"))
    with pytest.raises(expected):
        zcash.get_viewing_key(session, MAINNET, 0)


def test_viewing_key_wrong_response_type_returns_no_key_material() -> None:
    session = scripted(
        messages.ZcashAddress(address="u1example"),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.get_viewing_key(session, MAINNET, 0)
    assert_cancelled(session)
    assert not session.is_invalid


@pytest.mark.parametrize(
    "network, key",
    [
        (MAINNET, ""),
        (MAINNET, VIEWING_KEYS[TESTNET]),
        (TESTNET, VIEWING_KEYS[MAINNET]),
        (TESTNET, b"uviewtest1wrong-type"),
        (MAINNET, "uview1example"),
        (MAINNET, VIEWING_KEYS[MAINNET].upper()),
        (MAINNET, VIEWING_KEYS[MAINNET][:-1] + "q"),
        (MAINNET, VIEWING_KEYS[MAINNET] + "q"),
    ],
)
def test_get_viewing_key_rejects_invalid_or_cross_network_response(
    network: messages.ZcashNetwork, key: object
) -> None:
    session = scripted(
        messages.ZcashViewingKey(
            seed_fingerprint=SEED_FINGERPRINT, key=t.cast(str, key)
        ),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash viewing key"):
        zcash.get_viewing_key(session, network, 0)
    assert_cancelled(session)
    assert not session.is_invalid


def _encode_malformed_ufvk(hrp: str, payload: bytes) -> str:
    jumbled = bytearray(payload)
    zcash._f4jumble(jumbled, inverse=False)
    data = zcash._convert_bits(jumbled, 8, 5, pad=True)
    return zcash._bech32m_encode(hrp, data)


@pytest.mark.parametrize(
    "payload",
    [
        bytes((2, 96)) + bytes(96) + b"uview" + bytes(11),
        bytes((3, 95)) + bytes(96) + b"uview" + bytes(11),
        bytes((3, 96)) + bytes(96) + b"uview" + bytes(10) + b"x",
        bytes((3, 48)) + bytes(48) + bytes((3, 46)) + bytes(46) + b"uview" + bytes(11),
    ],
    ids=["wrong-typecode", "wrong-item-length", "wrong-padding", "two-items"],
)
def test_get_viewing_key_rejects_well_checksummed_non_orchard_shapes(
    payload: bytes,
) -> None:
    key = _encode_malformed_ufvk("uview", payload)
    assert len(key) == 195
    session = scripted(
        messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=key),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash viewing key"):
        zcash.get_viewing_key(session, MAINNET, 0)
    assert_cancelled(session)
    assert not session.is_invalid


def test_get_viewing_key_rejects_noncanonical_bit_conversion() -> None:
    hrp, data = zcash._bech32m_decode(VIEWING_KEYS[MAINNET])
    data[-1] |= 1  # the final three bits are required zero padding
    key = zcash._bech32m_encode(hrp, data)
    session = scripted(
        messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=key),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash viewing key"):
        zcash.get_viewing_key(session, MAINNET, 0)
    assert_cancelled(session)
    assert not session.is_invalid
    assert not remaining(session)


def test_get_viewing_key_rejects_noncanonical_compact_size() -> None:
    payload = b"\xfd\x03\x00\x60" + bytes(96) + b"uview" + bytes(11)
    key = _encode_malformed_ufvk("uview", payload)
    assert len(key) > 195
    session = scripted(
        messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=key),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash viewing key"):
        zcash.get_viewing_key(session, MAINNET, 0)
    assert_cancelled(session)
    assert not session.is_invalid
    assert not remaining(session)


def test_get_viewing_key_rejects_bech32_instead_of_bech32m() -> None:
    hrp, data = zcash._bech32m_decode(VIEWING_KEYS[MAINNET])
    values = zcash._bech32_hrp_expand(hrp) + data + [0] * 6
    checksum = zcash._bech32_polymod(values) ^ 1
    checksum_values = [(checksum >> (5 * (5 - index))) & 31 for index in range(6)]
    key = (
        hrp
        + "1"
        + "".join(zcash._BECH32_CHARSET[item] for item in data + checksum_values)
    )
    session = scripted(
        messages.ZcashViewingKey(seed_fingerprint=SEED_FINGERPRINT, key=key),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash viewing key"):
        zcash.get_viewing_key(session, MAINNET, 0)
    assert_cancelled(session)
    assert not session.is_invalid
    assert not remaining(session)


def test_ufvk_envelope_check_does_not_replace_orchard_key_parsing() -> None:
    key = _encode_malformed_ufvk(
        "uview", bytes((3, 96)) + bytes(96) + b"uview" + bytes(11)
    )
    assert zcash._has_canonical_orchard_ufvk_envelope(key, MAINNET)


def test_malformed_response_cancels_and_raises_protocol_error() -> None:
    session = scripted(
        ValueError("invalid protobuf payload"),
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Malformed Zcash response"):
        zcash.get_address(session, MAINNET, 0, DIVERSIFIER_INDEX)
    assert_cancelled(session)
    assert not session.is_invalid
    assert not remaining(session)


@pytest.mark.parametrize(
    "decode_error",
    [
        TypeError("invalid decoded field type"),
        KeyError(32_767),
        OSError("interrupted uvarint"),
        exceptions.ProtocolError("bad transport framing"),
    ],
    ids=["type", "unknown-wire-type", "truncated-varint", "bad-framing"],
)
def test_other_decode_failures_cancel_and_raise_protocol_error(
    decode_error: Exception,
) -> None:
    session = scripted(
        decode_error,
        messages.Failure(code=messages.FailureType.ActionCancelled),
    )
    with pytest.raises(exceptions.ProtocolError, match="Malformed Zcash response"):
        zcash.get_address(session, MAINNET, 0, DIVERSIFIER_INDEX)
    assert_cancelled(session)
    assert not session.is_invalid
    assert not remaining(session)


# ====== Shared caller-input validation ====== #


@pytest.mark.parametrize("account", [-1, 2**31, 2**32, "0", 1.0, True, None])
def test_bad_account_is_rejected_before_any_io(account: object) -> None:
    for call in (
        lambda s: zcash.get_address(
            s, MAINNET, t.cast(int, account), DIVERSIFIER_INDEX
        ),
        lambda s: zcash.get_viewing_key(s, MAINNET, t.cast(int, account)),
        lambda s: zcash.sign_pczt(
            s, b"x", MAINNET, t.cast(int, account), REFERENCE_HEIGHT
        ),
    ):
        session = scripted()
        with pytest.raises(ValueError):
            call(session)
        assert sent(session) == []


@pytest.mark.parametrize("network", [None, 2, 99, -1, "mainnet", True, 1.0])
def test_bad_network_is_rejected_before_any_io(network: object) -> None:
    for call in (
        lambda s: zcash.get_address(
            s, t.cast(messages.ZcashNetwork, network), 0, DIVERSIFIER_INDEX
        ),
        lambda s: zcash.get_viewing_key(s, t.cast(messages.ZcashNetwork, network), 0),
        lambda s: zcash.sign_pczt(
            s, b"x", t.cast(messages.ZcashNetwork, network), 0, REFERENCE_HEIGHT
        ),
    ):
        session = scripted()
        with pytest.raises(ValueError):
            call(session)
        assert sent(session) == []


@pytest.mark.parametrize("height", [-1, 2**32, "10", 1.5, None])
def test_bad_reference_height_is_rejected_before_any_io(height: object) -> None:
    session = scripted()
    with pytest.raises(ValueError):
        zcash.sign_pczt(session, b"x", MAINNET, 0, t.cast(int, height))
    assert sent(session) == []


@pytest.mark.parametrize(
    "pczt, expected",
    [
        (b"", ValueError),
        (bytes(zcash.MAX_PCZT_BYTES + 1), ValueError),
        (bytearray(b"x"), TypeError),
        (memoryview(b"x"), TypeError),
        ("x", TypeError),
        (None, TypeError),
    ],
    ids=["empty", "oversize", "bytearray", "memoryview", "str", "none"],
)
def test_bad_pczt_is_rejected_before_any_io(
    pczt: object, expected: type[Exception]
) -> None:
    session = scripted()
    with pytest.raises(expected):
        zcash.sign_pczt(session, t.cast(bytes, pczt), MAINNET, 0, REFERENCE_HEIGHT)
    assert sent(session) == []


# ====== sign_pczt happy paths ====== #


@pytest.mark.parametrize("actions", [1, 2, 8])
@pytest.mark.parametrize("network", NETWORKS)
def test_sign_pczt_transcript(actions: int, network: messages.ZcashNetwork) -> None:
    pczt = _load(PCZT_BY_ACTIONS[actions])
    # One record per action; the host never checks records against the PCZT.
    records = records_for(range(actions))
    session = scripted(*sign_script(pczt, records))

    result = zcash.sign_pczt(session, pczt, network, 3, REFERENCE_HEIGHT)
    assert result == expected_signatures(records)
    assert [r.action_index for r in result] == list(range(actions))
    assert sent(session) == expected_host_messages(pczt, network=network, account=3)
    assert not remaining(session)
    assert_no_cancel(session)


def test_sign_pczt_chunk_counts_match_the_manifest() -> None:
    for record in FIXTURE_MANIFEST["fixtures"]:
        pczt = _load(record["file"])
        session = scripted(*sign_script(pczt, records_for([0])))
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
        acks = [m for m in sent(session) if isinstance(m, messages.ZcashPcztAck)]
        assert len(acks) == record["chunks_at_1024"]


@pytest.mark.parametrize(
    "indices",
    [[0], [3], [0, 1], [0, 5, 31], [31], list(range(zcash.MAX_ACTIONS))],
    ids=["single", "nonzero", "pair", "sparse", "last-index", "max-count"],
)
def test_sign_pczt_accepts_every_admissible_record_set(indices: list[int]) -> None:
    """Records are one per real spend, ascending; dummies simply have none."""
    pczt = _load(PCZT_BY_ACTIONS[2])
    records = records_for(indices)
    session = scripted(*sign_script(pczt, records))
    result = zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert result == expected_signatures(records)
    assert all(len(r.signature) == 64 for r in result)
    assert not remaining(session)
    assert_no_cancel(session)


def test_sign_pczt_ignores_measurement_trailer() -> None:
    """The optional debug trailer never changes the parsed result."""
    pczt = _load(PCZT_BY_ACTIONS[1])
    records = records_for([0])
    response = messages.ZcashSpendAuthSignatures(
        transfer_id=TRANSFER_ID, records=records, debug_timings=b"derive=1"
    )
    session = scripted(*upload_requests(len(pczt)), response)
    result = zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert result == expected_signatures(records)
    assert zcash.last_debug_timings == b"derive=1"


@pytest.mark.parametrize(
    "total", [1, CHUNK - 1, CHUNK, CHUNK + 1, zcash.MAX_PCZT_BYTES]
)
def test_sign_pczt_boundary_lengths(total: int) -> None:
    pczt = bytes((i * 7 + 1) % 256 for i in range(total))
    records = records_for([0])
    session = scripted(*sign_script(pczt, records))
    result = zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert result == expected_signatures(records)
    assert sent(session) == expected_host_messages(pczt)


# ====== sign_pczt: hostile upload ====== #


@pytest.mark.parametrize(
    "transfer_id",
    [b"", bytes(15), bytes(17), bytes(32)],
    ids=lambda b: f"{len(b)}B",
)
def test_upload_rejects_bad_transfer_id_length(transfer_id: bytes) -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    script: t.Sequence[MessageType] = [
        messages.ZcashPcztRequest(transfer_id=transfer_id, offset=0, length=CHUNK)
    ]
    session = assert_violation(script, pczt)
    # Nothing but the starter and the cancel went out.
    assert not any(isinstance(m, messages.ZcashPcztAck) for m in sent(session))


def test_upload_rejects_changed_transfer_id() -> None:
    pczt = _load(PCZT_BY_ACTIONS[2])
    requests = upload_requests(len(pczt))
    requests[1] = messages.ZcashPcztRequest(
        transfer_id=OTHER_TRANSFER_ID, offset=requests[1].offset, length=CHUNK
    )
    assert_violation((requests), pczt)


@pytest.mark.parametrize("bad_offset", [1, CHUNK, 2 * CHUNK, 2**31])
def test_upload_rejects_wrong_first_offset(bad_offset: int) -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    script: t.Sequence[MessageType] = [
        messages.ZcashPcztRequest(
            transfer_id=TRANSFER_ID, offset=bad_offset, length=CHUNK
        )
    ]
    assert_violation(script, pczt)


def test_upload_rejects_repeated_offset() -> None:
    pczt = _load(PCZT_BY_ACTIONS[2])
    requests = upload_requests(len(pczt))
    requests[1] = requests[0]  # replay the first request
    assert_violation((requests), pczt)


def test_upload_rejects_backwards_offset() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    requests = upload_requests(len(pczt))
    requests[3] = messages.ZcashPcztRequest(
        transfer_id=TRANSFER_ID, offset=CHUNK, length=CHUNK
    )
    assert_violation((requests), pczt)


def test_upload_rejects_skipped_offset() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    requests = upload_requests(len(pczt))
    del requests[2]
    assert_violation((requests), pczt)


@pytest.mark.parametrize("length", [0, 1, CHUNK - 1, CHUNK + 1, 2**31])
def test_upload_rejects_wrong_requested_length(length: int) -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    script: t.Sequence[MessageType] = [
        messages.ZcashPcztRequest(transfer_id=TRANSFER_ID, offset=0, length=length)
    ]
    assert_violation(script, pczt)


def test_upload_rejects_overlong_final_length() -> None:
    """The device must not be able to pull past the declared total."""
    pczt = _load(PCZT_BY_ACTIONS[1])
    requests = upload_requests(len(pczt))
    requests[-1] = messages.ZcashPcztRequest(
        transfer_id=TRANSFER_ID, offset=requests[-1].offset, length=CHUNK
    )
    assert_violation((requests), pczt)


def test_upload_rejects_unexpected_message_class() -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    session = scripted(messages.ZcashAddress(address="u1example"))
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)


def test_upload_rejects_premature_success() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt))[:2],
        messages.Success(),
    ]
    session = scripted(*script)
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)


def test_upload_rejects_signatures_before_upload_completes() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    script: t.Sequence[MessageType] = [
        upload_requests(len(pczt))[0],
        signatures(records_for([0])),
    ]
    session = scripted(*script)
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)


@pytest.mark.parametrize(
    "failure, expected",
    [
        (messages.FailureType.ActionCancelled, exceptions.Cancelled),
        (messages.FailureType.DataError, exceptions.TrezorFailure),
        (messages.FailureType.ProcessError, exceptions.TrezorFailure),
        (messages.FailureType.UnexpectedMessage, exceptions.TrezorFailure),
        (messages.FailureType.InvalidSession, exceptions.InvalidSessionError),
        (messages.FailureType.FirmwareError, exceptions.TrezorFailure),
    ],
)
def test_failure_during_upload_yields_no_result(
    failure: int, expected: type[Exception]
) -> None:
    pczt = _load(PCZT_BY_ACTIONS[2])
    script: t.Sequence[MessageType] = [
        upload_requests(len(pczt))[0],
        messages.Failure(code=failure, message="unstable text, not an oracle"),
    ]
    session = scripted(*script)
    with pytest.raises(expected):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    # The device already terminated; the host must not pile on a Cancel.
    assert_no_cancel(session)


# ====== sign_pczt: hostile signature response ====== #


def assert_records_rejected(records: bytes) -> Session:
    """A malformed record blob must yield no result.

    The device workflow is already complete when the blob arrives, so there is
    nothing to cancel: the host must raise without sending a Cancel.
    """
    pczt = _load(PCZT_BY_ACTIONS[2])
    session = scripted(*sign_script(pczt, records))
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash signature"):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_no_cancel(session)
    assert not remaining(session)
    return session


@pytest.mark.parametrize(
    "records",
    [
        b"",
        bytes(RECORD - 1),
        bytes(RECORD + 1),
        records_for([0]) + b"\x00",
        records_for([0])[:-1],
        records_for(range(zcash.MAX_ACTIONS + 1)),
    ],
    ids=["empty", "short", "long", "trailing-byte", "truncated", "too-many"],
)
def test_signatures_reject_bad_blob_length(records: bytes) -> None:
    assert_records_rejected(records)


@pytest.mark.parametrize("pool", [0x00, 0x01, 0x02, 0x04, 0xFF])
def test_signatures_reject_wrong_pool(pool: int) -> None:
    assert_records_rejected(records_for([0], pool=pool))


def test_signatures_reject_wrong_pool_in_any_position() -> None:
    good, bad = records_for([0]), records_for([1], pool=0x02)
    assert_records_rejected(good + bad)
    assert_records_rejected(bad + good)


@pytest.mark.parametrize(
    "indices",
    [[1, 0], [0, 0], [0, 2, 1], [3, 3, 4], [5, 4, 6]],
    ids=["descending", "repeated", "swap-tail", "repeated-tail", "dip"],
)
def test_signatures_reject_non_ascending_index(indices: list[int]) -> None:
    assert_records_rejected(records_for(indices))


@pytest.mark.parametrize("index", [zcash.MAX_ACTIONS, zcash.MAX_ACTIONS + 1, 255])
def test_signatures_reject_out_of_range_index(index: int) -> None:
    assert_records_rejected(records_for([index]))
    assert_records_rejected(records_for([0, index]))


def test_signatures_reject_changed_transfer_id() -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    session = scripted(
        *upload_requests(len(pczt), TRANSFER_ID),
        signatures(records_for([0]), OTHER_TRANSFER_ID),
    )
    with pytest.raises(exceptions.ProtocolError, match="Changed transfer ID"):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_no_cancel(session)


def test_signatures_reject_non_bytes_records() -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    session = scripted(
        *upload_requests(len(pczt)),
        messages.ZcashSpendAuthSignatures(
            transfer_id=TRANSFER_ID,
            records=t.cast(bytes, records_for([0]).decode("latin-1")),
        ),
    )
    with pytest.raises(exceptions.ProtocolError, match="Invalid Zcash signature"):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_no_cancel(session)


@pytest.mark.parametrize(
    "response",
    [
        messages.Success(),
        messages.ZcashAddress(address="u1example"),
        messages.ZcashPcztRequest(transfer_id=TRANSFER_ID, offset=0, length=CHUNK),
    ],
    ids=["success", "address", "extra-request"],
)
def test_completed_upload_accepts_only_signatures(response: MessageType) -> None:
    """Once the last chunk is served, records are the only admissible answer."""
    pczt = _load(PCZT_BY_ACTIONS[8])
    session = scripted(*upload_requests(len(pczt)), response)
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)


@pytest.mark.parametrize(
    "failure, expected",
    [
        (messages.FailureType.ActionCancelled, exceptions.Cancelled),
        (messages.FailureType.FirmwareError, exceptions.TrezorFailure),
        (messages.FailureType.InvalidSession, exceptions.InvalidSessionError),
    ],
)
def test_failure_in_place_of_signatures_yields_no_result(
    failure: int, expected: type[Exception]
) -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    session = scripted(*upload_requests(len(pczt)), messages.Failure(code=failure))
    with pytest.raises(expected):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_no_cancel(session)


# ====== Cross-transfer confusion ====== #


def test_second_transfer_rejects_first_transfer_id() -> None:
    """A response minted for one transfer must not be usable in another."""
    pczt = _load(PCZT_BY_ACTIONS[2])
    records = records_for([0, 1])
    first = scripted(*sign_script(pczt, records, TRANSFER_ID))
    result = zcash.sign_pczt(first, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert result == expected_signatures(records)

    stale = upload_requests(len(pczt), OTHER_TRANSFER_ID)
    stale[1] = messages.ZcashPcztRequest(
        transfer_id=TRANSFER_ID, offset=stale[1].offset, length=CHUNK
    )
    second = scripted(*(stale))
    with pytest.raises(exceptions.ProtocolError):
        zcash.sign_pczt(second, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(second)


def test_host_never_reuses_a_transfer_id_across_workflows() -> None:
    """The host echoes only the ID the device minted for this workflow."""
    pczt = _load(PCZT_BY_ACTIONS[2])
    for transfer_id in (TRANSFER_ID, OTHER_TRANSFER_ID):
        session = scripted(*sign_script(pczt, records_for([0]), transfer_id))
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
        ids = {
            m.transfer_id for m in sent(session) if isinstance(m, messages.ZcashPcztAck)
        }
        assert ids == {transfer_id}


# ====== No retry, no resume, no partial result ====== #


def test_violation_stops_the_workflow_immediately() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    requests = upload_requests(len(pczt))
    requests[2] = messages.ZcashPcztRequest(
        transfer_id=TRANSFER_ID, offset=0, length=CHUNK
    )
    session = assert_violation((requests), pczt)
    acks = [m for m in sent(session) if isinstance(m, messages.ZcashPcztAck)]
    # Two chunks were served before the bad request; nothing after it.
    assert [m.offset for m in acks] == [0, CHUNK]
    assert remaining(session), "the host stopped reading, as required"


# ====== Session stays synchronized after a cancel ====== #


class OrderedWireClient(TrezorClient):
    """A client backed by a real request-ordered wire, not a fixed reply list.

    `ScriptedClient` pops replies regardless of what was written, so it cannot
    show a desynchronized session. This one answers each written message from a
    handler, which makes "the next call read the previous call's reply" visible.
    """

    def __init__(self, handler: t.Callable[[MessageType], MessageType]) -> None:
        super().__init__(
            app=AppManifest(app_name="trezorlib-zcash-tests"),
            transport=t.cast(t.Any, _FakeTransport()),
            model=None,
            mapping=None,
            pairing=t.cast(t.Any, None),
        )
        self.handler = handler
        self.queue: list[MessageType] = []

    def _write(self, session: t.Any, msg: MessageType) -> None:
        self.queue.append(self.handler(msg))

    def _read(self, session: t.Any, timeout: float | None = None) -> MessageType:
        if not self.queue:
            raise AssertionError("host read without writing")
        return self.queue.pop(0)

    def _get_any_session(self) -> t.Any:  # pragma: no cover - unused
        raise NotImplementedError

    def _get_session(self, **kwargs: t.Any) -> t.Any:  # pragma: no cover - unused
        raise NotImplementedError


def test_cancel_after_violation_keeps_the_session_synchronized() -> None:
    """A violation must not leave a reply queued for the next caller.

    Regression test. `Session.cancel()` is write-only, so cancelling that way
    strands the device's `Failure(ActionCancelled)` on the wire; the next call
    then reads the *previous* request's reply and `get_address` hands back the
    confirmed address of an account nobody asked for.
    """

    def device(msg: MessageType) -> MessageType:
        if isinstance(msg, messages.ZcashSignPczt):
            # The single violation: the device asks for the wrong offset.
            return messages.ZcashPcztRequest(
                transfer_id=TRANSFER_ID, offset=7, length=CHUNK
            )
        if isinstance(msg, messages.ZcashGetAddress):
            return messages.ZcashAddress(address=f"u1-account-{msg.account}")
        if isinstance(msg, messages.Cancel):
            return messages.Failure(code=messages.FailureType.ActionCancelled)
        raise AssertionError(f"unexpected host message {msg}")

    session = Session(OrderedWireClient(device), id=b"ordered")

    with pytest.raises(exceptions.ProtocolError):
        zcash.sign_pczt(
            session, _load(PCZT_BY_ACTIONS[1]), MAINNET, 0, REFERENCE_HEIGHT
        )

    # Each later call must receive its own answer, not the previous one's.
    for account in (1, 2, 3):
        assert (
            zcash.get_address(session, MAINNET, account, DIVERSIFIER_INDEX)
            == f"u1-account-{account}"
        )


def test_cancel_after_wrong_message_class_keeps_the_session_synchronized() -> None:
    """Same guarantee on the `_expect` path, which also cancels."""

    def device(msg: MessageType) -> MessageType:
        if isinstance(msg, messages.ZcashGetViewingKey):
            return messages.ZcashAddress(address="u1wrong-class")
        if isinstance(msg, messages.ZcashGetAddress):
            return messages.ZcashAddress(address=f"u1-account-{msg.account}")
        if isinstance(msg, messages.Cancel):
            return messages.Failure(code=messages.FailureType.ActionCancelled)
        raise AssertionError(f"unexpected host message {msg}")

    session = Session(OrderedWireClient(device), id=b"ordered")

    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.get_viewing_key(session, MAINNET, 0)

    assert zcash.get_address(session, MAINNET, 5, DIVERSIFIER_INDEX) == "u1-account-5"
