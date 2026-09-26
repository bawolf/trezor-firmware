# Scratch reproduction (not for commit): the host answers the Ethereum sample's
# data request (TxRequest) with an ExtAppMessage whose instance_id is not the
# running app's.
import io

import pytest
from trezorlib import protobuf
from trezorlib.exceptions import TrezorFailure
from trezorlib.messages import ExtAppMessage, ExtAppResponse
from trezorlib.tools import parse_path

from . import ethereum_ext
from .generated import messages as m

PATH = parse_path("m/44h/60h/0h/0/0")


def test_zz_answer_with_another_instance_id(session, instance_id):
    data = bytes(3000)
    ethereum_ext.call_ext(
        session,
        instance_id,
        msg_data=m.SignTx(
            address_n=PATH, nonce=b"\x00", gas_price=b"\x01", gas_limit=b"\x52\x08",
            to="0x1d1c328764a41bda0492b66baa30c4a339ff85ef", value=b"\x00",
            chain_id=1, data_length=len(data), data_initial_chunk=data[:1024],
        ),
        expect=[m.TxRequest],
    )
    buf = io.BytesIO()
    protobuf.dump_message(buf, m.TxAck(data_chunk=data[1024:2048]))
    ack = ExtAppMessage(
        instance_id=(instance_id + 1) % 2**32,
        message_id=int(m.MessageType.TxAck),
        data=buf.getvalue(),
    )
    with session:
        try:
            resp = session.client._call(session, ack, expect=ExtAppResponse)
            answer = protobuf.load_message(io.BytesIO(resp.data), m.TxRequest)
            print(f"\nOBSERVED: the app took the answer and asks for {answer.data_length} more bytes")
        except TrezorFailure as e:
            print(f"\nOBSERVED: {e}")
