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
) -> None:
    """Start streaming one PCZT for the account derived from the wallet seed.
    Allocations of the signing core are carved from a boot-lifetime native
    region (no caller-provided buffer)."""


# rust/src/micropython/ironwood.rs
def session_feed(chunk: AnyBytes) -> tuple[int, int, tuple | None]:
    """Consume PCZT bytes. Returns (consumed, kind, payload): kind 0 needs more
    bytes; kind 1 is a payment output to confirm, payload
    (action_index, receiver, value, is_change, memo_kind, memo) where
    memo_kind 0 is no memo (memo None), 1 a text memo (memo: its UTF-8
    bytes, at most 256) and 2 a memo not shown verbatim (memo: the 32-byte
    BLAKE2b-256 of the memo); kind 2 is the review, payload
    (expiry_height, blocks_until_expiry, input_total, payment_total,
    change_total, fee, padding_outputs, payment_outputs, action_count).
    Unconsumed bytes must be fed again. ValueError: malformed / too many
    actions; RuntimeError: rejected."""


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
def session_region_high_water() -> tuple[int, int, int]:
    """MEASUREMENT-ONLY (ironwood-measurement). Region measurement counters,
    in bytes from the region base (all 0 on the emulator):
    (per_session_peak, in_use_at_begin, boot_peak). per_session_peak resets
    at each session_begin, so on an ascending-N single-boot sweep it stays
    ~flat; in_use_at_begin is the persistent set already allocated when the
    session began (constant in steady state — any drift is a cross-session
    leak); boot_peak is the boot-monotone maximum."""


# rust/src/micropython/ironwood.rs
def bench(selector: int, region: bytearray, iters: int) -> int:
    """MEASUREMENT-ONLY (ironwood-measurement). Run `iters` iterations of one
    crypto operation (0 warmup, 1 note_commitment, 2 sinsemilla_hash,
    3 scalar_mul, 4 commit_ivk) over the signing path's Sinsemilla/Pallas
    instances and return a folded accumulator so nothing is optimised away.
    `region` is accepted for API compatibility but IGNORED: the bench
    allocates from the same boot-lifetime `.buf` region the signing path
    installs, so an empty bytearray() is fine. Time it with utime.ticks_ms
    on the Python side. Changes no signing behavior."""
