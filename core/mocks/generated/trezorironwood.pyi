from typing import *
from buffer_types import *


# rust/src/micropython/ironwood.rs
def derive_receiver(
    seed: AnyBytes,
    network: int,
    account: int,
    diversifier_index: AnyBytes,
) -> bytes:
    """Internal synchronous adapter; seed must come from device wallet state."""


# rust/src/micropython/ironwood.rs
def derive_viewing_key(
    seed: bytes,
    network: int,
    account: int,
    output: AnyBuffer,
) -> None:
    """Fill a 96-byte Orchard FVK buffer from device wallet state."""
