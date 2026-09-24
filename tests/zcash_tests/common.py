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

"""Shared pieces of the Zcash shielded device tests.

These tests need firmware built with `ZCASH_SHIELDED=1` (T3B1, T3T1, T3W1).
They live in their own directory, and their UI fixtures in their own group,
so the stock device-test jobs never see them; `make test_emu_zcash_ui` runs
them with `--ui-check-missing` against a `--zcash-shielded` emulator.
"""

import json

from trezorlib import messages
from trezorlib.debuglink import DebugSession as Session
from trezorlib.debuglink import LayoutType

from ..common import COMMON_FIXTURES_DIR

MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
NETWORKS = {
    "mainnet": messages.ZcashNetwork.Mainnet,
    "testnet": messages.ZcashNetwork.Testnet,
}


def screen_text(session: Session) -> str:
    """Everything the current screen shows, paging where the model needs it.

    On T3B1 (caesar, 128x64) a long value spills onto later pages, and the
    right press that advances a page is the same press that confirms on the
    last one. So on caesar read every page and stop on the last, leaving the
    caller's `press_yes` to confirm. delizia and eckhart show the value on the
    first page and are left alone.
    """
    debug = session.debug
    layout = debug.read_layout()
    shown = layout.text_content()
    if debug.layout_type is LayoutType.Caesar:
        for _ in range(layout.page_count() - 1):
            debug.press_right()
            shown += " " + debug.read_layout().text_content()
    return shown


def address_pieces(screen: str, address: str) -> list[str]:
    """The runs of `address` on the screen, dropping labels like "Mainnet"."""
    return [word for word in screen.split() if word and word in address]


def vector(name: str) -> tuple[dict, dict]:
    """One checked-in signing vector by name, for flows parametrization cannot express."""
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
