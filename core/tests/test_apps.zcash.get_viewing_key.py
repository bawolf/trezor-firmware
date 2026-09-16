# flake8: noqa: F403,F405
from common import *  # isort:skip

from mock import patch
from trezor import TR, utils, wire
from trezor.enums import ButtonRequestType, MessageType, ZcashNetwork
from trezor.ui import layouts
from trezor.wire import context

from apps.common import seed

if not utils.BITCOIN_ONLY:
    from trezor.messages import ZcashGetViewingKey

    from apps import workflow_handlers
    from apps.zcash import get_viewing_key, ironwood_account, unified_addresses


class _Cache:
    def __init__(self, state) -> None:
        self.state = state

    def export_session_id(self) -> bytes:
        return self.state[0]


class _Context:
    def __init__(self, state) -> None:
        self.cache = _Cache(state)


@unittest.skipUnless(not utils.BITCOIN_ONLY, "altcoin")
class TestIronwoodGetViewingKey(unittest.TestCase):
    def setUp(self):
        self.patchers = []
        self.calls = []
        self.session = [b"session-a"]
        self.wallet_seed = bytes(range(32))
        self.raw_fvk = bytes(range(96))
        self.output = None
        self.real_call_native = get_viewing_key._call_native

        self._patch(utils, "USE_IRONWOOD", True)
        self._patch(utils, "USE_THP", False)
        self._patch(context, "get_context", lambda: _Context(self.session))
        self._patch(seed, "raise_if_not_initialized", lambda: None)
        self._patch(seed, "get_seed", self._get_seed)
        self._patch(ironwood_account, "has_weak_backup", lambda: False)
        self._patch(get_viewing_key, "_call_native", self._native)
        self._patch(utils, "zero_unused_stack", self._clear_stack)
        self._patch(layouts, "confirm_action", self._confirm)
        self._patch(layouts, "show_warning", self._show_warning)

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

    def _native(self, wallet_seed, network, account, output):
        self.calls.append(("native", wallet_seed, network, account, output))
        self.output = output
        output[:] = self.raw_fvk

    def _clear_stack(self):
        self.calls.append(("stack_clear",))

    async def _confirm(self, *args, **kwargs):
        self.calls.append(("confirm", args, kwargs))

    async def _show_warning(self, *args, **kwargs):
        self.calls.append(("warning", args, kwargs))

    @staticmethod
    def _message(network=ZcashNetwork.Mainnet, account=0):
        return ZcashGetViewingKey(network=network, account=account)

    def test_network_account_consent_and_exact_key(self):
        vectors = (
            (
                ZcashNetwork.Mainnet,
                0,
                "uview18kwgqjck7spf75xa2svp720mflgwnw786vyxn9jfnf4l5q0ct6hv80vpx9r33m8zmhxmnr03q9v6yk6j3474w2lekdnghn25eafdhatyr3zr2t380e9rkyxtv3fg2jdg7jfkgwglyhnjkm7gw8z9jmke0mky5ja4h2arpkvkmsc4h55pgdjzdts4q7wz7",
                "Zcash Mainnet\nZEC #1\nm/32'/133'/0'",
            ),
            (
                ZcashNetwork.Testnet,
                7,
                "uviewtest1ewkpkaqar0lepvymgvldj84se9dy32wtgywydq4qtgd4n5atjp4thf54s4w8hd2vq4ja7rq730anvau40f38726lsk04vh0q2peznfkxehf0yz4mrkfnft0k3g37ms2fxka6cf7dxscmrh64nls5hxjp8glj40kqvlpmahrr2y0xzhkku3pvflghadxqf",
                "Zcash Testnet\nZEC #8\nm/32'/1'/7'",
            ),
        )
        for network, account, expected, details in vectors:
            self.calls.clear()
            response = await_result(
                get_viewing_key.get_viewing_key(self._message(network, account))
            )
            self.assertEqual(response.key, expected)
            self.assertEqual(
                [call[0] for call in self.calls],
                ["confirm", "seed", "native", "stack_clear"],
            )
            confirmation = self.calls[0]
            self.assertEqual(confirmation[1][0], "ironwood_export_viewing_key")
            self.assertEqual(confirmation[1][1], TR.zcash__export_viewing_key)
            self.assertEqual(confirmation[2]["action"], details)
            self.assertEqual(
                confirmation[2]["description"], TR.zcash__viewing_key_warning
            )
            self.assertEqual(confirmation[2]["br_code"], ButtonRequestType.SignTx)
            self.assertTrue(confirmation[2]["prompt_screen"])
            self.assertEqual(self.output, bytearray(96))

    def test_consent_rejection_accesses_no_seed_or_native(self):
        async def reject(*args, **kwargs):
            self.calls.append(("confirm", args, kwargs))
            raise wire.ActionCancelled()

        self._patch(layouts, "confirm_action", reject)
        with self.assertRaises(wire.ActionCancelled):
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["confirm"])

    def test_weak_backup_warning_follows_privacy_consent(self):
        self.wallet_seed = bytes(range(16))
        self._patch(ironwood_account, "has_weak_backup", lambda: True)
        await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual(
            [call[0] for call in self.calls],
            ["confirm", "warning", "seed", "native", "stack_clear"],
        )
        warning = self.calls[1][2]
        self.assertEqual(warning["content"], TR.zcash__weak_backup_warning)
        self.assertEqual(warning["br_code"], ButtonRequestType.Warning)

    def test_weak_backup_warning_cancellation_derives_no_key(self):
        self._patch(ironwood_account, "has_weak_backup", lambda: True)

        async def reject(*args, **kwargs):
            self.calls.append(("warning", args, kwargs))
            raise wire.ActionCancelled()

        self._patch(layouts, "show_warning", reject)
        with self.assertRaises(wire.ActionCancelled):
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["confirm", "warning"])

    def test_session_change_after_each_await(self):
        async def change_during_confirmation(*args, **kwargs):
            self.calls.append(("confirm", args, kwargs))
            self.session[0] = b"session-b"

        self._patch(layouts, "confirm_action", change_during_confirmation)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["confirm"])

        self.calls.clear()
        self.session[0] = b"session-a"
        self._patch(layouts, "confirm_action", self._confirm)
        self._patch(ironwood_account, "has_weak_backup", lambda: True)

        async def change_during_warning(*args, **kwargs):
            self.calls.append(("warning", args, kwargs))
            self.session[0] = b"session-b"

        self._patch(layouts, "show_warning", change_during_warning)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["confirm", "warning"])

        self.calls.clear()
        self.session[0] = b"session-a"
        self._patch(ironwood_account, "has_weak_backup", lambda: False)

        async def change_during_seed():
            self.calls.append(("seed", self.wallet_seed))
            self.session[0] = b"session-b"
            return self.wallet_seed

        self._patch(seed, "get_seed", change_during_seed)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual([call[0] for call in self.calls], ["confirm", "seed"])

    def test_final_session_check_returns_no_key(self):
        def derive_and_replace(*args):
            self._native(*args)
            self.session[0] = b"session-b"

        self._patch(get_viewing_key, "_call_native", derive_and_replace)
        with self.assertRaises(wire.InvalidSession):
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual(
            [call[0] for call in self.calls],
            ["confirm", "seed", "native", "stack_clear"],
        )
        self.assertEqual(self.output, bytearray(96))

    def test_invalid_inputs_fail_before_consent_or_wallet_access(self):
        malformed = (self._message(network=True), self._message(account=False))
        for msg in malformed:
            with self.assertRaises(wire.DataError) as raised:
                await_result(get_viewing_key.get_viewing_key(msg))
            self.assertEqual(raised.value.message, "Malformed Zcash request")

        disallowed = (
            self._message(network=2),
            self._message(account=-1),
            self._message(account=1 << 31),
        )
        for msg in disallowed:
            with self.assertRaises(wire.ProcessError) as raised:
                await_result(get_viewing_key.get_viewing_key(msg))
            self.assertEqual(
                raised.value.message, "Zcash request violates device policy"
            )
        self.assertEqual(self.calls, [])

    def test_native_errors_are_stable_clear_stack_and_output(self):
        for error in (ValueError(), RuntimeError()):
            self.calls.clear()

            def fail(*args):
                self.output = args[3]
                self.output[:] = bytes([0xA5]) * 96
                self.calls.append(("native",))
                raise error

            self._patch(get_viewing_key, "_call_native", fail)
            with self.assertRaises(wire.ProcessError) as raised:
                await_result(get_viewing_key.get_viewing_key(self._message()))
            self.assertEqual(
                raised.value.message, "Zcash viewing key derivation failed"
            )
            self.assertEqual(
                [call[0] for call in self.calls],
                ["confirm", "seed", "native", "stack_clear"],
            )
            self.assertEqual(self.output, bytearray(96))

    def test_encoder_error_is_stable_and_clears_output(self):
        def fail(*args):
            raise ValueError("internal encoder detail")

        self._patch(unified_addresses, "encode_fvk", fail)
        with self.assertRaises(wire.ProcessError) as raised:
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual(raised.value.message, "Zcash viewing key derivation failed")
        self.assertEqual(
            [call[0] for call in self.calls],
            ["confirm", "seed", "native", "stack_clear"],
        )
        self.assertEqual(self.output, bytearray(96))

    @unittest.skipUnless(utils.USE_IRONWOOD, "requires native Ironwood support")
    def test_real_native_key_is_encoded_by_handler(self):
        self._patch(get_viewing_key, "_call_native", self.real_call_native)
        vectors = (
            (
                ZcashNetwork.Mainnet,
                0,
                "uview1zdu9t6lz8leye6d4s5kwvum4e5xeaz0sn2trxtq7236e8c9xjk9keqezce387205y2nry9dvmuua7tl2ev7q8gamrduwucsc49yzn38sjw5xv4kpxzutjteuyvyuecwskt7j4t35hwqrx0zxvsflut39mexv3k06wz4e0vgdf3f90lkju9yj84qcz56lz",
            ),
            (
                ZcashNetwork.Mainnet,
                9,
                "uview17j0q0nnczz63ducvkhe409f4r8sa2gx88unakv64k95dpe4r2hvn3lhe2gdfn00vsl830682a7tdhzwuhtsw2dp7usgxzdgqxujgu4pv50xrhuakfuk294xjcuhrs5ag0esenlp4wsawqmuqaaspykcplgk0vrds7fm0hrp3up2mmzgh7rdfhycgu2xp8",
            ),
            (
                ZcashNetwork.Testnet,
                7,
                "uviewtest1am4uul0ak7snnnt35lerj6mlnpyrhtxrg45qe08u97vpsgrxth6mt330hwwj06cgdenk7ft6zrx0arwxa3240h6hvh5ym6nsgyxqc526a8uvv4g9xwqh6hzjtqma4npk5xvlvhmlsss72nljesc56dwkjvwzh07tngnm9230h5gp2xnvjfqxueqlqckmc",
            ),
        )
        for network, account, expected in vectors:
            self.calls.clear()
            response = await_result(
                get_viewing_key.get_viewing_key(self._message(network, account))
            )
            self.assertEqual(response.key, expected)
            self.assertEqual(
                [call[0] for call in self.calls],
                ["confirm", "seed", "stack_clear"],
            )

    def test_direct_feature_disabled_invocation_fails_closed(self):
        self._patch(utils, "USE_IRONWOOD", False)
        with self.assertRaises(wire.ProcessError) as raised:
            await_result(get_viewing_key.get_viewing_key(self._message()))
        self.assertEqual(raised.value.message, "Ironwood is not supported")
        self.assertEqual(self.calls, [])

    def test_handler_registration_is_feature_gated(self):
        with patch(utils, "BITCOIN_ONLY", False):
            with patch(utils, "USE_IRONWOOD", True):
                self.assertEqual(
                    workflow_handlers._find_message_handler_module(
                        MessageType.ZcashGetViewingKey
                    ),
                    "apps.zcash.get_viewing_key",
                )
            with patch(utils, "USE_IRONWOOD", False):
                with self.assertRaises(ValueError):
                    workflow_handlers._find_message_handler_module(
                        MessageType.ZcashGetViewingKey
                    )


if __name__ == "__main__":
    unittest.main()
