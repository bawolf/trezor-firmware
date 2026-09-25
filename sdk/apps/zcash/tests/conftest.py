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

"""Device-test fixtures of the Zcash app.

A reduced copy of the Ethereum app's conftest: every test wipes the device,
loads a mnemonic (`setup_client` marker) and then the app (`--app`). There
are no UI screenshot fixtures yet, so `--ui` is refused.
"""

from __future__ import annotations

import os
import subprocess
import sys
import typing as t
from dataclasses import asdict, dataclass
from pathlib import Path

import pytest

from trezorlib import client as client_module
from trezorlib import debuglink, log
from trezorlib.debuglink import TrezorTestContext
from trezorlib.testing import translations
from trezorlib.transport import get_transport
from trezorlib.transport.ble import BleTransport

if t.TYPE_CHECKING:
    from _pytest.config import Config
    from _pytest.config.argparsing import Parser

    from trezorlib.client import Session

ROOT = Path(__file__).resolve().parent.parent  # sdk/apps/zcash


@pytest.fixture(scope="session")
def _raw_test_ctx(request: pytest.FixtureRequest) -> TrezorTestContext:
    BleTransport.ENABLED = False
    # prevent tests from getting stuck in case there is an USB packet loss
    client_module._DEFAULT_READ_TIMEOUT = 50.0
    path = os.environ.get("TREZOR_PATH")
    if not path:
        raise RuntimeError("Set TREZOR_PATH, e.g. udp:127.0.0.1:21324")
    try:
        return TrezorTestContext(
            get_transport(path), auto_interact=True, force_wipe=True
        )
    except Exception:
        request.session.shouldstop = "Failed to communicate with Trezor"
        raise


@dataclass
class SetupParams:
    mnemonic: str = " ".join(["all"] * 12)
    extapp_path: str | None = None
    extapp_instance_id: int | None = None

    @classmethod
    def from_request(cls, request: pytest.FixtureRequest) -> SetupParams:
        params = asdict(cls())
        marker = request.node.get_closest_marker("setup_client")
        if marker:
            params.update(marker.kwargs)
        params["extapp_path"] = request.config.getoption("app")
        return cls(**params)

    def configure_client(self, session: Session) -> None:
        debuglink.load_device(
            session,
            mnemonic=self.mnemonic,
            pin=None,
            passphrase_protection=False,
            label="test",
        )
        if not self.extapp_path:
            raise ValueError("--app option must be provided")
        path = Path(self.extapp_path)
        if not path.exists():
            raise FileNotFoundError(f"Modular app not found: {path}")
        self.extapp_instance_id = debuglink.load_extapp(session, path)


@pytest.fixture(scope="function")
def setup_params(request: pytest.FixtureRequest) -> SetupParams:
    return SetupParams.from_request(request)


@pytest.fixture(scope="function")
def _prepared_test_ctx(
    request: pytest.FixtureRequest,
    _raw_test_ctx: TrezorTestContext,
    setup_params: SetupParams,
) -> TrezorTestContext:
    _raw_test_ctx.reset_debug_features()
    try:
        _raw_test_ctx.sync_responses()
    except Exception:
        request.session.shouldstop = "Failed to communicate with Trezor"
        pytest.fail("Failed to communicate with Trezor")

    # Use DebugLink to wipe (since THP channel requires unlocked device)
    _raw_test_ctx.wipe_device()
    session = _raw_test_ctx.get_session(passphrase=None)
    if not _raw_test_ctx.features.bootloader_mode:
        _raw_test_ctx.refresh_features()

    # Load language again, as it got erased in wipe
    translations.check_language(session, request.config.getoption("lang") or "en")

    setup_params.configure_client(session)
    session.close()
    return _raw_test_ctx


@pytest.fixture(scope="function")
def instance_id(
    setup_params: SetupParams, _prepared_test_ctx: TrezorTestContext
) -> int:
    """Returns instance_id of the loaded modular app."""
    if setup_params.extapp_instance_id is None:
        raise RuntimeError("Modular app was not loaded during test setup")
    return setup_params.extapp_instance_id


@pytest.fixture(scope="function")
def session(_prepared_test_ctx: TrezorTestContext) -> Session:
    return _prepared_test_ctx.get_session(passphrase="")


def _generate_python_protobuf() -> None:
    out_file = ROOT / "tests" / "generated" / "messages.py"
    out_file.parent.mkdir(parents=True, exist_ok=True)
    (out_file.parent / "__init__.py").touch(exist_ok=True)
    proto_dir = ROOT / "protob"
    subprocess.run(
        [
            sys.executable,
            str(proto_dir / "pb2py"),
            "--template",
            str(proto_dir / "messages.py.mako"),
            "--outfile",
            str(out_file),
            *[str(p) for p in sorted(proto_dir.glob("*.proto"))],
        ],
        cwd=ROOT,
        check=True,
    )


def pytest_sessionstart(session: pytest.Session) -> None:
    if session.config.getoption("ui"):
        raise pytest.UsageError("The Zcash app has no UI fixtures yet")
    _generate_python_protobuf()


def pytest_addoption(parser: "Parser") -> None:
    parser.addoption("--app", action="store", help="Path to the application to load")
    parser.addoption(
        "--lang",
        action="store",
        choices=translations.LANGUAGES,
        help="Run tests with a specified language: 'en' is the default",
    )
    parser.addoption("--ui", action="store", help="Not supported")
    parser.addoption("--ui-check-missing", action="store_true")
    parser.addoption("--do-master-diff", action="store_true")


def pytest_configure(config: "Config") -> None:
    config.addinivalue_line(
        "markers",
        'setup_client(mnemonic="all all all..."): the mnemonic to load',
    )
    verbosity = config.getoption("verbose")
    if verbosity:
        log.enable_debug_output(verbosity)
