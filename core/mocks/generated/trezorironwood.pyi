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


# rust/src/micropython/ironwood.rs
def session_begin(
    seed: bytes,
    network: int,
    account: int,
    host_reference_height: int,
    maximum_fee: int,
    expiry_window: int,
    pczt_length: int,
) -> None:
    """Start streaming one PCZT for the account derived from the wallet seed.
    Allocations of the signing core are carved from a boot-lifetime native
    region (no caller-provided buffer)."""


# rust/src/micropython/ironwood.rs
def session_feed(chunk: AnyBytes) -> tuple[int, int, tuple | None]:
    """Consume PCZT bytes. Returns (consumed, kind, payload): kind 0 needs more
    bytes; kind 1 is a payment output to confirm, payload
    (action_index, receiver, value, is_change); kind 2 is the review, payload
    (expiry_height, blocks_until_expiry, input_total, payment_total,
    change_total, fee, padding_outputs, payment_outputs). Unconsumed bytes
    must be fed again. ValueError: malformed; RuntimeError: rejected."""


# rust/src/micropython/ironwood.rs
def session_approve() -> None:
    """Record consent for the reviewed PCZT; call only after the trusted totals screen."""


# rust/src/micropython/ironwood.rs
def session_sign(seed: bytes) -> bytes:
    """Sign every real spend and end the session. Returns concatenated
    66-byte records: pool (0x03) | action_index | signature[64]."""


# rust/src/micropython/ironwood.rs
def session_cancel() -> None:
    """End the session, if any, and wipe its state."""


# rust/src/micropython/ironwood.rs
def session_region_high_water() -> int:
    """Bytes of the region used so far (device only; 0 on the emulator)."""


# rust/src/micropython/ironwood.rs
def bench(selector: int, region: bytearray, iters: int) -> int:
    """MEASUREMENT-ONLY. Run `iters` iterations of one crypto operation
    (0 warmup, 1 note_commitment, 2 sinsemilla_hash, 3 scalar_mul,
    4 commit_ivk) over the signing path's Sinsemilla/Pallas instances and
    return a folded accumulator so nothing is optimised away. `region`
    backs the allocations and must stay referenced, unresized, for the
    call. Time it with utime.ticks_ms on the Python side. Changes no
    signing behavior."""
