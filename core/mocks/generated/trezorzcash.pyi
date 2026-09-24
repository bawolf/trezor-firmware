from typing import *
from buffer_types import *
SCRATCH_BYTES: int


# rust/src/micropython/zcash.rs
def derive_receiver(
    seed: AnyBytes,
    network: int,
    account: int,
    diversifier_index: AnyBytes,
) -> bytes:
    """Internal synchronous adapter; seed must come from device wallet state."""


# rust/src/micropython/zcash.rs
def derive_viewing_key(
    seed: bytes,
    network: int,
    account: int,
    output: AnyBuffer,
) -> None:
    """Fill a 96-byte Orchard FVK buffer from device wallet state."""


# rust/src/micropython/zcash.rs
def seed_fingerprint(seed: bytes, output: AnyBuffer) -> None:
    """Fill a 32-byte buffer with the ZIP-32 seed fingerprint of the wallet
    seed: a public identifier of the seed, not key material."""


# rust/src/micropython/zcash.rs
def debug_region_info() -> tuple[int, int, int, int] | None:
    """(persist_in_use, persist_peak, scratch_in_use, scratch_peak) of
    the two signing arenas on a debuglink DEVICE build, None otherwise
    -- including on the emulator, which has no arenas to report. An
    absent reading must be skipped, not read as zeros."""
