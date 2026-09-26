"""Load the Ethereum sample on a PYOPT=1 emulator (no debuglink) and ask it for
a public key over the normal wire. GetPublicKey shows no screen.

Copy it to sdk/apps/ethereum and run it there, after one pytest run of the
sample has generated tests/generated/messages.py:
  uv run python repro_pyopt_getpublickey.py udp:127.0.0.1:21324 ../target/artifacts/t3t1-emu/ethereum.elf
"""

import io
import sys
import time
from pathlib import Path

from trezorlib import extapp, messages, protobuf
from trezorlib.client import AppManifest, get_client
from trezorlib.exceptions import TrezorFailure
from trezorlib.tools import parse_path
from trezorlib.transport import get_transport

from tests.generated import messages as eth

path, elf = sys.argv[1], Path(sys.argv[2])


def connect():
    client = get_client(AppManifest(app_name="repro"), get_transport(path))
    return client.get_session(passphrase=None)


session = connect()
instance_id = extapp.load(
    session,
    elf.read_bytes(),
    elf.with_suffix(".proof").read_bytes(),
    (elf.parent / "rootpacket_0-timestamped-signed.tmr").read_bytes(),
    None,
)

buf = io.BytesIO()
protobuf.dump_message(buf, eth.GetPublicKey(address_n=parse_path("m/44h/60h/0h")))
request = messages.ExtAppMessage(
    instance_id=instance_id,
    message_id=int(eth.MessageType.GetPublicKey),
    data=buf.getvalue(),
)
try:
    response = session.call(request, expect=messages.ExtAppResponse, timeout=20)
    xpub = protobuf.load_message(io.BytesIO(response.data), eth.PublicKey).xpub
    print(f"GetPublicKey: {xpub[:16]}...")
except TrezorFailure as e:
    print(f"GetPublicKey failed: {e}")
except Exception as e:
    print(f"GetPublicKey got no answer: {type(e).__name__}: {e}")

time.sleep(2)
try:
    connect().call(messages.GetFeatures(), expect=messages.Features, timeout=10)
    print("GetFeatures: Core answers")
except Exception as e:
    print(f"GetFeatures: {type(e).__name__}: {e}")
