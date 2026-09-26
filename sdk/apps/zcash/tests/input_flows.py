"""
Central place for defining all input flows for the device tests.

Each model has potentially its own input flow, and in most cases
we need to distinguish between them. Doing it at one place
offers a better overview of the differences and makes it easier
to maintain. The whole `device_tests` folder can then focus
only on the actual tests and data-assertions, not on the lower-level
input flow details.
"""

from __future__ import annotations

from collections.abc import Callable

from trezorlib import messages as trezor_messages
from trezorlib.debuglink import TrezorTestContext as Client
from trezorlib.testing import translations as TR
from trezorlib.testing.common import BRGeneratorType, get_text_possible_pagination

from .zcash_ext import Network

B = trezor_messages.ButtonRequestType


def address_pieces(screen: str, address: str) -> list[str]:
    """The runs of `address` on the screen, without labels like "Mainnet"."""
    return [word for word in screen.split() if word in address]


def is_shown_whole(screen: str, address: str) -> bool:
    return "".join(address_pieces(screen, address)) == address


def is_chunked(screen: str, address: str) -> bool:
    """Whether the address is shown in groups of four characters: the text of
    every line but the last is a whole number of groups."""
    pieces = address_pieces(screen, address)
    return all(len(piece) % 4 == 0 for piece in pieces[:-1])


class InputFlowBase:
    def __init__(self, client: Client):
        self.client = client
        self.debug = client.debug
        self.layout_type = client.layout_type

    def get(self) -> Callable[[], BRGeneratorType]:
        assert hasattr(self, "input_flow")
        return getattr(self, "input_flow")

    def text_content(self) -> str:
        return " ".join(self.debug.read_layout().text_content().split())

    def confirm_account(self, network: Network, account: int) -> BRGeneratorType:
        """Core's request to let the app use a Zcash account, the first time
        an app instance asks for its keys."""
        br = yield
        assert (br.code, br.name) == (B.Other, "zip32_orchard_account")
        network_name = {
            Network.Mainnet: TR.extapp__mainnet,
            Network.Testnet: TR.extapp__testnet,
        }[network]
        text = TR.extapp__spending_key_template.format(
            "Zcash", account + 1, network_name
        )
        assert text in self.text_content()
        self.debug.press_yes()

    def confirm_weak_backup(self) -> BRGeneratorType:
        """The ZIP-315 warning for a 12-word wallet."""
        br = yield
        assert (br.code, br.name) == (B.Warning, "zcash_weak_backup")
        self.debug.press_yes()

    def confirm_keys(
        self, network: Network, account: int, first_use: bool
    ) -> BRGeneratorType:
        if first_use:
            yield from self.confirm_account(network, account)
        yield from self.confirm_weak_backup()


class InputFlowShowAddress(InputFlowBase):
    def __init__(
        self,
        client: Client,
        network: Network = Network.Mainnet,
        account: int = 0,
        first_use: bool = True,
    ):
        super().__init__(client)
        self.network = network
        self.account = account
        self.first_use = first_use
        self.screen = ""
        self.subtitle = ""

    def input_flow(self) -> BRGeneratorType:
        yield from self.confirm_keys(self.network, self.account, self.first_use)
        br = yield
        assert (br.code, br.name) == (B.Address, "show_address")
        self.subtitle = self.debug.read_layout().subtitle()
        self.screen = get_text_possible_pagination(self.debug, br)
        self.debug.press_yes()


class InputFlowDeclineAccount(InputFlowBase):
    def input_flow(self) -> BRGeneratorType:
        br = yield
        assert br.name == "zip32_orchard_account"
        self.debug.press_no()


class InputFlowExportViewingKey(InputFlowBase):
    def __init__(
        self,
        client: Client,
        network: Network,
        account: int,
        include_seed_fingerprint: bool,
    ):
        super().__init__(client)
        self.network = network
        self.account = account
        self.include_seed_fingerprint = include_seed_fingerprint
        self.export_screen = ""
        self.fingerprint_screen = ""

    def input_flow(self) -> BRGeneratorType:
        br = yield
        assert (br.code, br.name) == (B.PublicKey, "zcash_export_viewing_key")
        self.export_screen = self.text_content()
        self.debug.press_yes()
        if self.include_seed_fingerprint:
            br = yield
            assert (br.code, br.name) == (B.Warning, "zcash_seed_fingerprint")
            self.fingerprint_screen = self.text_content()
            self.debug.press_yes()
        yield from self.confirm_keys(self.network, self.account, first_use=True)
