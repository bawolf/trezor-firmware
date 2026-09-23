"""Streamed Ironwood signing on the emulator (T3B1 / T3T1 / T3W1).

The PCZTs are checked-in vectors, `common/tests/fixtures/zcash/sign_pczt*.json`,
built for the emulator's wallet and consumed like every other coin's corpus.
Each vector carries the hex PCZT, the network, account and host height the
device is asked for, and what the device must show and return: the payments,
the totals, the memo classification, the viewing key, the seed fingerprint,
and the action indices that must come back signed.

The signatures themselves are not re-verified here -- a hedged nonce makes them
non-deterministic, and host-side RedPallas verification is not something
trezorlib does. The cryptographic argument is `cargo test -p ironwood`, whose
equivalence and conformance suites apply every signature with the `pczt` crate's
signer against the action's `rk` and the host-computed sighash.
"""

import json

import pytest

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session
from trezorlib.debuglink import LayoutType
from trezorlib.exceptions import Cancelled, TrezorFailure

from ...common import COMMON_FIXTURES_DIR, parametrize_using_common_fixtures

B = messages.ButtonRequestType

MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
NETWORKS = {
    "mainnet": messages.ZcashNetwork.Mainnet,
    "testnet": messages.ZcashNetwork.Testnet,
}

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.zcash,
    pytest.mark.ironwood,
    pytest.mark.models("t3b1", "t3t1", "t3w1"),
    pytest.mark.setup_client(mnemonic=MNEMONIC),
]


def _sign(session: Session, parameters: dict, flow) -> list:
    with session.test_ctx as client:
        client.set_input_flow(flow)
        return zcash.sign_pczt(
            session,
            bytes.fromhex(parameters["pczt"]),
            NETWORKS[parameters["network"]],
            parameters["account"],
            parameters["height"],
        )


def _vector(name: str) -> tuple[dict, dict]:
    """One checked-in vector by name, for the flows parametrization cannot express."""
    for path in (
        "sign_pczt.json",
        "sign_pczt.memos.json",
        "sign_pczt.transparent.json",
    ):
        fixture = json.loads((COMMON_FIXTURES_DIR / "zcash" / path).read_text())
        for test in fixture["tests"]:
            if test["name"] == name:
                return test["parameters"], test["result"]
    raise KeyError(name)


# On T3T1 (delizia), T3W1 (eckhart) and T3B1 (caesar) each payment is two
# ConfirmOutput screens: the address, then the amount.
OUTPUT_SCREENS = 2


def _accept_outputs(session: Session, payments: int):
    for _ in range(payments * OUTPUT_SCREENS):
        br = yield
        assert br.code == B.ConfirmOutput
        session.debug.press_yes()


def _accept_flow(session: Session, payments: int):
    # 12-word wallet: the ZIP-315 weak-backup warning comes first.
    br = yield
    assert br.code == B.Warning
    session.debug.press_yes()
    yield from _accept_outputs(session, payments)
    br = yield
    assert br.code == B.SignTx
    session.debug.press_yes()


@parametrize_using_common_fixtures("zcash/sign_pczt.json")
def test_streamed_sign(session: Session, parameters: dict, result: dict) -> None:
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

    signatures = _sign(session, parameters, _accept_flow(session, payments))

    # The builder shuffles action order, so which action carries the real spend
    # is a property of the vector, not of its size.
    assert [s.action_index for s in signatures] == result["real_spend_actions"]
    assert all(len(s.signature) == 64 for s in signatures)


def _accept_transparent_flow(
    session: Session, outputs: int, requests: list, shown: list
):
    """The deshield flow, recording every ButtonRequest and address screen.

    The order is fixed by the encoding: the transparent bundle precedes the
    shielded actions, so the privacy warning and the transparent outputs come
    before any shielded payment. The warning is once per transaction, however
    many transparent outputs follow.
    """
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
        shown.append(_address_screen_text(session))
        session.debug.press_yes()
        br = yield
        requests.append((br.code, br.name))
        session.debug.press_yes()
    br = yield
    requests.append((br.code, br.name))
    session.debug.press_yes()


@parametrize_using_common_fixtures("zcash/sign_pczt.transparent.json")
def test_transparent_outputs(session: Session, parameters: dict, result: dict) -> None:
    """A deshield: transparent outputs shown as t-addresses, behind one warning.

    The addresses are the vector's, and the vector's come from
    `zcash_transparent`'s own encoder in the generator -- a second
    implementation of the Base58Check the device does with `coininfo`'s version
    bytes. The screen must agree with it, because that string is the only thing
    the user can compare against their wallet.
    """
    assert len(bytes.fromhex(parameters["pczt"])) == result["pczt_length"]
    transparent = result["transparent_outputs"]
    shielded = len(result["payments"])
    requests: list = []
    shown: list = []

    signatures = _sign(
        session,
        parameters,
        _accept_transparent_flow(session, len(transparent) + shielded, requests, shown),
    )

    assert [s.action_index for s in signatures] == result["real_spend_actions"]
    # Weak backup, the privacy warning, two screens per output, then consent.
    assert requests == (
        [(B.Warning, "ironwood_weak_backup"), (B.Warning, "zcash_transparent_payment")]
        + [(B.ConfirmOutput, "confirm_output")]
        * (len(transparent) + shielded)
        * OUTPUT_SCREENS
        + [(B.SignTx, "confirm_total")]
    )
    # The version bytes are the network's, not the wallet's: mainnet renders
    # t1…/t3…, testnet tm…/t2…. Pinned here as well as in the vector, so a
    # coininfo regeneration that moved either pair cannot pass by moving the
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
        pieces = _address_pieces(screen, output["address"])
        assert pieces, screen
        assert "".join(pieces) == output["address"]
        assert max(len(piece) for piece in pieces) % 4 == 0


MEMO_TEXT_BUDGET = 256


def _memo_screen_text(session: Session) -> str:
    """Everything the memo screen shows, paging where the model needs it.

    On Safe 3 (caesar, 128x64) a page holds about 40 characters, so a memo at
    the display budget -- and even the hash label on its own -- spills onto
    later pages, and the right press that advances a page is the same press
    that confirms on the last one. So on caesar read every page and stop on
    the last, leaving the caller's `press_yes` to confirm. delizia and
    eckhart show the value on the first page and are left alone.
    """
    debug = session.debug
    layout = debug.read_layout()
    shown = layout.text_content()
    if debug.layout_type is LayoutType.Caesar:
        for _ in range(layout.page_count() - 1):
            debug.press_right()
            shown += " " + debug.read_layout().text_content()
    return shown


def _accept_outputs_with_memos(session: Session, payments: int, expected: str):
    # Address, amount, then the memo screen, which must show `expected`.
    for _ in range(payments):
        yield from _accept_outputs(session, 1)
        br = yield
        assert br.code == B.ConfirmOutput
        assert br.name == "confirm_memo"
        shown = _memo_screen_text(session)
        assert expected in shown.replace(" ", "").replace("\n", "") or expected in shown
        session.debug.press_yes()


def _accept_flow_with_memos(session: Session, payments: int, expected: str):
    br = yield
    assert br.code == B.Warning
    session.debug.press_yes()
    yield from _accept_outputs_with_memos(session, payments, expected)
    br = yield
    assert br.code == B.SignTx
    session.debug.press_yes()


@parametrize_using_common_fixtures("zcash/sign_pczt.memos.json")
def test_memo_is_shown_and_signed(
    session: Session, parameters: dict, result: dict
) -> None:
    """A ZIP-302 text memo within the budget is shown verbatim on its own screen
    after the output; over the budget, not text, or not drawable as itself, the
    BLAKE2b-256 of the memo is shown instead (design note section 11).

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
        parameters,
        _accept_flow_with_memos(
            session, payments, expected[: 40 if memo["kind"] == "text" else 16]
        ),
    )

    assert [s.action_index for s in signatures] == result["real_spend_actions"]


def _accept_everything(session: Session):
    # Presses through whatever the device shows until it answers; used where
    # the rejection may land before or after the payment screens (the builder
    # shuffles the real spend's action index).
    while True:
        yield
        session.debug.press_yes()


@parametrize_using_common_fixtures("zcash/sign_pczt.failed.json")
def test_zip32_derivation_not_the_device_own_is_rejected(
    session: Session, parameters: dict, result: dict
) -> None:
    """A claim naming another seed or another account is refused as policy."""
    with (
        session.test_ctx as client,
        pytest.raises(TrezorFailure, match=result["error"]),
    ):
        client.set_input_flow(_accept_everything(session))
        zcash.sign_pczt(
            session,
            bytes.fromhex(parameters["pczt"]),
            NETWORKS[parameters["network"]],
            parameters["account"],
            parameters["height"],
        )


def test_device_fvk_matches_fixture(session: Session) -> None:
    """The signing derivation (orchard zip32) and the viewing export (receive
    crate) agree, and the default export releases no seed fingerprint."""
    parameters, result = _vector("2_actions")

    def accept(session: Session):
        br = yield
        assert br.code == B.SignTx
        assert br.name == "ironwood_export_viewing_key"
        session.debug.press_yes()
        # The weak-backup warning; there is no seed-fingerprint screen
        # because the host did not ask for the fingerprint.
        br = yield
        assert br.code == B.Warning
        assert br.name == "ironwood_weak_backup"
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        export = zcash.export_viewing_key(
            session, NETWORKS[parameters["network"]], parameters["account"]
        )

    _hrp, data = zcash._bech32m_decode(export.key)
    jumbled = bytearray(zcash._convert_bits(data, 5, 8, pad=False))
    zcash._f4jumble(jumbled, inverse=True)
    assert jumbled[2:98].hex() == result["fvk"]
    assert export.seed_fingerprint is None


def test_seed_fingerprint_export_is_opt_in(session: Session) -> None:
    """Asking for the seed fingerprint inserts a screen that says what it links.

    The account viewing key's own screen promises account scope; the
    fingerprint is the same value for every account and both networks, so it
    gets its own warning between that screen and any use of the seed.
    """
    parameters, result = _vector("2_actions")
    shown = []

    def accept(session: Session):
        br = yield
        assert br.code == B.SignTx
        assert br.name == "ironwood_export_viewing_key"
        session.debug.press_yes()
        br = yield
        assert br.code == B.Warning
        assert br.name == "ironwood_seed_fingerprint"
        shown.append(session.debug.read_layout().text_content())
        session.debug.press_yes()
        br = yield
        assert br.code == B.Warning
        assert br.name == "ironwood_weak_backup"
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        export = zcash.export_viewing_key(
            session,
            NETWORKS[parameters["network"]],
            parameters["account"],
            include_seed_fingerprint=True,
        )

    # The first page of the warning names the seed; caesar paginates the rest.
    assert "recovery seed" in " ".join(shown[0].split())
    # The vector carries zip32::fingerprint::SeedFingerprint of the same seed;
    # the device computes it natively (trezorironwood).
    assert export.seed_fingerprint.hex() == result["seed_fingerprint"]
    assert len(export.seed_fingerprint) == 32


def _address_screen_text(session: Session) -> str:
    """The receive-address screen, paging where the model needs it."""
    debug = session.debug
    layout = debug.read_layout()
    shown = layout.text_content()
    if debug.layout_type is LayoutType.Caesar:
        for _ in range(layout.page_count() - 1):
            debug.press_right()
            shown += " " + debug.read_layout().text_content()
    return shown


def _address_pieces(screen: str, address: str) -> list[str]:
    """The runs of address on screen, dropping labels like "Mainnet"."""
    return [word for word in screen.split() if word and word in address]


def _get_address_reading_the_screen(session: Session, chunkify: bool):
    """Drive ZcashGetAddress, recording the ButtonRequests and the screen."""
    shown = []
    requests = []

    def accept(session: Session):
        br = yield
        requests.append((br.code, br.name))
        session.debug.press_yes()
        br = yield
        requests.append((br.code, br.name))
        shown.append(_address_screen_text(session))
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        address = zcash.get_address(
            session, messages.ZcashNetwork.Mainnet, 0, bytes(11), chunkify=chunkify
        )
    return address, requests, shown[0]


def test_receive_address_chunkify(session: Session) -> None:
    """`chunkify` groups the 106-character UA on screen, like Bitcoin's.

    It is presentation only: the address the device returns and the
    ButtonRequests it sends are the same either way, so it changes nothing a
    host verifies. Off by default, so a host that has never heard of the field
    gets exactly the screen it got before.
    """
    plain_addr, plain_brs, plain_screen = _get_address_reading_the_screen(
        session, chunkify=False
    )
    chunked_addr, chunked_brs, chunked_screen = _get_address_reading_the_screen(
        session, chunkify=True
    )

    assert plain_addr == chunked_addr
    assert plain_addr.startswith("u1")
    assert len(plain_addr) == 106

    expected_brs = [
        (B.Warning, "ironwood_weak_backup"),
        (B.Address, "ironwood_receive"),
    ]
    assert plain_brs == expected_brs
    assert chunked_brs == expected_brs

    # Both screens carry the address, in order, from the start, and nothing
    # else that looks like it. The debug layout reports lines, not the spaces
    # inside them, so what chunking shows up as is width: a full chunked line
    # is a whole number of four-character groups and is narrower than a full
    # unchunked one. Paying width is why the chunked first page holds a prefix
    # rather than all 106 characters -- the trade the host opts into.
    plain_pieces = _address_pieces(plain_screen, plain_addr)
    chunked_pieces = _address_pieces(chunked_screen, chunked_addr)
    assert plain_pieces and chunked_pieces
    assert plain_addr.startswith("".join(plain_pieces))
    assert chunked_addr.startswith("".join(chunked_pieces))
    widest_chunked = max(len(piece) for piece in chunked_pieces)
    assert widest_chunked % 4 == 0
    assert widest_chunked < max(len(piece) for piece in plain_pieces)
    assert chunked_screen != plain_screen


def test_cancel_at_output(session: Session) -> None:
    parameters, _result = _vector("8_actions")

    def cancel_second_output(session: Session):
        br = yield
        assert br.code == B.Warning
        session.debug.press_yes()
        # Whole first payment, then the address screen of the second.
        yield from _accept_outputs(session, 1)
        br = yield
        assert br.code == B.ConfirmOutput
        session.debug.press_no()

    with session.test_ctx as client, pytest.raises(Cancelled):
        client.set_input_flow(cancel_second_output(session))
        zcash.sign_pczt(
            session,
            bytes.fromhex(parameters["pczt"]),
            NETWORKS[parameters["network"]],
            parameters["account"],
            parameters["height"],
        )


def test_cancel_at_totals_then_sign(session: Session) -> None:
    parameters, result = _vector("2_actions")
    payments = len(result["payments"])

    def cancel_totals(session: Session):
        br = yield
        assert br.code == B.Warning
        session.debug.press_yes()
        yield from _accept_outputs(session, payments)
        br = yield
        assert br.code == B.SignTx
        session.debug.press_no()

    with session.test_ctx as client, pytest.raises(Cancelled):
        client.set_input_flow(cancel_totals(session))
        zcash.sign_pczt(
            session,
            bytes.fromhex(parameters["pczt"]),
            NETWORKS[parameters["network"]],
            parameters["account"],
            parameters["height"],
        )

    # A cancelled review leaves nothing pending: the next request signs normally.
    signatures = _sign(session, parameters, _accept_flow(session, payments))
    assert [s.action_index for s in signatures] == result["real_spend_actions"]


# The two tiers, pinned here as well as in the firmware. `SCRATCH_BYTES` is
# `apps.zcash.sign_pczt.SCRATCH_BYTES` (and `trezorironwood.SCRATCH_BYTES`,
# which it is asserted equal to in `core/tests/test_trezorironwood.py`);
# `REGION_BYTES` is `ironwood::allocator::REGION_BYTES`, the `.zcash_region`
# static. Pinned rather than imported because this file talks to a device
# whose firmware may not be this tree's.
SCRATCH_BYTES = 24 * 1024
REGION_BYTES = 40 * 1024

REGION_KEYS = ("persist_in_use", "persist_peak", "scratch_in_use", "scratch_peak")


def _region_info(session: Session) -> dict | None:
    """The two signing arenas, read over debuglink, or `None` where absent.

    `apps.debug` appends the four figures to `DebugLinkGcInfo`, whose payload
    is a free-form name/value list, because the signing region is not on the
    GC heap and `gc.mem_info()` cannot see it. They are absent on a build
    without debuglink and on the emulator, whose allocator is plain `malloc`
    and has no arenas at all (`rust/src/ironwood/allocator_unix.rs`).
    """
    debug = session.debug
    if not debug.has_gc_info:
        return None
    response = debug._call(
        messages.DebugLinkGetGcInfo(), expect=messages.DebugLinkGcInfo
    )
    info = {item.name: item.value for item in response.items}
    if not all(key in info for key in REGION_KEYS):
        return None
    return {key: info[key] for key in REGION_KEYS}


def test_two_signs_in_one_boot_leave_the_region_as_they_found_it(
    session: Session,
) -> None:
    """The two invariants the two-tier region rests on, on real hardware.

    The rooted tier is a boot-lifetime `.zcash_region` static holding the
    persistent set (Pasta's square-root table, orchard's commitment-domain
    caches); the scratch tier is a `bytearray` borrowed from the GC heap for
    one session and given back. So:

    * `persist_in_use` must be IDENTICAL after each sign. Drift is the
      2026-09-18 cross-session bug returning -- something the first sign
      rooted and the second rooted again.
    * `scratch_in_use` must be ZERO once the workflow's `finally` has run.
      A non-zero figure is a Rust static that was first filled during a
      session and now points into memory the collector has taken back.

    Neither is reachable from the emulator: `allocator_unix.rs` delegates to
    `malloc` and installs no tier, so `debug_region_info()` is `None` there
    and this SKIPS rather than passing on four zeros. The equivalent host
    statement is `ironwood/tests/region_budget.rs`, which runs the device's
    block arithmetic over the real allocation traces; this is the same claim
    on 32-bit silicon.

    Note that `scratch_in_use != 0` cannot be observed after the fact: the
    device treats it as fatal at `release_scratch`, so a device that reaches
    the second reading has already passed it. The assertion below is the
    belt to that braces, and it is also what would catch the counter and the
    block chain disagreeing (`region_info` reads the chains).
    """
    parameters, result = _vector("2_actions")
    payments = len(result["payments"])

    if _region_info(session) is None:
        pytest.skip(
            "no signing-region counters: this build has no debuglink, or it is "
            "the emulator, which has no arenas (allocator_unix.rs is malloc)"
        )

    readings = []
    for attempt in range(2):
        signatures = _sign(session, parameters, _accept_flow(session, payments))
        assert [s.action_index for s in signatures] == result["real_spend_actions"], (
            f"sign {attempt + 1}"
        )
        readings.append(_region_info(session))

    first, second = readings
    # Not vacuous: the first sign really did root a persistent set. The host
    # model puts it at 30,304 B; what matters here is that it is non-zero and
    # then never moves.
    assert first["persist_in_use"] > 0
    assert first["persist_in_use"] == second["persist_in_use"], readings
    for attempt, reading in enumerate(readings):
        assert reading["scratch_in_use"] == 0, f"sign {attempt + 1}: {reading}"
        assert reading["scratch_peak"] <= SCRATCH_BYTES, f"sign {attempt + 1}"
        assert reading["persist_peak"] <= REGION_BYTES, f"sign {attempt + 1}"
