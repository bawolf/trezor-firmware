from typing import *
from buffer_types import *


# rust/src/crypto/api/firmware_micropython.rs
def send_crypto_result(
    *,
    result: CryptoResult,
    ipc_cb: Callable[[bytes], None],
) -> None:
    """Serialize a crypto result (e.g. CryptoResult) into bytes and send it back via the ipc_cb callback."""


# rust/src/crypto/api/firmware_micropython.rs
def deserialize_crypto_message(
    *,
    data: bytes,
    message_id: int,
) -> Obj:
    """Validate a crypto request for operation `message_id` and return its
    fields as a MicroPython object. Raises ValueError if it is malformed or
    another operation."""
