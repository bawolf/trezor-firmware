import unittest

from trezor import utils


@unittest.skipUnless(utils.USE_IRONWOOD, "requires native Ironwood support")
class TestIronwoodReceiver(unittest.TestCase):
    def setUp(self):
        from trezorironwood import derive_receiver

        self.derive_receiver = derive_receiver
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
        for length in (16, 31, 253):
            with self.assertRaises(RuntimeError):
                self.derive_receiver(bytes(length), 1, 0, bytes(11))

    def test_argument_types(self):
        valid = (self.seed, 1, 9, bytes(11))
        for position in range(4):
            args = list(valid)
            args[position] = None
            with self.assertRaises(TypeError):
                self.derive_receiver(*args)


if __name__ == "__main__":
    unittest.main()
