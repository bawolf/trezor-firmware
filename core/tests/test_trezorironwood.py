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


if __name__ == "__main__":
    unittest.main()
