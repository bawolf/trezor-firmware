from typing import *
from buffer_types import *
SCRATCH_BYTES: int


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


# rust/src/micropython/ironwood.rs
def seed_fingerprint(seed: bytes, output: AnyBuffer) -> None:
    """Fill a 32-byte buffer with the ZIP-32 seed fingerprint of the wallet
    seed: a public identifier of the seed, not key material."""


# rust/src/micropython/ironwood.rs
def session_begin(
    seed: bytes,
    network: int,
    account: int,
    host_reference_height: int,
    maximum_fee: int,
    expiry_window: int,
    pczt_length: int,
    scratch: AnyBuffer,
) -> int:
    """Start streaming one PCZT for the account derived from the wallet seed.
    Returns the session handle, which `session_feed`, `session_approve` and
    `session_sign` require: it binds the native request to the workflow that
    began it, so a second request cannot adopt this one.
    `scratch` is a writable buffer of at least SCRATCH_BYTES that the
    caller must keep referenced until it has called `session_cancel`;
    the session's own allocations are carved from it, while what must
    outlive the session stays in a boot-lifetime native region.
    ValueError: the scratch buffer is too small."""


# rust/src/micropython/ironwood.rs
def session_feed(handle: int, chunk: AnyBytes) -> tuple[int, int, tuple | None]:
    """Consume PCZT bytes. Returns (consumed, kind, payload): kind 0 needs more
    bytes; kind 1 is a payment output to confirm, payload
    (action_index, receiver, value, memo_kind, memo) where
    memo_kind 0 is no memo (memo None), 1 a text memo (memo: its UTF-8
    bytes, at most 256) and 2 a memo not shown verbatim (memo: the 32-byte
    BLAKE2b-256 of the memo); kind 2 is the review, payload
    (expiry_height, blocks_until_expiry, input_total, payment_total,
    change_total, transparent_total, fee, padding_outputs,
    payment_outputs, transparent_outputs, action_count); kind 3 is a
    transparent output to confirm, payload (index, kind, hash, value)
    where kind 0 is a public key hash and 1 a script hash, and hash is
    the 20 bytes the Base58Check address encodes.
    Unconsumed bytes must be fed again. A kind 3 may consume nothing:
    transparent outputs are held until the action count is known, and
    are then released one per call. ValueError: malformed / too many
    actions; RuntimeError: rejected."""


# rust/src/micropython/ironwood.rs
def session_approve(handle: int) -> None:
    """Record consent for the reviewed PCZT; call only after the trusted totals screen."""


# rust/src/micropython/ironwood.rs
def session_sign(handle: int, seed: bytes) -> bytes:
    """Sign every real spend and end the session. Returns concatenated
    66-byte records: pool (0x03) | action_index | signature[64]."""


# rust/src/micropython/ironwood.rs
def session_cancel(handle: int | None = None) -> None:
    """End the session and wipe its state. With a handle, only that
    handle's session is ended, so a workflow cannot tear down a session
    that is no longer its own. Without one, whatever is live is ended:
    teardown runs from a `finally` that may have no handle yet, and
    autolock unwinds the workflow with a GeneratorExit from outside.
    Once nothing is live the scratch buffer is wiped and given back, so
    the caller may drop its reference after this returns."""


# rust/src/micropython/ironwood.rs
def debug_region_info() -> tuple[int, int, int, int] | None:
    """(persist_in_use, persist_peak, scratch_in_use, scratch_peak) of
    the two signing arenas on a debuglink build, None otherwise."""
