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

"""Shared pieces of the Zcash app's device tests."""

from trezorlib import messages
from trezorlib.debuglink import DebugSession as Session
from trezorlib.debuglink import LayoutType

from .generated.messages import ZcashNetwork

# The app gets its keys from Core's key service, which derives them from the
# seed of the mnemonic the harness loads, so every vector below is a statement
# about the whole chain: device seed, Core's ZIP-32 derivation, the app.
MNEMONIC = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"

# Computed with upstream `orchard` 0.15.5, `zcash_address` 0.13 and `zip32`
# 0.2.1 (crates.io) from the BIP-39 seed of MNEMONIC.
SEED_FINGERPRINT = "21ed3d7882c7e37fe012b54a6408048048cb09782d4b2938617da793ccd27815"
MAINNET_FVK = (
    "4c9c066f081a62eceb7bf8195e442352fab1fdbe5774b3fce5ee6357e201790c"
    "c59c63ef351379e9438626c1ce720f3233af7a3bef8c2f2a7fc74471dd5b133a"
    "9945a045cd6e9ca8d827d039a59a6dc33d1995cc429b3de887d47694a49f883e"
)
TESTNET_FVK = (
    "6cb31f4dac3d1c8a4ba98ab750a3d592ddd69e1a7ad14f2ad34f144e346cdb3e"
    "6d18e6916558f56738f5df2abcf6a177adabe16b5dbf1782423dd65f2d6b9b34"
    "d5335adea30337d22d0d3e9480f96f950d433bbed20ff68e023bbf287a07c53f"
)
# A second wallet, the harness's default "all all ... all", computed the same
# way: mainnet account 0, diversifier index 0.
ALL_MNEMONIC = " ".join(["all"] * 12)
ALL_MAINNET_ADDRESS = "u1uzslnccvrw4r2y2kgjz7fm477xcnzge9z45scm4e6l6c63ren0ru29teedxw5vxu7c8xchp3ec2pu3wkgldc5zphwtm4w3fchcwrl26c"
# The external Orchard address at (network, account, diversifier index).
ADDRESSES = {
    (
        ZcashNetwork.Mainnet,
        0,
        0,
    ): "u1y2z9wqt9du4stq2keex78l4vvlkfh3c0n7le0pz80lc4ttcuz5h9qyts73awns77lkgw8zy67qwf0s86rauwg6e9wz7te7yf6vxjtk5g",
    (
        ZcashNetwork.Mainnet,
        0,
        1,
    ): "u17vtyr8qvuv55znf7cuh33yxt9p090fxd46urhx9h07c5xuqvpgjhudaekac6k80lu9953hu2q99d4rk7a9ue5hejqmlgjvpacq0c74xq",
    (
        ZcashNetwork.Mainnet,
        1,
        0,
    ): "u1gy8cry4mumdktu4yvuucdcyrpgsce38cnqszldwcq72s06aqtj0805wjtwuvv8a4ggu5jpy2662cjfurwwr4rfvjyjs3whm6f5g4dgzl",
    (
        ZcashNetwork.Testnet,
        0,
        0,
    ): "utest16gxxqp2n35ze6p2qzlxx2sd0e9s6favez0spnxe6f8useeew37hkm5zrk90373268u70e3l6en9dav8ytzp2q5lnj4gvjcprkg63xcxc",
}


def screen_text(session: Session) -> str:
    """Everything the current screen shows, following a long value across pages.

    Each model continues a long value differently: eckhart (T3W1) reports a
    page count and advances with the action bar's right button (`ok`); a
    delizia (T3T1) page that ends in "..." is continued by a tap. This stops on
    the value's last page, where the caller's `press_yes` confirms.
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
        debug.click(debug.screen_buttons.ok())
        shown += " " + debug.read_layout().text_content()
    return shown


def address_pieces(screen: str, address: str) -> list[str]:
    """The runs of `address` on the screen, dropping labels like "Mainnet"."""
    return [word for word in screen.split() if word and word in address]


def no_screens(session: Session):
    """An input flow that fails the test on any ButtonRequest."""
    br = yield
    raise AssertionError(f"unexpected screen {br.name}")


def accept_account_request(
    session: Session, account: int = 0, network: str = "Mainnet"
):
    """Core's one-time request to let the app use a Zcash account: the first
    time an app instance asks for an account's keys on a network."""
    br = yield
    assert (br.code, br.name) == (
        messages.ButtonRequestType.Other,
        "zip32_orchard_account",
    )
    text = " ".join(session.debug.read_layout().text_content().split())
    assert "Zcash account" in text
    assert f"#{account + 1}" in text
    assert network in text
    session.debug.press_yes()
