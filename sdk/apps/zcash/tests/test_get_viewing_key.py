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

"""ZcashGetViewingKey on the emulator."""

import pytest

from trezorlib import messages
from trezorlib.debuglink import DebugSession as Session

from . import zcash_ext
from .common import (
    MAINNET_FVK,
    MNEMONIC,
    SEED_FINGERPRINT,
    TESTNET_FVK,
    accept_account_request,
)
from .generated.messages import ZcashNetwork

B = messages.ButtonRequestType

pytestmark = [pytest.mark.setup_client(mnemonic=MNEMONIC)]


@pytest.mark.parametrize(
    "network, label, prefix, fvk, path",
    [
        (ZcashNetwork.Mainnet, "Mainnet", "uview1", MAINNET_FVK, "m/32'/133'/0'"),
        (ZcashNetwork.Testnet, "Testnet", "uviewtest1", TESTNET_FVK, "m/32'/1'/0'"),
    ],
)
def test_device_fvk_matches_fixture(
    session: Session,
    instance_id: int,
    network: ZcashNetwork,
    label: str,
    prefix: str,
    fvk: str,
    path: str,
) -> None:
    """The exported UFVK carries the fixture's FVK, and the default export
    releases no seed fingerprint."""
    shown = []

    def accept(session: Session):
        br = yield
        assert br.code == B.SignTx
        assert br.name == "zcash_export_viewing_key"
        shown.append(session.debug.read_layout().text_content())
        session.debug.press_yes()
        # There is no seed-fingerprint screen because the host did not ask
        # for the fingerprint. Core's account request, then the weak-backup
        # warning.
        yield from accept_account_request(session, 0, label)
        br = yield
        assert br.code == B.Warning
        assert br.name == "zcash_weak_backup"
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        export = zcash_ext.export_viewing_key(session, instance_id, network, 0)

    assert export.key.startswith(prefix)
    assert zcash_ext.orchard_fvk(export.key).hex() == fvk
    assert export.seed_fingerprint is None
    assert path in shown[0]


def test_seed_fingerprint_export_is_opt_in(session: Session, instance_id: int) -> None:
    """Asking for the seed fingerprint inserts a screen that says what it links."""
    shown = []

    def accept(session: Session):
        br = yield
        assert br.code == B.SignTx
        assert br.name == "zcash_export_viewing_key"
        session.debug.press_yes()
        br = yield
        assert br.code == B.Warning
        assert br.name == "zcash_seed_fingerprint"
        shown.append(session.debug.read_layout().text_content())
        session.debug.press_yes()
        yield from accept_account_request(session)
        br = yield
        assert br.code == B.Warning
        assert br.name == "zcash_weak_backup"
        session.debug.press_yes()

    with session.test_ctx as client:
        client.set_input_flow(accept(session))
        export = zcash_ext.export_viewing_key(
            session, instance_id, ZcashNetwork.Mainnet, 0, include_seed_fingerprint=True
        )

    assert "recovery seed" in " ".join(shown[0].split())
    assert export.seed_fingerprint.hex() == SEED_FINGERPRINT
    assert zcash_ext.orchard_fvk(export.key).hex() == MAINNET_FVK
