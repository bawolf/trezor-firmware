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

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session
from trezorlib.debuglink import LayoutType

from ..common import COMMON_FIXTURES_DIR

MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
NETWORKS = {
    "mainnet": messages.ZcashNetwork.Mainnet,
    "testnet": messages.ZcashNetwork.Testnet,
}


def screen_text(session: Session) -> str:
    """Everything the current screen shows, following a long value across pages.

    Each model continues a long value differently: caesar (T3B1) and eckhart
    (T3W1) report a page count and advance with the right button and the
    action bar's right button; a delizia (T3T1) page that ends in "..." is
    continued by a tap. This stops on the value's last page, where the caller's
    `press_yes` confirms.
    """
    debug = session.debug
    layout = debug.read_layout()
    shown = layout.text_content()
    if debug.layout_type is LayoutType.Delizia:
        while layout.screen_content().endswith("..."):
            debug.click(debug.screen_buttons.tap_to_confirm())
            layout = debug.read_layout()
            shown += " " + layout.text_content()
        return shown
    for _ in range(layout.page_count() - 1):
        if debug.layout_type is LayoutType.Caesar:
            debug.press_right()
        else:
            debug.click(debug.screen_buttons.actionbar_right())
        shown += " " + debug.read_layout().text_content()
    return shown


def address_pieces(screen: str, address: str) -> list[str]:
    """The runs of `address` on the screen, dropping labels like "Mainnet"."""
    return [word for word in screen.split() if word and word in address]


def unified_address(receivers: dict[int, bytes], hrp: str) -> str:
    """A ZIP-316 unified address for `receivers` (typecode -> receiver bytes),
    built on the host for vectors that need one the wallet did not make."""
    items = b"".join(
        bytes((typecode, len(receiver))) + receiver
        for typecode, receiver in sorted(receivers.items())
    )
    message = bytearray(items + hrp.encode() + bytes(16 - len(hrp)))
    zcash._f4jumble(message, inverse=False)
    return zcash._bech32m_encode(
        hrp, zcash._convert_bits(list(message), 8, 5, pad=True)
    )


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
