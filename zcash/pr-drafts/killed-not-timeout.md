# fix(core): report a stopped extapp instead of a timeout

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches. No committed test; a scratch fixture
reproduces it.
Rerun 2026-09-26 on sdk-wip and on the pushed tip `fc136e637c` (T3W1 non-frozen
`--apps` emulator built at each): receipts `pr-drafts-rerun/logs/killed-{base,tip}.log`.
Local macOS build fix only (`15de3664a6` cherry-picked for the emulator app); not
needed on Linux.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/killed-not-timeout @ `fc136e637c`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

An app that faults while handling a request sends nothing, so `run.py`
reported the crash as "Timeout waiting for message", the same as a slow app.
On timeout, an image that is no longer running now fails with the loop's
existing `DataError("Task stopped: …")`; a slow app still gets the timeout.

Tested on a T3W1 `--apps` emulator with a scratch fault in the Tron sample
([repro/killed-not-timeout](repro/killed-not-timeout/): `git apply
get_address.patch`, copy `test_fault_probe.py` to `sdk/apps/tron/tests/`),
`xtask modular build -p tron -m t3w1 --lang en -d -e`, then
`pytest --app=../target/artifacts/t3w1-emu/tron.elf --lang=en -s tests/test_fault_probe.py`
from `sdk/apps/tron`:

```
before: DataError: Timeout waiting for message
after:  DataError: Task stopped: 224053394
```

The emulator console shows the app's `Fatal: probe: intentional fault at
get_address.rs:14` in both runs. A permanent test could go with the applet
fixtures of `bieleluk/extapp-device-tests` once that branch has a dispatching
`run.py`.
