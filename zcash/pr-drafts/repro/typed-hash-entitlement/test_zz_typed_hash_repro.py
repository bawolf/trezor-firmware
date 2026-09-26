# Scratch reproduction (not for commit): with get_address.patch applied, the
# Tron sample (which declares only m/44'/195'/...) asks Core's SignTypedHash for
# a signature at m/44'/60'/0'/0/0 when it gets an empty path.
import pytest
from ecdsa import SECP256k1, VerifyingKey
from ecdsa.util import sigdecode_string
from trezorlib import ethereum
from trezorlib.exceptions import TrezorFailure
from trezorlib.tools import parse_path

from . import tron_ext

ETH_PATH = parse_path("m/44h/60h/0h/0/0")
HASH = bytes([0x11] * 32)


def test_tron_app_cannot_sign_for_ethereum(session, instance_id):
    try:
        response = tron_ext.get_authenticated_address(session, instance_id, [])
    except TrezorFailure as e:
        print(f"refused: {e}")
        return
    signature = response.mac
    node = ethereum.get_public_node(session, ETH_PATH).node
    verifies = VerifyingKey.from_string(node.public_key, curve=SECP256k1).verify_digest(
        signature[1:], HASH, sigdecode=sigdecode_string
    )
    pytest.fail(
        f"the Tron app got a signature by {ethereum.get_address(session, ETH_PATH)} "
        f"(m/44'/60'/0'/0/0) over {HASH.hex()}: {signature.hex()}; verifies: {verifies}"
    )
