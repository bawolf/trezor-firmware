# flake8: noqa: F403,F405
from common import *  # isort:skip

import ustruct
from mock import patch
from storage import cache_common
from trezor import io, loop
from trezor.messages import ExtAppMessage
from trezor.wire import context

if not utils.USE_THP:
    from storage import cache_codec

if utils.USE_APP_LOADING:
    from apps.extapp import run as extapp_run

_WIRE_END = 2  # run.py's _SERVICE_WIRE_END, a const() and so not importable


class _Message:
    def __init__(self, service: int, data: bytes) -> None:
        self.fn = service << 16
        self.data = data


class _Slot:
    """Stands in for `app`, `io` and `loop` in run.py: one running app, which
    answers every request with b"reply". `inbox` holds its messages to Core."""

    Timeout = loop.Timeout

    def __init__(self, inbox: list[_Message]) -> None:
        self.inbox = inbox
        # Read here, not in the class body: only builds with IPC have it.
        self.IPC2_EVENT = io.IPC2_EVENT
        self.POLL_READ = io.POLL_READ

    def image_by_handle(self, handle: int) -> "_Slot":
        return self

    def allowed_curves(self) -> list[str]:
        return ["secp256k1"]

    def allowed_paths(self) -> list[str]:
        return ["m/44'/60'/account'"]

    def is_running(self) -> bool:
        return True

    def task_id(self) -> int:
        return 2

    def ipc_send(self, task_id: int, fn: int, data: bytes) -> None:
        self.inbox.append(_Message(_WIRE_END, b"reply"))

    async def wait(self, iface: int, timeout_ms: int) -> _Message:
        if not self.inbox:
            raise loop.Timeout
        return self.inbox.pop(0)


@unittest.skipUnless(utils.USE_APP_LOADING, "app loading")
class TestExtappRun(TestCaseWithContext):
    def setUp(self):
        if not utils.USE_THP:
            cache_codec.start_session()

    def tearDown(self):
        context.cache_delete(cache_common.APP_EXTAPP_IDS)

    def test_drops_message_sent_before_the_request(self):
        # e.g. left in the slot by an earlier instance
        slot = _Slot([_Message(_WIRE_END, b"stale")])
        instance_id = 1
        context.cache_set(
            cache_common.APP_EXTAPP_IDS, ustruct.pack("<II", 0, instance_id)
        )
        request = ExtAppMessage(instance_id=instance_id, message_id=0, data=b"")
        with patch(extapp_run, "app", slot), patch(extapp_run, "io", slot):
            with patch(extapp_run, "loop", slot):
                response = await_result(extapp_run.run(request))
        self.assertEqual(response.data, b"reply")


if __name__ == "__main__":
    unittest.main()
