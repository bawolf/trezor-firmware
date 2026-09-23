# flake8: noqa: F403,F405
from common import *  # isort:skip

import sys

from mock import patch
from trezor import TR, utils

if not utils.BITCOIN_ONLY:
    from trezor.messages import ZcashPcztAck
    from trezor.ui.layouts import progress as progress_module

    from apps.zcash import ironwood_account, sign_pczt

# Event kinds recorded on one shared timeline, in the order they happen.
REPORT = "report"
BEGIN = "begin"
SIGN = "sign"
WIPE = "wipe"
FEED = "feed"
PARK_IN = "park_in"
PARK_OUT = "park_out"

CHUNK = 1024
FEEDS_PER_CHUNK = 2
CONSUMED = CHUNK // FEEDS_PER_CHUNK
CHUNKS = 4
PCZT_LENGTH = CHUNK * CHUNKS

_RECEIVER = bytes(range(43))
# The review payload `_confirm_totals` is handed; it is patched out here.
_TOTALS = (10_000_040, 40, 500_000, 300_000, 180_000, 20_000, 2, 2, 8)


class _FakeIronwood:
    """Stands in for the native `trezorironwood` module.

    `_stream_and_sign` does `from trezorironwood import ...` at call time, so
    putting this in `sys.modules` is enough; nothing here computes anything.
    Each `session_feed` consumes a fixed slice so the loop is fully scripted.
    """

    def __init__(self, events, steps) -> None:
        self.events = events
        self.steps = list(steps)
        self.begun = False
        self.approved = False
        self.cancelled = []
        self.handle = 0

    def session_begin(self, *args) -> int:
        self.begun = True
        self.events.append((BEGIN, None))
        self.handle += 1
        return self.handle

    def session_feed(self, handle, view):
        assert handle == self.handle, "feed drove another workflow's session"
        self.events.append((FEED, len(self.steps)))
        return self.steps.pop(0)

    def session_approve(self, handle) -> None:
        assert handle == self.handle, "approve consented to another workflow's session"
        self.approved = True

    def session_sign(self, handle, seed) -> bytes:
        assert handle == self.handle, "sign released another workflow's signatures"
        self.events.append((SIGN, None))
        return bytes(sign_pczt.RECORD_LEN)

    def session_cancel(self, handle=None) -> None:
        # The handler cancels by name once it has one: a teardown that named
        # someone else's session would be a way to cancel the running one.
        assert handle in (None, self.handle), "cancel named another session"
        self.cancelled.append(handle)


class _RecordingProgress:
    """Stands in for `ui.ProgressLayout`, recording every `report()`."""

    def __init__(self, events, description) -> None:
        self.events = events
        self.description = description

    def report(self, value: int, description=None) -> None:
        self.events.append((REPORT, value))


@unittest.skipUnless(not utils.BITCOIN_ONLY, "altcoin")
class TestIronwoodSignPcztProgress(unittest.TestCase):
    """The progress layout is what keeps a long sign alive and visible.

    `_stream_and_sign` no longer calls `workflow.idle_timer.touch()` itself: it
    relies on `ProgressLayout.report`, which touches the timer before it paints
    (trezor/ui/__init__.py). Two properties carry that argument, so assert them
    here, where they cost nothing:

      1. A report precedes every `session_feed`, so at least one fires per host
         chunk -- strictly more often than the per-chunk touch it replaced, and
         it is also what lights the screen back up after a confirmation.
      2. No report fires while the workflow is parked on a ButtonRequest, so an
         abandoned sign still reaches autolock.
    """

    def setUp(self):
        self.patchers = []
        self.events = []
        self.layouts = []
        self.wallet_seed = bytes(range(32))

        self.native = _FakeIronwood(self.events, self._steps())
        self.real_module = sys.modules.get("trezorironwood")
        sys.modules["trezorironwood"] = self.native

        self._patch(progress_module, "progress", self._progress)
        self._patch(sign_pczt, "_call", self._call)
        self._patch(sign_pczt, "_confirm_output", self._confirm_output)
        self._patch(sign_pczt, "_confirm_memo", self._confirm_memo)
        self._patch(sign_pczt, "_confirm_totals", self._confirm_totals)
        self._patch(ironwood_account, "require_session", lambda session: None)
        self._patch(utils, "zero_unused_stack", self._wipe)

    def tearDown(self):
        for patcher in reversed(self.patchers):
            patcher.__exit__(None, None, None)
        if self.real_module is None:
            del sys.modules["trezorironwood"]
        else:
            sys.modules["trezorironwood"] = self.real_module

    def _patch(self, obj, attr, value):
        patcher = patch(obj, attr, value)
        patcher.__enter__()
        self.patchers.append(patcher)

    @staticmethod
    def _steps():
        """Eight feeds over four chunks: outputs at 2 and 5, review last.

        One of the outputs carries a memo, so the run covers the two-screen
        case as well as the one-screen one.
        """
        plain = (
            CONSUMED,
            sign_pczt._STEP_OUTPUT,
            (0, _RECEIVER, 30_000, 0, b""),
        )
        with_memo = (
            CONSUMED,
            sign_pczt._STEP_OUTPUT,
            (1, _RECEIVER, 40_000, sign_pczt._MEMO_TEXT, b"hi"),
        )
        silent = (CONSUMED, 0, None)
        review = (CONSUMED, sign_pczt._STEP_REVIEW, _TOTALS)
        return [silent, plain, silent, silent, with_memo, silent, silent, review]

    def _progress(
        self, description=None, title=None, indeterminate=False, danger=False
    ):
        layout = _RecordingProgress(self.events, description)
        self.layouts.append(layout)
        return layout

    async def _call(self, msg, expected_type):
        return ZcashPcztAck(
            transfer_id=msg.transfer_id,
            offset=msg.offset,
            data=bytes(msg.length),
        )

    def _wipe(self):
        self.events.append((WIPE, None))

    async def _park(self, name):
        self.events.append((PARK_IN, name))
        self.events.append((PARK_OUT, name))

    async def _confirm_output(self, *args, **kwargs):
        await self._park("output")

    async def _confirm_memo(self, *args, **kwargs):
        await self._park("memo")

    async def _confirm_totals(self, *args, **kwargs):
        await self._park("totals")

    def _run(self):
        self.handle_out = []
        return await_result(
            sign_pczt._stream_and_sign(
                self.wallet_seed,
                0,  # ZcashNetwork.Mainnet
                0,
                10_000_000,
                PCZT_LENGTH,
                object(),  # session identity; require_session is patched out
                "Zcash",
                "Mainnet",
                "ZEC #1",
                "m/32'/133'/0'",
                self.handle_out,
            )
        )

    def test_the_handle_reaches_the_caller_that_has_to_cancel(self):
        """`sign_pczt`'s `finally` cancels by name, so it needs the handle.

        Without this the teardown is blind: it ends whatever session is live,
        which after an unwind may belong to a later workflow. The handle is
        published as soon as `session_begin` returns, before anything that can
        fail.
        """
        self._run()

        self.assertEqual(self.handle_out, [self.native.handle])
        sign_pczt._cancel_native(self.handle_out[0])
        self.assertEqual(self.native.cancelled, [self.native.handle])

    def test_teardown_before_begin_cancels_blind(self):
        """A failure before `session_begin` leaves no handle to name."""
        sign_pczt._cancel_native(None)
        self.assertEqual(self.native.cancelled, [None])

    def test_a_report_precedes_every_feed(self):
        """Report first, verify second -- the screen is lit for the work.

        Reporting after the feed would leave the ~3 s verification that
        follows each confirmation running on a screen whose backlight
        `Layout.stop()` just faded out.
        """
        self._run()

        feeds = [i for i, event in enumerate(self.events) if event[0] == FEED]
        self.assertEqual(len(feeds), len(self._steps()))
        for i in feeds:
            self.assertTrue(i > 0)
            self.assertEqual(self.events[i - 1][0], REPORT)

    def test_at_least_one_report_per_host_chunk(self):
        """The property that replaced the per-chunk `idle_timer.touch()`."""
        self._run()

        reports = [event for event in self.events if event[0] == REPORT]
        # One per feed, plus the report(0) that raises the layout before
        # `session_begin`, plus the signing layout's 0 and 1000.
        self.assertTrue(len(reports) >= CHUNKS)
        self.assertEqual(len(reports), len(self._steps()) + 3)

    def test_no_report_while_parked_on_a_button_request(self):
        """An abandoned sign must still reach autolock."""
        self._run()

        parked = False
        for kind, _payload in self.events:
            if kind == PARK_IN:
                parked = True
            elif kind == PARK_OUT:
                parked = False
            elif kind == REPORT:
                self.assertFalse(parked)

    def test_reported_values_are_monotonic_and_in_range(self):
        """The streaming bar tracks bytes verified and never goes backwards."""
        self._run()

        # The streaming layout is the first one built; the signing layout is
        # the second and restarts at zero, which is why they are read apart.
        streaming, signing = self.layouts
        self.assertEqual(streaming.description, TR.progress__loading_transaction)
        self.assertEqual(signing.description, TR.progress__signing_transaction)

        values = [
            value
            for index, (kind, value) in enumerate(self.events)
            if kind == REPORT and index < self._signing_starts_at()
        ]
        self.assertEqual(values, sorted(values))
        for value in values:
            self.assertTrue(0 <= value <= 1000)
        # Bytes verified, not actions: the first feed is reported at zero and
        # the last at (PCZT_LENGTH - CONSUMED) / PCZT_LENGTH.
        self.assertEqual(values[0], 0)
        self.assertEqual(values[-1], 1000 * (PCZT_LENGTH - CONSUMED) // PCZT_LENGTH)

    def test_every_seed_touching_native_call_is_followed_by_a_stack_wipe(self):
        """`session_begin` derives the account key; so does `session_sign`.

        `session_begin` runs the whole ZIP-32 path and `FullViewingKey::from`,
        which computes the spend-authorizing scalar as a temporary, and
        `zip32`'s `HardenedOnlyKey` is not `Zeroize`. Both leave key-derived
        bytes below the stack pointer, so both must be wrapped the way
        `get_address` and `get_viewing_key` wrap theirs. Before this was
        pinned, the first wipe of the run was after the first `session_feed`:
        a host round trip (up to CHUNK_TIMEOUT_MS) and one action's
        verification later.
        """
        self._run()

        kinds = [kind for kind, _payload in self.events]
        for call in (BEGIN, SIGN):
            index = kinds.index(call)
            self.assertEqual(kinds[index + 1], WIPE)
        # And nothing at all happens between begin and its wipe.
        self.assertEqual(kinds.index(BEGIN) + 1, kinds.index(WIPE))
        # The wipe after begin precedes the first host chunk and the first feed.
        self.assertTrue(kinds.index(WIPE) < kinds.index(FEED))

    def _signing_starts_at(self):
        # Everything after the totals confirmation belongs to the signing
        # layout, which starts its own count at zero.
        for index, (kind, payload) in enumerate(self.events):
            if kind == PARK_OUT and payload == "totals":
                return index
        raise AssertionError("the totals screen never ran")

    def test_the_streaming_layout_is_up_before_any_native_work(self):
        """`session_begin` prewarms Pasta/Sinsemilla -- seconds, blocking.

        So the very first thing that happens is a report, which raises the
        layout; the loop's own first report follows (same value, before the
        first feed), and only then does any native call run.
        """
        self._run()

        self.assertEqual(self.events[0], (REPORT, 0))
        self.assertEqual(self.events[1][0], BEGIN)
        kinds = [kind for kind, _payload in self.events]
        # The loop's own first report follows (same value), then the first feed.
        self.assertEqual(self.events[kinds.index(FEED) - 1], (REPORT, 0))


if __name__ == "__main__":
    unittest.main()
