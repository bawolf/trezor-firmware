"""MEASUREMENT-ONLY Ironwood latency instrumentation.

This module is frozen into the firmware ONLY when it is built with the
`ironwood-measurement` Cargo feature (`xtask ... --ironwood-measurement`),
mirroring the native side: that feature also exposes the `bench` and
`session_region_high_water` bindings. A PRODUCTION build (default) freezes no
copy of this module, so `sign_pczt` / `get_address` take their `ImportError`
paths and carry no timing code at all — no `utime.ticks_ms` in the signing
loop, no bench body in the address app.

Everything here times COMPUTE only (the native calls), never the UI / button
waits, and changes no signing behavior.
"""

import utime
from micropython import const

# The full boot-lifetime signing region size: the denominator for the
# high-water ratios (mirrors ironwood_allocator.rs).
_REGION_BYTES = const(96 * 1024)


class Timings:
    """Per-phase on-device wall-clock for one streamed signing session.

    All phases are Python-driven, so `utime.ticks_ms` around the native calls
    captures the on-device wall-clock spent in each. `sign_pczt._stream_and_sign`
    calls the paired start/stop hooks; in a production build no `Timings` is
    constructed, so the signing loop runs with none of this bookkeeping.
    """

    def __init__(self) -> None:
        self.derive_ms = 0
        self.feed_ms = 0  # cumulative wall-clock inside session_feed (compute only)
        self.feed_seg_ms = []  # per-step feed compute, split at each output / the review
        self.sign_ms = 0
        self._t = 0

    def begin_start(self) -> None:
        self._t = utime.ticks_ms()

    def begin_done(self) -> None:
        self.derive_ms = utime.ticks_diff(utime.ticks_ms(), self._t)

    def feed_start(self) -> None:
        self._t = utime.ticks_ms()

    def feed_done(self) -> None:
        self.feed_ms += utime.ticks_diff(utime.ticks_ms(), self._t)

    def segment(self) -> None:
        # Close the current feed segment at each output and at the review step.
        self.feed_seg_ms.append(self.feed_ms - sum(self.feed_seg_ms))

    def sign_start(self) -> None:
        self._t = utime.ticks_ms()

    def sign_done(self) -> None:
        self.sign_ms = utime.ticks_diff(utime.ticks_ms(), self._t)

    def trailer(self, action_count: int) -> bytes:
        # An ASCII key=value breakdown of the per-phase on-device wall-clock plus
        # the region counters, so a plain host (no DebugLink) reads where the
        # ~35 s first-review and ~103 s sign time go.
        #
        # SWEEP PROTOCOL: run ascending N (2, 4, 8, 16, 32) on ONE boot.
        # session_peak is per-session (resets at session_begin), so it should
        # stay ~flat (~45,760 B) across N — that is the O(1)-RAM claim.
        # in_use_at_begin is the persistent set (Pasta table + orchard OnceBox
        # caches) already allocated when each session began; after session 1 it
        # must stay CONSTANT — any upward drift is a cross-session leak.
        # boot_peak is the boot-monotone max (only grows).
        from trezorironwood import session_region_high_water

        session_peak, in_use_at_begin, boot_peak = session_region_high_water()
        # `action_count` is the full bundle count (payments + change + padding)
        # from the review totals, NOT the payment count, so the trailer figure
        # matches the 32-action guardrail cap.
        debug_timings = (
            "derive_ms=%d feed_ms=%d sign_ms=%d action_count=%d "
            "feed_seg_ms=%s session_peak_bytes=%d in_use_at_begin_bytes=%d "
            "boot_peak_bytes=%d region_bytes=%d"
            % (
                self.derive_ms,
                self.feed_ms,
                self.sign_ms,
                action_count,
                ",".join(str(x) for x in self.feed_seg_ms),
                session_peak,
                in_use_at_begin,
                boot_peak,
                _REGION_BYTES,
            )
        ).encode()
        if __debug__:
            from trezor import log

            log.debug(__name__, "ironwood latency: %s", debug_timings)
        print("ironwood latency:", debug_timings)  # JTAG/emulator log readout
        return debug_timings


def op_timing_bench() -> str:
    # Per-operation latency bench. Times the two candidate per-action
    # verification costs in isolation, so a hardware run can attribute the ~23 s
    # per real spend to Sinsemilla hashing versus Pallas scalar multiplication.
    # Each op is one native call timed with utime.ticks_ms here on the Python
    # side; the native side returns a folded accumulator so nothing is optimised
    # away. Uses the same computed-generator Sinsemilla and pinned Pasta the
    # signing path links. Changes no signing behavior.
    from trezorironwood import bench, session_region_high_water

    # `bench` ignores this argument (it allocates from the boot-lifetime `.buf`
    # region the signing path installs), so pass an EMPTY bytearray. A 96 KiB GC
    # bytearray here would be dead weight — a single contiguous allocation from
    # the now-138.7 KiB heap that can raise MemoryError under fragmentation for a
    # reason unrelated to the bench.
    region = bytearray()
    slow_iters = 6  # Sinsemilla ops cost seconds each under computed generators
    fast_iters = 200  # scalar mult is milliseconds

    # Warm one-time init (Pasta sqrt table, domain generator derivation) off the
    # clock so it is not billed to the first timed op.
    bench(0, region, 1)

    def _time(selector, iters):
        start = utime.ticks_ms()
        acc = bench(selector, region, iters)
        elapsed = utime.ticks_diff(utime.ticks_ms(), start)
        return elapsed / iters, acc

    nc_ms, nc_acc = _time(1, slow_iters)  # note commitment (hash + blinding)
    sh_ms, sh_acc = _time(2, slow_iters)  # Sinsemilla hash only
    sm_ms, sm_acc = _time(3, fast_iters)  # Pallas variable-base scalar mult
    iv_ms, iv_acc = _time(4, slow_iters)  # commit_ivk (FVK derivation)
    session_peak, _in_use_at_begin, boot_peak = session_region_high_water()

    result = (
        "ironwood op timing slow_iters=%d fast_iters=%d "
        "note_commitment_ms=%.2f sinsemilla_hash_ms=%.2f blinding_mult_ms=%.2f "
        "scalar_mul_ms=%.4f commit_ivk_ms=%.2f "
        "session_peak=%d boot_peak=%d acc=%d,%d,%d,%d"
        % (
            slow_iters,
            fast_iters,
            nc_ms,
            sh_ms,
            nc_ms - sh_ms,
            sm_ms,
            iv_ms,
            session_peak,
            boot_peak,
            nc_acc,
            sh_acc,
            sm_acc,
            iv_acc,
        )
    )
    if __debug__:
        from trezor import log

        log.debug(__name__, "%s", result)
    print("ironwood op timing:", result)
    return result
