"""Streamed Ironwood signing on the T3T1 emulator.

The PCZT fixture is built for the emulator's wallet by the Rust example
`core/embed/ironwood/examples/ironwood_fixture.rs`, and every returned
signature is verified by the same tool with the pczt crate's signer (against
the action's rk and the host-computed sighash). Requires a host `cargo` able
to build the ironwood crate; set IRONWOOD_CARGO to point at one.
"""

import json
import os
import shutil
import subprocess
import time
from pathlib import Path

import pytest
from mnemonic import Mnemonic

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session
from trezorlib.exceptions import Cancelled

B = messages.ButtonRequestType

MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
SEED_HEX = Mnemonic("english").to_seed(MNEMONIC, passphrase="").hex()
NETWORK = messages.ZcashNetwork.Mainnet
ACCOUNT = 0
# Any NU6.3 height works for the emulator; the fixture expires 40 blocks later.
HOST_HEIGHT = 10_000_000

REPO = Path(__file__).resolve().parents[3]
CRATE = REPO / "core" / "embed" / "ironwood"

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.models("t3t1", "t3w1"),
    pytest.mark.setup_client(mnemonic=MNEMONIC),
]


def _cargo() -> list[str]:
    cargo = os.environ.get("IRONWOOD_CARGO") or shutil.which("cargo")
    if cargo is None:
        pytest.skip("no cargo to build the ironwood fixture tool")
    return [cargo]


@pytest.fixture(scope="module")
def fixture_tool(tmp_path_factory) -> Path:
    # A separate target directory so the test never contends with a firmware build.
    target = REPO / "core" / "build-xtask" / "ironwood-examples"
    subprocess.run(
        _cargo()
        + [
            "build",
            "--quiet",
            "--manifest-path",
            str(CRATE / "Cargo.toml"),
            "--example",
            "ironwood_fixture",
        ],
        check=True,
        env={**os.environ, "CARGO_TARGET_DIR": str(target)},
    )
    return target / "debug" / "examples" / "ironwood_fixture"


def _build_fixture(tool: Path, directory: Path, actions: int) -> tuple[bytes, dict]:
    out = directory / f"fixture-{actions}.pczt"
    summary = subprocess.run(
        [
            tool,
            "build",
            SEED_HEX,
            "mainnet",
            str(ACCOUNT),
            str(HOST_HEIGHT),
            str(actions),
            str(out),
        ],
        check=True,
        capture_output=True,
        text=True,
    ).stdout
    return out.read_bytes(), json.loads(summary)


def _verify(tool: Path, directory: Path, actions: int, signatures) -> str:
    records = directory / f"records-{actions}.bin"
    records.write_bytes(
        b"".join(
            bytes((zcash.POOL_IRONWOOD, s.action_index)) + s.signature
            for s in signatures
        )
    )
    return subprocess.run(
        [tool, "verify", str(directory / f"fixture-{actions}.pczt"), str(records)],
        check=True,
        capture_output=True,
        text=True,
    ).stdout


# On T3T1 (delizia) and T3W1 (eckhart) each payment is two ConfirmOutput screens:
# the address, then the amount.
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


@pytest.mark.parametrize("actions", [2, 8, 16, 32])
def test_streamed_sign_verifies(
    session: Session, fixture_tool: Path, tmp_path: Path, actions: int
) -> None:
    pczt, summary = _build_fixture(fixture_tool, tmp_path, actions)
    assert summary["pczt_length"] == len(pczt)
    payments = len(summary["payments"])

    with session.test_ctx as client:
        client.set_input_flow(_accept_flow(session, payments))
        signatures = zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)

    # One record for the single real spend (the builder shuffles action order),
    # verified by the pczt signer against rk and the host-computed sighash; the
    # tool also checks the signed set equals the set of real spends.
    assert len(signatures) == 1
    assert all(len(s.signature) == 64 for s in signatures)
    assert "verified 1 signature(s)" in _verify(
        fixture_tool, tmp_path, actions, signatures
    )


def test_device_fvk_matches_fixture(
    session: Session, fixture_tool: Path, tmp_path: Path
) -> None:
    """The signing derivation (orchard zip32) and the viewing export (receive crate) agree."""
    _pczt, summary = _build_fixture(fixture_tool, tmp_path, 2)

    def accept(session: Session):
        br = yield
        assert br.code == B.SignTx
        session.debug.press_yes()
        br = yield
        assert br.code == B.Warning
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        key = zcash.get_viewing_key(session, NETWORK, ACCOUNT)

    _hrp, data = zcash._bech32m_decode(key)
    jumbled = bytearray(zcash._convert_bits(data, 5, 8, pad=False))
    zcash._f4jumble(jumbled, inverse=True)
    assert jumbled[2:98].hex() == summary["fvk"]


def test_cancel_at_output(session: Session, fixture_tool: Path, tmp_path: Path) -> None:
    pczt, _summary = _build_fixture(fixture_tool, tmp_path, 8)

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
        zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)


def test_cancel_at_totals_then_sign(
    session: Session, fixture_tool: Path, tmp_path: Path
) -> None:
    pczt, summary = _build_fixture(fixture_tool, tmp_path, 2)
    payments = len(summary["payments"])

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
        zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)

    # A cancelled review leaves nothing pending: the next request signs normally.
    with session.test_ctx as client:
        client.set_input_flow(_accept_flow(session, payments))
        signatures = zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)
    assert "verified 1 signature(s)" in _verify(fixture_tool, tmp_path, 2, signatures)


# ---------------------------------------------------------------------------
# Autolock teardown (Fable review #S2) — DEFERRED.
#
# The timeout-removal change (commit 0e15ccd8) relies on Trezor's standard
# autolock to tear down an abandoned in-flight sign: when the idle timer fires,
# `lock_manager.lock_device` calls `workflow.close_others()`, which throws
# GeneratorExit into the parked `sign_pczt` workflow; that unwinds through its
# `finally`, running `_cancel_native()` (native `session_cancel`, wiping the
# session secrets). No existing test exercises this path. A `LockDevice` message
# is NOT a substitute: it reaches the workflow as an *unexpected message* (the
# exception path), not via `close_others()`, so it does not cover the autolock
# case.
#
# This test is deferred because a faithful version needs (a) a PIN-configured
# emulator so `can_lock_device()` is true and autolock actually calls
# `close_others()` (without a PIN, autolock only shows a screensaver and never
# tears the workflow down), (b) the emulator's real idle timer to fire on a wall
# clock, and (c) a way to prove the native slot was cleared — either the
# measurement build's `session_region_high_water`/`session_active()` probe, or
# the "fresh sign after unlock succeeds" proxy below. None of that runs in a
# CI-less firmware worktree; it needs the full emulator + debuglink harness and
# a firmware build.
#
# Exact test to add once the emulator harness is available:
@pytest.mark.skip(
    reason="deferred (Fable #S2): needs PIN-configured emulator + real autolock "
    "idle timer + session-active probe; not runnable without the device build"
)
@pytest.mark.setup_client(mnemonic=MNEMONIC, pin="1234")
def test_autolock_tears_down_abandoned_sign(
    session: Session, fixture_tool: Path, tmp_path: Path
) -> None:
    pczt, summary = _build_fixture(fixture_tool, tmp_path, 8)
    payments = len(summary["payments"])

    # Debug-build minimum autolock delay is 10 s (storage/device.py:68).
    session.call(messages.ApplySettings(auto_lock_delay_ms=10_000))

    def walk_away(session: Session):
        # Advance to the first ConfirmOutput ButtonRequest, then never ACK it:
        # the workflow parks on the layout while the idle timer runs down.
        br = yield
        assert br.code == B.Warning
        session.debug.press_yes()
        br = yield
        assert br.code == B.ConfirmOutput
        # Do NOT press; wait past auto_lock_delay_ms so autolock fires and
        # `close_others()` throws GeneratorExit into the parked sign_pczt.
        time.sleep(12)

    # Autolock closing the workflow surfaces to the host as Failure(ActionCancelled).
    with session.test_ctx as client, pytest.raises(Cancelled):
        client.set_input_flow(walk_away(session))
        zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)

    # The device is now PIN-locked (autolock ran `config.lock()`).
    assert session.features.unlocked is False

    # Prove the native slot was wiped/cleared by the `finally` teardown: after
    # unlocking, a fresh sign succeeds. If the previous request's native session
    # had leaked, `session_begin` would have to silently cancel a stale request;
    # for an airtight check, additionally assert `session_active()` was False at
    # begin via the measurement build's probe.
    session.debug.press_yes()  # unlock via PIN entry in the input flow / debuglink
    with session.test_ctx as client:
        client.set_input_flow(_accept_flow(session, payments))
        signatures = zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)
    assert "verified 1 signature(s)" in _verify(fixture_tool, tmp_path, 8, signatures)
