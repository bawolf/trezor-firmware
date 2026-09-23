import unittest

from trezor import utils


@unittest.skipUnless(utils.USE_IRONWOOD, "requires native Ironwood support")
class TestIronwoodReceiver(unittest.TestCase):
    def setUp(self):
        from trezorironwood import derive_receiver, derive_viewing_key

        self.derive_receiver = derive_receiver
        self.derive_viewing_key = derive_viewing_key
        self.seed = bytes(range(32))

    def test_public_wallet_receiver(self):
        expected = bytes.fromhex(
            "fa727e62284586952d102565a00fd15e85a59a11ef6477d78dd7546c2a2e2fec3d618ba71c7c21c818c000"
        )
        receiver = self.derive_receiver(self.seed, 1, 9, bytes(11))
        self.assertIsInstance(receiver, bytes)
        self.assertEqual(len(receiver), 43)
        self.assertEqual(receiver, expected)

    def test_policy_and_length_rejections(self):
        for network in (2, 0xFFFFFFFF):
            with self.assertRaises(ValueError):
                self.derive_receiver(self.seed, network, 0, bytes(11))
        with self.assertRaises(ValueError):
            self.derive_receiver(self.seed, 1, 1 << 31, bytes(11))
        for length in (10, 12):
            with self.assertRaises(ValueError):
                self.derive_receiver(self.seed, 1, 0, bytes(length))
        for length in (15, 17, 31, 253):
            with self.assertRaises(RuntimeError):
                self.derive_receiver(bytes(length), 1, 0, bytes(11))

    def test_restored_128_bit_slip39_seed(self):
        expected = bytes.fromhex(
            "8b446ad4a86e639364748511325865bef5778af22f520fd0f95eec80f6ea3ef2012fd4eaabbb32bb14aea1"
        )
        self.assertEqual(
            self.derive_receiver(bytes([0xA5]) * 16, 1, 0, bytes(11)), expected
        )

    def test_argument_types(self):
        valid = (self.seed, 1, 9, bytes(11))
        for position in range(4):
            args = list(valid)
            args[position] = None
            with self.assertRaises(TypeError):
                self.derive_receiver(*args)

    def test_full_viewing_key_fills_caller_buffer(self):
        expected = bytes.fromhex(
            "20f8c2edbe19901c0d1b5cc7ab185e67354511bfc5174fe6bc0e6362c5880b28"
            "fabbf237258f8d03b200ad7fe0f3fa7e80e628f2b745dc9983b038c3a81f8237"
            "b6654db322e68436a972c6d3bc56e5560fb8658055524a11d6ee62e5a7d7a516"
        )
        output = bytearray(96)
        result = self.derive_viewing_key(self.seed, 0, 9, output)
        self.assertIsNone(result)
        self.assertEqual(output, expected)

    def test_full_viewing_key_policy_and_output_errors(self):
        for network in (2, 0xFFFFFFFF):
            with self.assertRaises(ValueError):
                self.derive_viewing_key(self.seed, network, 0, bytearray(96))
        with self.assertRaises(ValueError):
            self.derive_viewing_key(self.seed, 0, 1 << 31, bytearray(96))
        for length in (95, 97):
            with self.assertRaises(ValueError):
                self.derive_viewing_key(self.seed, 0, 0, bytearray(length))
        for length in (15, 17, 31, 253):
            with self.assertRaises(RuntimeError):
                self.derive_viewing_key(bytes(length), 0, 0, bytearray(96))

        # The seed must be immutable so it cannot alias the writable output.
        with self.assertRaises(TypeError):
            self.derive_viewing_key(bytearray(32), 0, 0, bytearray(96))
        with self.assertRaises(TypeError):
            self.derive_viewing_key(self.seed, 0, 0, bytes(96))

    def test_full_viewing_key_accepts_restored_128_bit_seed(self):
        output = bytearray(96)
        self.derive_viewing_key(bytes([0xA5]) * 16, 1, 0, output)
        self.assertNotEqual(output, bytearray(96))

    def test_module_exposes_no_spending_key_boundary(self):
        import trezorironwood

        self.assertFalse(hasattr(trezorironwood, "derive_spending_key"))
        self.assertFalse(hasattr(trezorironwood, "derive_spend_authorizing_key"))


@unittest.skipUnless(utils.USE_IRONWOOD, "requires native Ironwood support")
class TestIronwoodSessionHandle(unittest.TestCase):
    """The handle binds a native signing request to the workflow that began it.

    `feed`, `approve` and `sign` refuse any other handle, and refuse it
    *without touching the live request*: destroying it there would turn a stray
    call from an already-dead workflow into a way to cancel the running one.
    The same argument applies to `cancel` once a handle is given.
    """

    SEED = bytes(range(32))
    NETWORK = 1  # Testnet
    ACCOUNT = 0
    HEIGHT = 10_000_000
    MAXIMUM_FEE = 100_000
    EXPIRY_WINDOW = 40
    PCZT_LENGTH = 4096

    def setUp(self):
        import trezorironwood

        self.native = trezorironwood
        # One buffer per test: the allocator holds a raw pointer into it for
        # the length of the session and gives it back at `session_cancel`.
        self.scratch = bytearray(trezorironwood.SCRATCH_BYTES)
        self.handle = self._begin()

    def tearDown(self):
        self.native.session_cancel()

    def _begin(self, scratch=None):
        return self.native.session_begin(
            self.SEED,
            self.NETWORK,
            self.ACCOUNT,
            self.HEIGHT,
            self.MAXIMUM_FEE,
            self.EXPIRY_WINDOW,
            self.PCZT_LENGTH,
            self.scratch if scratch is None else scratch,
        )

    def _assert_alive(self, handle):
        """Feed a byte the live session accepts as "not enough yet"."""
        consumed, kind, payload = self.native.session_feed(handle, b"\x00")
        self.assertEqual(kind, 0)
        self.assertIsNone(payload)
        self.assertEqual(consumed, 1)

    def test_handle_is_never_zero(self):
        self.assertNotEqual(self.handle, 0)
        with self.assertRaises(ValueError):
            self.native.session_feed(0, b"\x00")

    def test_a_foreign_handle_is_refused_and_leaves_the_request_alone(self):
        foreign = self.handle ^ 0xFFFF
        for call in (
            lambda: self.native.session_feed(foreign, b"\x00"),
            lambda: self.native.session_approve(foreign),
            lambda: self.native.session_sign(foreign, self.SEED),
        ):
            with self.assertRaises(RuntimeError) as raised:
                call()
            self.assertEqual(str(raised.value), "Invalid signing state")
            # The live request survived every one of them.
            self._assert_alive(self.handle)

    def test_cancel_with_a_foreign_handle_does_not_tear_down_the_request(self):
        self.native.session_cancel(self.handle ^ 0xFFFF)
        self._assert_alive(self.handle)

    def test_cancel_with_the_owning_handle_ends_the_request(self):
        self.native.session_cancel(self.handle)
        with self.assertRaises(RuntimeError):
            self.native.session_feed(self.handle, b"\x00")

    def test_cancel_without_a_handle_ends_whatever_is_live(self):
        self.native.session_cancel()
        with self.assertRaises(RuntimeError):
            self.native.session_feed(self.handle, b"\x00")

    def test_a_scratch_below_the_native_minimum_is_refused(self):
        self.native.session_cancel()
        with self.assertRaises(ValueError):
            self._begin(bytearray(self.native.SCRATCH_BYTES - 1))

    def test_the_workflow_asks_for_exactly_the_native_minimum(self):
        from apps.zcash.sign_pczt import SCRATCH_BYTES

        self.assertEqual(SCRATCH_BYTES, self.native.SCRATCH_BYTES)

    def test_a_second_session_takes_a_second_scratch(self):
        """The scratch is per-session; only the rooted tier spans a boot.

        Cancelling gives the buffer back, so the next session may bring a
        different one -- which is what the workflow does, one `bytearray` per
        sign. A native tier that outlived its buffer would fail here.
        """
        self.native.session_cancel(self.handle)
        self.scratch = bytearray(self.native.SCRATCH_BYTES)
        handle = self._begin()
        self._assert_alive(handle)

    def test_region_info_is_a_debug_instrument_or_nothing(self):
        info = self.native.debug_region_info()
        if info is None:
            return
        persist_in_use, persist_peak, scratch_in_use, scratch_peak = info
        self.assertTrue(persist_in_use <= persist_peak)
        self.assertTrue(scratch_in_use <= scratch_peak)
        # Idempotent: teardown runs from a `finally` that may run twice.
        self.native.session_cancel()
        self.native.session_cancel(self.handle)

    def test_a_second_begin_after_a_cancel_retires_the_first_handle(self):
        """A new session invalidates the old handle, so a stray call from an
        already-dead workflow cannot reach the live one.

        The cancel first is the contract, not a convenience: on a device
        `session_begin` refuses to start while a scratch is still installed,
        because a scratch nobody released means the workflow that installed it
        never ran its `finally` and the `bytearray` behind it may already have
        been collected. That check is native and fatal, so it cannot be
        asserted here -- the emulator has no arenas to install (see
        `rust/src/ironwood/allocator_unix.rs`); the mechanism is unit-tested in
        `ironwood::arena`.
        """
        self.native.session_cancel(self.handle)
        second = self._begin()
        self.assertNotEqual(second, self.handle)
        with self.assertRaises(RuntimeError):
            self.native.session_feed(self.handle, b"\x00")
        self._assert_alive(second)


if __name__ == "__main__":
    unittest.main()
