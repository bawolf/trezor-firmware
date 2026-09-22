"""EMULATOR RAM high-water sweep for Ironwood streaming signing.

TEMPORARY investigation harness (emulator-ram-sweep-20260918); not for commit.
Reuses the ironwood_fixture host tool to build N-action PCZTs, signs each on the
T3T1 emulator, and records the per-session region counters
(`session_peak_bytes`/`in_use_at_begin_bytes`/`boot_peak_bytes`/`region_bytes`)
carried in the returned `debug_timings` (via trezorlib.zcash.last_debug_timings).
Run the sweep ASCENDING N on ONE boot: session_peak should stay ~flat and
in_use_at_begin constant after the first session.

Config via env:
  RAM_SWEEP_NS   comma list of action counts (default "1,2,4,6,8")
  RAM_SWEEP_OUT  path to append one result line per N (default stdout only)
  IRONWOOD_REGION_BYTES  inherited by the emulator; overrides the region size.

ZERO BROADCAST; public test wallet only ("abandon" x11 "about").
"""

import json
import os
import shutil
import subprocess
from pathlib import Path

import pytest
from mnemonic import Mnemonic

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session

B = messages.ButtonRequestType

MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
SEED_HEX = Mnemonic("english").to_seed(MNEMONIC, passphrase="").hex()
NETWORK = messages.ZcashNetwork.Mainnet
ACCOUNT = 0
HOST_HEIGHT = 10_000_000

REPO = Path(__file__).resolve().parents[3]
CRATE = REPO / "core" / "embed" / "ironwood"

NS = [int(x) for x in os.environ.get("RAM_SWEEP_NS", "1,2,4,6,8").split(",") if x]

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.models("t3b1", "t3t1", "t3w1"),
    pytest.mark.setup_client(mnemonic=MNEMONIC),
]


def _cargo() -> list[str]:
    cargo = os.environ.get("IRONWOOD_CARGO") or shutil.which("cargo")
    if cargo is None:
        pytest.skip("no cargo to build the ironwood fixture tool")
    return [cargo]


@pytest.fixture(scope="module")
def fixture_tool(tmp_path_factory) -> Path:
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
        [tool, "build", SEED_HEX, "mainnet", str(ACCOUNT), str(HOST_HEIGHT),
         str(actions), str(out)],
        check=True, capture_output=True, text=True,
    ).stdout
    return out.read_bytes(), json.loads(summary)


def _verify(tool: Path, directory: Path, actions: int, signatures) -> str:
    records = directory / f"records-{actions}.bin"
    records.write_bytes(
        b"".join(bytes((zcash.POOL_IRONWOOD, s.action_index)) + s.signature
                 for s in signatures)
    )
    return subprocess.run(
        [tool, "verify", str(directory / f"fixture-{actions}.pczt"), str(records)],
        check=True, capture_output=True, text=True,
    ).stdout


OUTPUT_SCREENS = 2


def _accept_flow(session: Session, payments: int):
    br = yield
    assert br.code == B.Warning
    session.debug.press_yes()
    for _ in range(payments * OUTPUT_SCREENS):
        br = yield
        assert br.code == B.ConfirmOutput
        session.debug.press_yes()
    br = yield
    assert br.code == B.SignTx
    session.debug.press_yes()


@pytest.mark.parametrize("actions", NS)
def test_ram_sweep(
    session: Session, fixture_tool: Path, tmp_path: Path, actions: int
) -> None:
    pczt, summary = _build_fixture(fixture_tool, tmp_path, actions)
    payments = len(summary["payments"])

    with session.test_ctx as client:
        client.set_input_flow(_accept_flow(session, payments))
        signatures = zcash.sign_pczt(session, pczt, NETWORK, ACCOUNT, HOST_HEIGHT)

    dt = zcash.last_debug_timings
    dt_s = dt.decode() if isinstance(dt, (bytes, bytearray)) else str(dt)
    fields = dict(kv.split("=", 1) for kv in dt_s.split(" ") if "=" in kv)
    # Per-session counters (Fable review #2): session_peak resets each
    # session_begin so it is ~flat across an ascending-N single-boot sweep;
    # in_use_at_begin is the persistent set and must stay constant (a leak
    # otherwise); boot_peak is the boot-monotone max.
    sp = fields.get("session_peak_bytes")
    iub = fields.get("in_use_at_begin_bytes")
    bp = fields.get("boot_peak_bytes")
    rb = fields.get("region_bytes")
    ac = fields.get("action_count")

    verified = _verify(fixture_tool, tmp_path, actions, signatures)
    line = (
        f"N_requested={actions} action_count={ac} payments={payments} "
        f"pczt_len={len(pczt)} session_peak_bytes={sp} in_use_at_begin_bytes={iub} "
        f"boot_peak_bytes={bp} region_bytes={rb} "
        f"signatures={len(signatures)} verify={verified.strip()!r} "
        f"env_region={os.environ.get('IRONWOOD_REGION_BYTES', 'default')}"
    )
    print("RAM_SWEEP_RESULT " + line)
    out = os.environ.get("RAM_SWEEP_OUT")
    if out:
        with open(out, "a") as f:
            f.write(line + "\n")

    assert len(signatures) == 1
    assert "verified 1 signature(s)" in verified
