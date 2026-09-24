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

import pytest

from trezorlib import messages
from trezorlib.debuglink import DebugSession as Session

pytestmark = [
    pytest.mark.altcoin,
    pytest.mark.zcash,
    pytest.mark.models("t3b1", "t3t1", "t3w1"),
]


def test_zcash_shielded_capability(session: Session) -> None:
    """The one test here that does not skip on a stock build.

    Every other test in this directory skips when the capability is missing,
    so without this one a run against the wrong firmware would look like a
    pass.
    """
    assert messages.Capability.Zcash_Shielded in session.features.capabilities
