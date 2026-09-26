"""Seed an emulator with the "all all ... all" test seed, through debuglink.

Run it against a PYOPT=0 build of the same tree, on the profile the PYOPT=1
build will then use (a PYOPT=1 build has no debuglink).

Usage (from core/): uv run python seed.py udp:127.0.0.1:21324
"""

import sys

from trezorlib import debuglink
from trezorlib.debuglink import TrezorTestContext
from trezorlib.transport import get_transport

client = TrezorTestContext(get_transport(sys.argv[1]))
debuglink.load_device(
    client.get_seedless_session(),
    mnemonic=" ".join(["all"] * 12),
    pin=None,
    passphrase_protection=False,
    label="test",
)
print("seeded")
