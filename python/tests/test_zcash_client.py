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
transfer protocol only, never signing.
"""

from __future__ import annotations

import json
import typing as t
from hashlib import sha256
from pathlib import Path

import pytest

from trezorlib import exceptions, messages, zcash
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

VIEWING_KEYS = {
    MAINNET: "uview17j0q0nnczz63ducvkhe409f4r8sa2gx88unakv64k95dpe4r2hvn3lhe2gdfn00vsl830682a7tdhzwuhtsw2dp7usgxzdgqxujgu4pv50xrhuakfuk294xjcuhrs5ag0esenlp4wsawqmuqaaspykcplgk0vrds7fm0hrp3up2mmzgh7rdfhycgu2xp8",
    TESTNET: "uviewtest1frzzf669pvkxdjsgwf6y43tvuulek5l3fujvjsfrddrqs07mvpmaa2tua4jhdw4n3ekkqdxq9zgl53r8axe6l3sdzddlwuv3fz6tkyzv4xfkpfmkuevv2q46sapk5d3lhp7m5te04k7ulpv9j3sa08w7akay2xlpj68ly3355l0pgcydz3kvu5c335ggc",
}
# Generated independently with published orchard 0.15.3 FullViewingKey and
# zcash_address 0.13.0 Ufvk APIs from the public seed bytes 00..1f.

CHUNK = zcash.CHUNK_BYTES


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


def download_chunks(
    signed: bytes, transfer_id: bytes = TRANSFER_ID
) -> list[messages.ZcashSignedPczt]:
    return [
        messages.ZcashSignedPczt(
            transfer_id=transfer_id,
            pczt_length=len(signed),
            offset=off,
            data=signed[off : off + length],
        )
        for off, length in chunks(len(signed))
    ]


def sign_script(
    pczt: bytes, signed: bytes, transfer_id: bytes = TRANSFER_ID
) -> list[MessageType]:
    """The full device side of a successful signing workflow."""
    return [
        *upload_requests(len(pczt), transfer_id),
        *download_chunks(signed, transfer_id),
        messages.Success(),
    ]


def expected_host_messages(
    pczt: bytes,
    signed: bytes,
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
    out += [
        messages.ZcashSignedPcztAck(transfer_id=transfer_id, next_offset=off + length)
        for off, length in chunks(len(signed))
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
        messages.ZcashViewingKey(key="uview1example"),
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
        messages.ZcashGetViewingKey(network=network, account=account)
    ]
    assert not remaining(session)


def test_viewing_key_export_has_no_full_selector() -> None:
    """v1 has exactly one product shape: the Orchard-only UFVK."""
    assert set(f.name for f in messages.ZcashGetViewingKey.FIELDS.values()) == {
        "network",
        "account",
    }
    assert set(f.name for f in messages.ZcashViewingKey.FIELDS.values()) == {"key"}


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
        messages.ZcashViewingKey(key=t.cast(str, key)),
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
        messages.ZcashViewingKey(key=key),
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
        messages.ZcashViewingKey(key=key),
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
        messages.ZcashViewingKey(key=key),
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
        messages.ZcashViewingKey(key=key),
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
    signed = pczt  # this lane cannot produce a signed PCZT; see the manifest
    session = scripted(*sign_script(pczt, signed))

    assert zcash.sign_pczt(session, pczt, network, 3, REFERENCE_HEIGHT) == signed
    assert sent(session) == expected_host_messages(
        pczt, signed, network=network, account=3
    )
    assert not remaining(session)
    assert_no_cancel(session)


def test_sign_pczt_chunk_counts_match_the_manifest() -> None:
    for record in FIXTURE_MANIFEST["fixtures"]:
        pczt = _load(record["file"])
        session = scripted(*sign_script(pczt, pczt))
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
        acks = [m for m in sent(session) if isinstance(m, messages.ZcashPcztAck)]
        assert len(acks) == record["chunks_at_1024"]


def test_sign_pczt_transfer_accepts_device_chosen_payload_length() -> None:
    """The transfer layer does not interpret or bind the returned PCZT bytes."""
    pczt = _load(PCZT_BY_ACTIONS[2])
    signed = pczt + b"\x00" * 64
    session = scripted(*sign_script(pczt, signed))
    assert zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT) == signed


@pytest.mark.parametrize(
    "total", [1, CHUNK - 1, CHUNK, CHUNK + 1, zcash.MAX_PCZT_BYTES]
)
def test_sign_pczt_boundary_lengths(total: int) -> None:
    pczt = bytes((i * 7 + 1) % 256 for i in range(total))
    session = scripted(*sign_script(pczt, pczt))
    assert zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT) == pczt


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


def test_upload_rejects_signed_chunk_before_upload_completes() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    script: t.Sequence[MessageType] = [
        upload_requests(len(pczt))[0],
        *download_chunks(pczt),
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


# ====== sign_pczt: hostile download ====== #


@pytest.mark.parametrize("total", [0, zcash.MAX_PCZT_BYTES + 1, 2**31])
def test_download_rejects_bad_total_length(total: int) -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt)),
        messages.ZcashSignedPczt(
            transfer_id=TRANSFER_ID, pczt_length=total, offset=0, data=b"\x00" * CHUNK
        ),
    ]
    assert_violation(script, pczt)


def test_download_rejects_changed_total_length() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    signed = pczt
    chunk_list = download_chunks(signed)
    chunk_list[2] = messages.ZcashSignedPczt(
        transfer_id=TRANSFER_ID,
        pczt_length=len(signed) + CHUNK,
        offset=chunk_list[2].offset,
        data=chunk_list[2].data,
    )
    script: t.Sequence[MessageType] = [*upload_requests(len(pczt)), *chunk_list]
    assert_violation(script, pczt)


def test_download_rejects_changed_transfer_id() -> None:
    pczt = _load(PCZT_BY_ACTIONS[2])
    chunk_list = download_chunks(pczt)
    chunk_list[1] = messages.ZcashSignedPczt(
        transfer_id=OTHER_TRANSFER_ID,
        pczt_length=len(pczt),
        offset=chunk_list[1].offset,
        data=chunk_list[1].data,
    )
    script: t.Sequence[MessageType] = [*upload_requests(len(pczt)), *chunk_list]
    assert_violation(script, pczt)


@pytest.mark.parametrize("bad_offset", [1, CHUNK, 2**31])
def test_download_rejects_wrong_first_offset(bad_offset: int) -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt)),
        messages.ZcashSignedPczt(
            transfer_id=TRANSFER_ID,
            pczt_length=len(pczt),
            offset=bad_offset,
            data=b"\x00" * CHUNK,
        ),
    ]
    assert_violation(script, pczt)


def test_download_rejects_repeated_chunk() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    chunk_list = download_chunks(pczt)
    chunk_list[4] = chunk_list[3]
    script: t.Sequence[MessageType] = [*upload_requests(len(pczt)), *chunk_list]
    assert_violation(script, pczt)


def test_download_rejects_repeated_final_chunk() -> None:
    pczt = _load(PCZT_BY_ACTIONS[2])
    chunk_list = download_chunks(pczt)
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt)),
        *chunk_list,
        chunk_list[-1],
    ]
    session = scripted(*script)
    # The transfer is complete, so anything but Success is a violation.
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)


@pytest.mark.parametrize("delta", [-1, 1], ids=["short", "long"])
def test_download_rejects_wrong_chunk_size(delta: int) -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    chunk_list = download_chunks(pczt)
    first = chunk_list[0]
    data = first.data[:-1] if delta < 0 else first.data + b"\x00"
    chunk_list[0] = messages.ZcashSignedPczt(
        transfer_id=TRANSFER_ID,
        pczt_length=len(pczt),
        offset=0,
        data=data,
    )
    script: t.Sequence[MessageType] = [*upload_requests(len(pczt)), *chunk_list]
    assert_violation(script, pczt)


def test_download_rejects_short_final_chunk() -> None:
    pczt = _load(PCZT_BY_ACTIONS[2])
    chunk_list = download_chunks(pczt)
    last = chunk_list[-1]
    chunk_list[-1] = messages.ZcashSignedPczt(
        transfer_id=TRANSFER_ID,
        pczt_length=len(pczt),
        offset=last.offset,
        data=last.data[:-1],
    )
    script: t.Sequence[MessageType] = [*upload_requests(len(pczt)), *chunk_list]
    assert_violation(script, pczt)


def test_download_rejects_premature_success() -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt)),
        *download_chunks(pczt)[:3],
        messages.Success(),
    ]
    session = scripted(*script)
    with pytest.raises(exceptions.UnexpectedMessageError):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(session)


def test_download_requires_success_to_terminate() -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt)),
        *download_chunks(pczt),
        messages.ZcashAddress(address="u1example"),
    ]
    session = scripted(*script)
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
def test_failure_during_download_yields_no_result(
    failure: int, expected: type[Exception]
) -> None:
    pczt = _load(PCZT_BY_ACTIONS[8])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt)),
        *download_chunks(pczt)[:2],
        messages.Failure(code=failure),
    ]
    session = scripted(*script)
    with pytest.raises(expected):
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_no_cancel(session)


# ====== Cross-transfer confusion ====== #


def test_second_transfer_rejects_first_transfer_id() -> None:
    """A response minted for one transfer must not be usable in another."""
    pczt = _load(PCZT_BY_ACTIONS[2])
    first = scripted(*sign_script(pczt, pczt, TRANSFER_ID))
    assert zcash.sign_pczt(first, pczt, MAINNET, 0, REFERENCE_HEIGHT) == pczt

    stale = upload_requests(len(pczt), OTHER_TRANSFER_ID)
    stale[1] = messages.ZcashPcztRequest(
        transfer_id=TRANSFER_ID, offset=stale[1].offset, length=CHUNK
    )
    second = scripted(*(stale))
    with pytest.raises(exceptions.ProtocolError):
        zcash.sign_pczt(second, pczt, MAINNET, 0, REFERENCE_HEIGHT)
    assert_cancelled(second)


def test_download_rejects_id_from_a_different_transfer() -> None:
    pczt = _load(PCZT_BY_ACTIONS[1])
    script: t.Sequence[MessageType] = [
        *upload_requests(len(pczt), TRANSFER_ID),
        *download_chunks(pczt, OTHER_TRANSFER_ID),
    ]
    assert_violation(script, pczt)


def test_host_never_reuses_a_transfer_id_across_workflows() -> None:
    """The host echoes only the ID the device minted for this workflow."""
    pczt = _load(PCZT_BY_ACTIONS[2])
    for transfer_id in (TRANSFER_ID, OTHER_TRANSFER_ID):
        session = scripted(*sign_script(pczt, pczt, transfer_id))
        zcash.sign_pczt(session, pczt, MAINNET, 0, REFERENCE_HEIGHT)
        ids = {
            m.transfer_id
            for m in sent(session)
            if isinstance(m, (messages.ZcashPcztAck, messages.ZcashSignedPcztAck))
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
