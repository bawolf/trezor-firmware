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

from trezorlib import messages, zcash
from trezorlib.debuglink import DebugSession as Session

from .common import MNEMONIC, NETWORKS, vector

B = messages.ButtonRequestType

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.zcash,
    pytest.mark.capabilities(messages.Capability.Zcash_Shielded),
    pytest.mark.models("t3b1", "t3t1", "t3w1"),
    pytest.mark.setup_client(mnemonic=MNEMONIC),
]


def test_device_fvk_matches_fixture(session: Session) -> None:
    """The signing derivation (orchard zip32) and the viewing export (receive
    crate) agree, and the default export releases no seed fingerprint."""
    parameters, result = vector("2_actions")

    def accept(session: Session):
        br = yield
        assert br.code == B.SignTx
        assert br.name == "zcash_export_viewing_key"
        session.debug.press_yes()
        # The weak-backup warning; there is no seed-fingerprint screen
        # because the host did not ask for the fingerprint.
        br = yield
        assert br.code == B.Warning
        assert br.name == "zcash_weak_backup"
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
    parameters, result = vector("2_actions")
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
        br = yield
        assert br.code == B.Warning
        assert br.name == "zcash_weak_backup"
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
    # the device computes it natively (trezorzcash).
    assert export.seed_fingerprint.hex() == result["seed_fingerprint"]
    assert len(export.seed_fingerprint) == 32
