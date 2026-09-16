# flake8: noqa: F403,F405
from common import *  # isort:skip

from mock import patch
from trezor import TR, utils, wire
from trezor.enums import ButtonRequestType, MessageType, ZcashNetwork
from trezor.messages import ZcashGetAddress
from trezor.ui import layouts
from trezor.wire import context

from apps import workflow_handlers
from apps.common import seed
from apps.zcash import get_address, ironwood_account


class _Cache:
    def __init__(self, state) -> None:
        self.state = state

    def export_session_id(self) -> bytes:
        return self.state[0]


class _Context:
    def __init__(self, state) -> None:
        self.cache = _Cache(state)


@unittest.skipUnless(not utils.BITCOIN_ONLY, "altcoin")
class TestIronwoodGetAddress(unittest.TestCase):
    def setUp(self):
        self.patchers = []
        self.calls = []
        self.session = [b"session-a"]
        self.wallet_seed = bytes(range(32))
        self.receiver = bytes(range(43))

        self._patch(utils, "USE_IRONWOOD", True)
        self._patch(utils, "USE_THP", False)
        self._patch(context, "get_context", lambda: _Context(self.session))
        self._patch(seed, "raise_if_not_initialized", lambda: None)
        self._patch(seed, "get_seed", self._get_seed)
        self._patch(ironwood_account, "has_weak_backup", lambda: False)
        self._patch(get_address, "_call_native", self._native)
        self._patch(utils, "zero_unused_stack", self._clear_stack)
        self._patch(layouts, "show_warning", self._show_warning)
        self._patch(layouts, "show_address", self._show_address)

    def tearDown(self):
        for patcher in reversed(self.patchers):
            patcher.__exit__(None, None, None)

    def _patch(self, obj, attr, value):
        patcher = patch(obj, attr, value)
        patcher.__enter__()
        self.patchers.append(patcher)

    async def _get_seed(self):
        self.calls.append(("seed", self.wallet_seed))
        return self.wallet_seed

    def _native(self, wallet_seed, network, account, diversifier_index):
        self.calls.append(("native", wallet_seed, network, account, diversifier_index))
        return self.receiver

    def _clear_stack(self):
        self.calls.append(("stack_clear",))

    async def _show_warning(self, *args, **kwargs):
        self.calls.append(("warning", args, kwargs))

    async def _show_address(self, *args, **kwargs):
        self.calls.append(("address", args, kwargs))

    @staticmethod
    def _message(network=ZcashNetwork.Mainnet, account=0, index=bytes(11)):
        return ZcashGetAddress(
            network=network,
            account=account,
            diversifier_index=index,
        )

    def test_mainnet_and_testnet_addresses_and_exact_ui(self):
        vectors = (
            (
                ZcashNetwork.Mainnet,
                0,
                "u19w0wmqnyrcx4kx9yxj3l9ewxzpqtrxqq62xswtkhu5r0temhnlhfpde2fud9xa6esrjdqknqsx0qkm8w3qfmue586pc0x3arjvhxc35u",
                "Mainnet",
                "ZEC #1",
                "m/32'/133'/0'",
            ),
            (
                ZcashNetwork.Testnet,
                7,
                "utest1jeykdhzrwz45pnx9pqd2uefaxdedyrmyq4l2r5l9uxq583gj8k397f6s0wvf724py7fjuhy94glytwljsmezgyeyruk2wcktec3ykh4y",
                "Testnet",
                "ZEC #8",
                "m/32'/1'/7'",
            ),
        )
        for network, account, expected, label, account_label, path in vectors:
            self.calls.clear()
            response = await_result(
                get_address.get_address(self._message(network, account))
            )
            self.assertEqual(response.address, expected)

            address_calls = [call for call in self.calls if call[0] == "address"]
            self.assertEqual(len(address_calls), 1)
            kwargs = address_calls[0][2]
            self.assertEqual(kwargs["address"], expected)
            self.assertEqual(kwargs["address_qr"], expected)
            self.assertFalse("title" in kwargs)
            self.assertEqual(kwargs["network"], label)
            self.assertEqual(kwargs["account"], account_label)
            self.assertEqual(kwargs["path"], path)
            self.assertEqual(kwargs["br_code"], ButtonRequestType.Address)
            self.assertFalse(kwargs["case_sensitive"])
            self.assertEqual(
                [call[0] for call in self.calls],
                ["seed", "native", "stack_clear", "address"],
            )

    @unittest.skipUnless(utils.USE_IRONWOOD, "requires native Ironwood support")
    def test_real_native_receiver_is_encoded_by_the_handler(self):
        from trezorironwood import derive_receiver

        self._patch(get_address, "_call_native", derive_receiver)
        response = await_result(
            get_address.get_address(self._message(ZcashNetwork.Testnet, 9))
        )
        expected = "utest1pxwu0ctu3sgmzje5pswsk5mcuxv9l25sr8um5f30ftakls0dgx8quzms8xe4n9k9ue6dj3qh0esy7a8dxfkways7w76xzryjsqxjtvgu"
        self.assertEqual(response.address, expected)
        self.assertEqual(self.calls[-1][2]["address"], expected)
        self.assertEqual(self.calls[-1][2]["address_qr"], expected)
        self.assertEqual(
            [call[0] for call in self.calls],
            ["seed", "stack_clear", "address"],
        )

    def test_weak_backup_warning_precedes_raw_16_byte_seed_derivation(self):
        self.wallet_seed = bytes(range(16))
        self._patch(ironwood_account, "has_weak_backup", lambda: True)

        response = await_result(get_address.get_address(self._message()))
        self.assertTrue(response.address.startswith("u1"))
        self.assertEqual(
            [call[0] for call in self.calls],
            ["warning", "seed", "native", "stack_clear", "address"],
        )
        warning = self.calls[0][2]
        self.assertEqual(warning["br_name"], "ironwood_weak_backup")
        self.assertEqual(warning["content"], TR.zcash__weak_backup_warning)
        self.assertEqual(warning["br_code"], ButtonRequestType.Warning)
        native = self.calls[2]
        self.assertIs(native[1], self.wallet_seed)
        self.assertEqual(len(native[1]), 16)

    def test_rejection_returns_no_address_and_does_not_derive(self):
        async def reject(*args, **kwargs):
            self.calls.append(("warning", args, kwargs))
            raise wire.ActionCancelled()

        self._patch(ironwood_account, "has_weak_backup", lambda: True)
        self._patch(layouts, "show_warning", reject)
        with self.assertRaises(wire.ActionCancelled):
            await_result(get_address.get_address(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["warning"])

    def test_address_cancellation_still_clears_native_stack(self):
        async def reject(*args, **kwargs):
            self.calls.append(("address", args, kwargs))
            raise wire.ActionCancelled()

        self._patch(layouts, "show_address", reject)
        with self.assertRaises(wire.ActionCancelled):
            await_result(get_address.get_address(self._message()))
        self.assertEqual(
            [call[0] for call in self.calls],
            ["seed", "native", "stack_clear", "address"],
        )

    def test_session_change_after_each_await(self):
        async def change_during_warning(*args, **kwargs):
            self.calls.append(("warning", args, kwargs))
            self.session[0] = b"session-b"

        self._patch(ironwood_account, "has_weak_backup", lambda: True)
        self._patch(layouts, "show_warning", change_during_warning)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_address.get_address(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["warning"])

        self.calls.clear()
        self.session[0] = b"session-a"
        self._patch(ironwood_account, "has_weak_backup", lambda: False)

        async def change_during_seed():
            self.calls.append(("seed", self.wallet_seed))
            self.session[0] = b"session-b"
            return self.wallet_seed

        self._patch(seed, "get_seed", change_during_seed)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_address.get_address(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["seed"])

    def test_session_change_after_confirmation_returns_no_response(self):
        async def change_during_address(*args, **kwargs):
            self.calls.append(("address", args, kwargs))
            self.session[0] = b"session-b"

        self._patch(layouts, "show_address", change_during_address)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_address.get_address(self._message()))
        self.assertEqual(
            [call[0] for call in self.calls],
            ["seed", "native", "stack_clear", "address"],
        )

    def test_invalid_inputs_fail_before_wallet_or_native_access(self):
        malformed = (
            self._message(network=True),
            self._message(account=False),
            self._message(index=bytes(10)),
        )
        for msg in malformed:
            with self.assertRaises(wire.DataError) as raised:
                await_result(get_address.get_address(msg))
            self.assertEqual(raised.value.message, "Malformed Zcash request")

        disallowed = (
            self._message(network=2),
            self._message(account=-1),
            self._message(account=1 << 31),
        )
        for msg in disallowed:
            with self.assertRaises(wire.ProcessError) as raised:
                await_result(get_address.get_address(msg))
            self.assertEqual(
                raised.value.message,
                "Zcash request violates device policy",
            )
        self.assertEqual(self.calls, [])

    def test_native_errors_are_stable_and_clear_stack(self):
        for error, message in (
            (ValueError(), "Zcash receiver derivation failed"),
            (RuntimeError(), "Zcash receiver derivation failed"),
        ):
            self.calls.clear()

            def fail(*args):
                self.calls.append(("native",))
                raise error

            self._patch(get_address, "_call_native", fail)
            with self.assertRaises(wire.ProcessError) as raised:
                await_result(get_address.get_address(self._message()))
            self.assertEqual(raised.value.message, message)
            self.assertEqual(
                [call[0] for call in self.calls],
                ["seed", "native", "stack_clear"],
            )

    def test_invalid_native_output_is_not_displayed(self):
        self.receiver = bytes(42)
        with self.assertRaises(wire.ProcessError) as raised:
            await_result(get_address.get_address(self._message()))
        self.assertEqual(raised.value.message, "Zcash receiver derivation failed")
        self.assertEqual(
            [call[0] for call in self.calls],
            ["seed", "native", "stack_clear"],
        )

    def test_handler_registration_is_feature_gated(self):
        with patch(utils, "BITCOIN_ONLY", False):
            with patch(utils, "USE_IRONWOOD", True):
                self.assertEqual(
                    workflow_handlers._find_message_handler_module(
                        MessageType.ZcashGetAddress
                    ),
                    "apps.zcash.get_address",
                )
            with patch(utils, "USE_IRONWOOD", False):
                with self.assertRaises(ValueError):
                    workflow_handlers._find_message_handler_module(
                        MessageType.ZcashGetAddress
                    )


if __name__ == "__main__":
    unittest.main()
