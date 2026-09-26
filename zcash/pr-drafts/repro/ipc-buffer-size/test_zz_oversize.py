# Scratch reproduction (not for commit): the Ethereum sample is built with
# `ipc-buffer-size = 2048` (manifest.patch). The host answers the app's data
# request with a TxAck larger than that inbox, then sends another request.
from trezorlib.tools import parse_path

from . import ethereum_ext
from .generated import messages as m

PATH = parse_path("m/44h/60h/0h/0/0")


def test_zz_oversized_host_message(session, instance_id):
    data = bytes(3000)
    ethereum_ext.call_ext(
        session,
        instance_id,
        msg_data=m.SignTx(
            address_n=PATH, nonce=b"\x00", gas_price=b"\x01", gas_limit=b"\x52\x08",
            to="0x1d1c328764a41bda0492b66baa30c4a339ff85ef", value=b"\x00",
            chain_id=1, data_length=len(data), data_initial_chunk=data[:512],
        ),
        expect=[m.TxRequest],
    )
    try:
        ethereum_ext.call_ext(
            session, instance_id,
            msg_data=m.TxAck(data_chunk=bytes(4096)), expect=[m.TxRequest],
        )
        print("\nOBSERVED: TxAck -> answered")
    except Exception as e:
        print(f"\nOBSERVED: TxAck -> {type(e).__name__}: {e}")
    try:
        ethereum_ext.call_ext(
            session, instance_id,
            msg_data=m.GetAddress(address_n=PATH), expect=[m.Address],
        )
        print("OBSERVED: GetAddress -> answered")
    except Exception as e:
        print(f"OBSERVED: GetAddress -> {type(e).__name__}: {e}")
