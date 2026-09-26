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

"""Streamed PCZT signing on the emulator.

The PCZTs are the firmware series' checked-in vectors
(bawolf/trezor-firmware@7864a22444, `common/tests/fixtures/zcash/`), copied to
`fixtures/` unchanged and built for the "abandon ... about" wallet. Each vector
carries the hex PCZT, the network, account and host height the device is asked
for, and what the device must show and return: the payments, the totals, the
memo classification, the viewing key, the seed fingerprint, and the action
indices that must come back signed.

The signatures themselves are not re-verified here -- a hedged nonce makes them
non-deterministic, and host-side RedPallas verification is not something
trezorlib does. The cryptographic argument is `cargo test -p ironwood`, whose
equivalence and conformance suites apply every signature with the `pczt` crate's
signer against the action's `rk` and the host-computed sighash.

Every flow starts with Core's one-time request to let this app instance use
the account, then the ZIP-315 warning of the 12-word test wallet.
"""

import io
import json
from pathlib import Path

import pytest

from trezorlib import exceptions, messages, protobuf
from trezorlib.debuglink import DebugSession as Session
from trezorlib.exceptions import Cancelled, TrezorFailure

from . import zcash_ext
from .common import (
    MNEMONIC,
    accept_account_request,
    address_pieces,
    no_screens,
    screen_text,
)
from .generated.messages import (
    MessageType,
    ZcashNetwork,
    ZcashPcztAck,
    ZcashPcztRequest,
    ZcashSignPczt,
)

B = messages.ButtonRequestType

pytestmark = [pytest.mark.setup_client(mnemonic=MNEMONIC)]

FIXTURES = Path(__file__).resolve().parent / "fixtures"
NETWORKS = {"mainnet": ZcashNetwork.Mainnet, "testnet": ZcashNetwork.Testnet}


def _fixture(name: str) -> list[dict]:
    fixture = json.loads((FIXTURES / name).read_text())
    assert fixture["setup"] == {"mnemonic": MNEMONIC, "passphrase": ""}
    return fixture["tests"]


def parametrize_using_fixture(name: str):
    return pytest.mark.parametrize(
        "parameters, result",
        [
            pytest.param(test["parameters"], test["result"], id=test["name"])
            for test in _fixture(name)
        ],
    )


def vector(name: str) -> tuple[dict, dict]:
    """One signing vector by name, for flows parametrization cannot express."""
    for path in (
        "sign_pczt.json",
        "sign_pczt.memos.json",
        "sign_pczt.transparent.json",
    ):
        for test in _fixture(path):
            if test["name"] == name:
                return test["parameters"], test["result"]
    raise KeyError(name)


def with_user_address(pczt: bytes, old: str, new: str) -> bytes:
    """`pczt` with one output's `user_address` replaced. Postcard encodes the
    `Some(String)` as 0x01, the byte length as a varint, then the bytes."""

    def field(text: str) -> bytes:
        raw = text.encode()
        length, varint = len(raw), bytearray()
        while True:
            varint.append(length & 0x7F | (0x80 if length >= 0x80 else 0))
            length >>= 7
            if not length:
                return b"\x01" + varint + raw

    assert pczt.count(field(old)) == 1
    return pczt.replace(field(old), field(new))


def _sign(session: Session, instance_id: int, parameters: dict, flow) -> list:
    with session.test_ctx as client:
        client.set_input_flow(flow)
        return zcash_ext.sign_pczt(
            session,
            instance_id,
            bytes.fromhex(parameters["pczt"]),
            NETWORKS[parameters["network"]],
            parameters["account"],
            parameters["height"],
        )


# On T3W1 (eckhart) each payment is two ConfirmOutput screens: the address,
# then the amount.
OUTPUT_SCREENS = 2


def _accept_outputs(session: Session, payments: int):
    for _ in range(payments * OUTPUT_SCREENS):
        br = yield
        assert (br.code, br.name) == (B.ConfirmOutput, "confirm_output")
        session.debug.press_yes()


def _accept_start(session: Session, first_use: bool = True):
    if first_use:
        yield from accept_account_request(session)
    # 12-word wallet: the ZIP-315 weak-backup warning comes next.
    br = yield
    assert (br.code, br.name) == (B.Warning, "zcash_weak_backup")
    session.debug.press_yes()


def _accept_total(session: Session):
    br = yield
    assert (br.code, br.name) == (B.SignTx, "confirm_total")
    session.debug.press_yes()


def _accept_flow(session: Session, payments: int, first_use: bool = True):
    yield from _accept_start(session, first_use)
    yield from _accept_outputs(session, payments)
    yield from _accept_total(session)


@parametrize_using_fixture("sign_pczt.json")
def test_streamed_sign(
    session: Session, instance_id: int, parameters: dict, result: dict
) -> None:
    """Every vector streams, reviews, and returns one record per real spend.

    The four action counts cover the bundle sizes the wire admits; `view=full`
    and `view=sdk` are what a stock-SDK wallet hands over unmodified (the empty
    Sapling bundle keeps its anchor and bsk, the Ironwood bsk stays, and `sdk`
    adds `OvkPolicy::Sender` plus the recipient string stamped on every payment
    as `user_address`); `zip32=own` attaches the claim the SDK makes for an
    account imported as `Spending { seed_fingerprint, index }`.
    """
    assert len(bytes.fromhex(parameters["pczt"])) == result["pczt_length"]
    payments = len(result["payments"])

    signatures = _sign(
        session, instance_id, parameters, _accept_flow(session, payments)
    )

    # The builder shuffles action order, so which action carries the real spend
    # is a property of the vector, not of its size.
    assert [s.action_index for s in signatures] == result["real_spend_actions"]
    assert all(len(s.signature) == 64 for s in signatures)


def test_totals_screen(session: Session, instance_id: int) -> None:
    """The consent screen shows what leaves the wallet and the fee."""
    parameters, result = vector("8_actions")
    payments = len(result["payments"])
    shown = []

    def flow(session: Session):
        yield from _accept_start(session)
        yield from _accept_outputs(session, payments)
        br = yield
        assert (br.code, br.name) == (B.SignTx, "confirm_total")
        shown.append(" ".join(session.debug.read_layout().text_content().split()))
        session.debug.press_yes()

    _sign(session, instance_id, parameters, flow(session))

    total = (result["payment_total"] + result["fee"]) / 10**8
    fee = result["fee"] / 10**8
    assert f"{total:g} ZEC" in shown[0]
    assert f"{fee:g} ZEC" in shown[0]


def _accept_flow_recording_addresses(
    session: Session, payments: int, shown: list, first_use: bool = True
):
    yield from _accept_start(session, first_use)
    for _ in range(payments):
        br = yield
        assert br.code == B.ConfirmOutput
        shown.append(screen_text(session))
        session.debug.press_yes()
        br = yield
        assert br.code == B.ConfirmOutput
        session.debug.press_yes()
    yield from _accept_total(session)


def test_payment_shows_its_user_address(session: Session, instance_id: int) -> None:
    """A payment is shown under the wallet's `user_address` (design §7).

    The vector's addresses carry a transparent receiver as well as the Orchard
    one, like the unified addresses wallets hand out, so the Orchard-only
    address the device would otherwise show cannot match them.
    """
    parameters, result = vector("stock_sdk_view_sdk")
    addresses = {payment["address"] for payment in result["payments"]}
    shown: list = []

    _sign(
        session,
        instance_id,
        parameters,
        _accept_flow_recording_addresses(session, len(addresses), shown),
    )

    # Actions are shuffled, so the payments arrive in either order.
    assert {
        address
        for screen in shown
        for address in addresses
        if "".join(address_pieces(screen, address)) == address
    } == addresses


# ZIP-316 receiver typecodes; 0x10 stands for one the device does not know.
P2PKH, SAPLING, ORCHARD, UNKNOWN = 0x00, 0x02, 0x03, 0x10


@pytest.mark.parametrize(
    "unknown_receiver_bytes",
    [
        pytest.param(None, id="three_receivers"),
        # 185 bytes brings a mainnet address to the 512-byte `user_address` budget.
        pytest.param(185, id="at_budget"),
    ],
)
def test_long_user_address_is_shown_whole(
    session: Session, instance_id: int, unknown_receiver_bytes: int | None
) -> None:
    """Every character of a long `user_address` is reachable before confirmation."""
    parameters, result = vector("stock_sdk_view_sdk")
    pczt = bytes.fromhex(parameters["pczt"])
    addresses = set()
    for payment in result["payments"]:
        receivers = {
            P2PKH: bytes(20),
            SAPLING: bytes(43),
            ORCHARD: bytes.fromhex(payment["receiver"]),
        }
        if unknown_receiver_bytes is not None:
            receivers[UNKNOWN] = bytes(unknown_receiver_bytes)
        address = zcash_ext.unified_address(receivers, "u")
        pczt = with_user_address(pczt, payment["address"], address)
        addresses.add(address)
    if unknown_receiver_bytes is not None:
        assert {len(address) for address in addresses} == {512}
    shown: list = []

    _sign(
        session,
        instance_id,
        {**parameters, "pczt": pczt.hex()},
        _accept_flow_recording_addresses(session, len(addresses), shown),
    )

    assert {
        address
        for screen in shown
        for address in addresses
        if "".join(address_pieces(screen, address)) == address
    } == addresses


def _accept_everything(session: Session):
    # Presses through whatever the device shows until it answers; used where
    # the rejection may land before or after the payment screens (the builder
    # shuffles the real spend's action index).
    while True:
        yield
        session.debug.press_yes()


def test_user_address_for_another_receiver_is_refused(
    session: Session, instance_id: int
) -> None:
    """Each payment names the other's address: refused before it is shown."""
    parameters, result = vector("stock_sdk_view_sdk")
    first, second = (payment["address"].encode() for payment in result["payments"])
    assert len(first) == len(second)
    pczt = bytes.fromhex(parameters["pczt"])
    placeholder = bytes(len(first))
    swapped = pczt.replace(first, placeholder).replace(second, first)
    swapped = swapped.replace(placeholder, second)

    with pytest.raises(TrezorFailure, match="does not match the Orchard receiver"):
        _sign(
            session,
            instance_id,
            {**parameters, "pczt": swapped.hex()},
            _accept_everything(session),
        )


def _accept_transparent_flow(
    session: Session, network: str, outputs: int, requests: list, shown: list
):
    """The deshield flow, recording every ButtonRequest and address screen.

    The order is fixed by the encoding: the transparent bundle precedes the
    shielded actions, so the privacy warning and the transparent outputs come
    before any shielded payment. The warning is once per transaction, however
    many transparent outputs follow.
    """
    yield from accept_account_request(session, 0, network.capitalize())
    # 12-word wallet: the ZIP-315 weak-backup warning comes first.
    br = yield
    requests.append((br.code, br.name))
    session.debug.press_yes()
    br = yield
    requests.append((br.code, br.name))
    session.debug.press_yes()
    for _ in range(outputs):
        br = yield
        requests.append((br.code, br.name))
        shown.append(screen_text(session))
        session.debug.press_yes()
        br = yield
        requests.append((br.code, br.name))
        session.debug.press_yes()
    br = yield
    requests.append((br.code, br.name))
    session.debug.press_yes()


@parametrize_using_fixture("sign_pczt.transparent.json")
def test_transparent_outputs(
    session: Session, instance_id: int, parameters: dict, result: dict
) -> None:
    """A deshield: transparent outputs shown as t-addresses, behind one warning.

    The addresses are the vector's, and the vector's come from
    `zcash_transparent`'s own encoder in the generator -- a second
    implementation of the Base58Check the app does with `zcash_address`. The
    screen must agree with it, because that string is the only thing the user
    can compare against their wallet.
    """
    assert len(bytes.fromhex(parameters["pczt"])) == result["pczt_length"]
    transparent = result["transparent_outputs"]
    shielded = len(result["payments"])
    requests: list = []
    shown: list = []

    signatures = _sign(
        session,
        instance_id,
        parameters,
        _accept_transparent_flow(
            session,
            parameters["network"],
            len(transparent) + shielded,
            requests,
            shown,
        ),
    )

    assert [s.action_index for s in signatures] == result["real_spend_actions"]
    # Weak backup, the privacy warning, two screens per output, then consent.
    assert requests == (
        [(B.Warning, "zcash_weak_backup"), (B.Warning, "zcash_transparent_payment")]
        + [(B.ConfirmOutput, "confirm_output")]
        * (len(transparent) + shielded)
        * OUTPUT_SCREENS
        + [(B.SignTx, "confirm_total")]
    )
    # The version bytes are the network's, not the wallet's: mainnet renders
    # t1…/t3…, testnet tm…/t2…. Pinned here as well as in the vector, so a
    # regeneration that moved either pair cannot pass by moving the
    # expectation with it.
    p2pkh_prefix, p2sh_prefix = {
        "mainnet": ("t1", "t3"),
        "testnet": ("tm", "t2"),
    }[parameters["network"]]
    for output in transparent:
        prefix = p2pkh_prefix if output["kind"] == "p2pkh" else p2sh_prefix
        assert output["address"].startswith(prefix), output
    # The transparent outputs come first, in bundle order, each shown as the
    # address the vector says, chunked in fours.
    for output, screen in zip(transparent, shown):
        pieces = address_pieces(screen, output["address"])
        assert pieces, screen
        assert "".join(pieces) == output["address"]
        assert max(len(piece) for piece in pieces) % 4 == 0


def _accept_outputs_with_memos(session: Session, payments: int, expected: str):
    # Address, amount, then the memo screen, which must show `expected`.
    for _ in range(payments):
        yield from _accept_outputs(session, 1)
        br = yield
        assert br.code == B.ConfirmOutput
        assert br.name == "confirm_memo"
        shown = screen_text(session)
        assert expected in shown.replace(" ", "").replace("\n", "") or expected in shown
        session.debug.press_yes()


def _accept_flow_with_memos(session: Session, payments: int, expected: str):
    yield from _accept_start(session)
    yield from _accept_outputs_with_memos(session, payments, expected)
    yield from _accept_total(session)


@parametrize_using_fixture("sign_pczt.memos.json")
def test_memo_is_shown_and_signed(
    session: Session, instance_id: int, parameters: dict, result: dict
) -> None:
    """A ZIP-302 text memo within the budget is shown verbatim on its own screen
    after the output; over the budget, not text, or not drawable as itself, the
    BLAKE2b-256 of the memo is shown instead (design §7).

    The vectors that must fall back to the digest are the over-budget memo, an
    arbitrary-data memo, a code point outside the BMP (the glyph lookup
    truncates to u16, so it would draw as some unrelated BMP character), and
    U+202E RIGHT-TO-LEFT OVERRIDE (which reverses what is drawn after it).
    """
    memo = result["memo"]
    expected = memo["text"] if memo["kind"] == "text" else memo["hex"]
    payments = len(result["payments"])

    signatures = _sign(
        session,
        instance_id,
        parameters,
        _accept_flow_with_memos(
            session, payments, expected[: 40 if memo["kind"] == "text" else 16]
        ),
    )

    assert [s.action_index for s in signatures] == result["real_spend_actions"]


@parametrize_using_fixture("sign_pczt.failed.json")
def test_zip32_derivation_not_the_device_own_is_rejected(
    session: Session, instance_id: int, parameters: dict, result: dict
) -> None:
    """A claim naming another seed or another account is refused as policy."""
    with pytest.raises(TrezorFailure, match=result["error"]):
        _sign(session, instance_id, parameters, _accept_everything(session))


def test_cancel_at_output(session: Session, instance_id: int) -> None:
    parameters, _result = vector("8_actions")

    def cancel_second_output(session: Session):
        yield from _accept_start(session)
        # Whole first payment, then the address screen of the second.
        yield from _accept_outputs(session, 1)
        br = yield
        assert br.code == B.ConfirmOutput
        session.debug.press_no()

    with pytest.raises(Cancelled):
        _sign(session, instance_id, parameters, cancel_second_output(session))


def test_cancel_at_totals_then_sign(session: Session, instance_id: int) -> None:
    parameters, result = vector("2_actions")
    payments = len(result["payments"])

    def cancel_totals(session: Session):
        yield from _accept_start(session)
        yield from _accept_outputs(session, payments)
        br = yield
        assert br.code == B.SignTx
        session.debug.press_no()

    with pytest.raises(Cancelled):
        _sign(session, instance_id, parameters, cancel_totals(session))

    # A cancelled review leaves nothing pending: the next request signs
    # normally, and the account needs no second request.
    signatures = _sign(
        session,
        instance_id,
        parameters,
        _accept_flow(session, payments, first_use=False),
    )
    assert [s.action_index for s in signatures] == result["real_spend_actions"]


def test_two_signs_in_one_app_instance(session: Session, instance_id: int) -> None:
    """The persistent caches the first sign builds serve the second; the
    session of the first leaves nothing behind that the second trips on.

    Stands in for the series' region test, whose arenas the app does not
    have: its heap is its own and dies with it."""
    parameters, result = vector("8_actions")
    payments = len(result["payments"])
    for first_use in (True, False):
        signatures = _sign(
            session,
            instance_id,
            parameters,
            _accept_flow(session, payments, first_use=first_use),
        )
        assert [s.action_index for s in signatures] == result["real_spend_actions"]


# The app's own transport checks, which the series had in Core.


def _begin(session: Session, instance_id: int, parameters: dict) -> ZcashPcztRequest:
    """Starts a sign and returns the device's first chunk request, past the
    account request and the weak-backup warning."""
    pczt = bytes.fromhex(parameters["pczt"])
    response = zcash_ext._exchange(
        session,
        instance_id,
        MessageType.ZcashSignPczt,
        zcash_ext._encode(
            ZcashSignPczt(
                network=NETWORKS[parameters["network"]],
                account=parameters["account"],
                pczt_length=len(pczt),
                host_reference_height=parameters["height"],
            )
        ),
    )
    assert response.message_id == MessageType.ZcashPcztRequest
    assert not response.finished
    return protobuf.load_message(io.BytesIO(response.data), ZcashPcztRequest)


def _ack(
    session: Session, instance_id: int, ack: ZcashPcztAck
) -> messages.ExtAppResponse:
    return zcash_ext._exchange(
        session, instance_id, MessageType.ZcashPcztAck, zcash_ext._encode(ack)
    )


@pytest.mark.parametrize(
    "tamper",
    [
        pytest.param(lambda ack: setattr(ack, "offset", 1), id="offset"),
        pytest.param(
            lambda ack: setattr(ack, "transfer_id", bytes(16)), id="transfer_id"
        ),
        pytest.param(lambda ack: setattr(ack, "data", ack.data[:-1]), id="short"),
        pytest.param(lambda ack: setattr(ack, "data", None), id="no-data"),
    ],
)
def test_a_chunk_that_is_not_the_one_requested_is_refused(
    session: Session, instance_id: int, tamper
) -> None:
    parameters, _result = vector("2_actions")
    pczt = bytes.fromhex(parameters["pczt"])
    with session.test_ctx as client:
        client.set_input_flow(_accept_start(session))
        request = _begin(session, instance_id, parameters)
        ack = ZcashPcztAck(
            transfer_id=request.transfer_id,
            offset=request.offset,
            data=pczt[: request.length],
        )
        tamper(ack)
        with pytest.raises(TrezorFailure, match="Invalid PCZT chunk"):
            _ack(session, instance_id, ack)


def test_host_cancel_mid_stream(session: Session, instance_id: int) -> None:
    """`ZcashCancel` in place of a chunk ends the request with ActionCancelled
    and leaves the session usable."""
    parameters, result = vector("2_actions")
    with session.test_ctx as client:
        client.set_input_flow(_accept_start(session))
        request = _begin(session, instance_id, parameters)
        assert request.offset == 0
        zcash_ext.cancel(session, instance_id)
    assert not session.is_invalid

    signatures = _sign(
        session,
        instance_id,
        parameters,
        _accept_flow(session, len(result["payments"]), first_use=False),
    )
    assert [s.action_index for s in signatures] == result["real_spend_actions"]


@pytest.mark.parametrize(
    "fields, message",
    [
        pytest.param(
            dict(account=0, pczt_length=100, host_reference_height=10_000_000),
            "Malformed Zcash request",
            id="no-network",
        ),
        pytest.param(
            dict(
                network=ZcashNetwork.Mainnet,
                account=0,
                host_reference_height=10_000_000,
            ),
            "Malformed Zcash request",
            id="no-length",
        ),
        pytest.param(
            dict(network=ZcashNetwork.Mainnet, account=0, pczt_length=100),
            "Malformed Zcash request",
            id="no-height",
        ),
        pytest.param(
            dict(
                network=ZcashNetwork.Mainnet,
                account=0,
                pczt_length=0,
                host_reference_height=10_000_000,
            ),
            "Invalid PCZT length",
            id="length-0",
        ),
        pytest.param(
            dict(
                network=ZcashNetwork.Mainnet,
                account=0,
                pczt_length=65_537,
                host_reference_height=10_000_000,
            ),
            "Invalid PCZT length",
            id="length-65537",
        ),
        pytest.param(
            dict(
                network=ZcashNetwork.Mainnet,
                account=2**31,
                pczt_length=100,
                host_reference_height=10_000_000,
            ),
            "Zcash request violates device policy",
            id="account-2^31",
        ),
    ],
)
def test_refuses_bad_requests_before_any_screen(
    session: Session, instance_id: int, fields: dict, message: str
) -> None:
    with session.test_ctx as client:
        client.set_input_flow(no_screens(session))
        with pytest.raises(exceptions.TrezorFailure, match=message):
            zcash_ext.call_ext(
                session,
                instance_id,
                ZcashSignPczt(**fields),
                zcash_ext.zcash_messages.ZcashSpendAuthSignatures,
            )
