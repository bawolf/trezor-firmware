# fix(core): raise extapp bridge errors instead of unwrapping

Status: note for the owners of `bieleluk/sdk-wip` (#7516), not a PR. The
bridges exist only on the draft SDK branches.
Stacked on `extapp/bridge-validate-untrusted-input` (same lines).
Reproduction status: a Core unit test ends Core before and passes after.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/bridge-raise-not-rsod @ `5ae2052a08`
(one commit on `extapp/bridge-validate-untrusted-input` @ `fc12e1b57b`)

---

The UI and Crypto bridges ended Core with "unwrap failed" when the IPC reply
callback failed (the app is gone, or its inbox is full), when an allocation
sized by the app's request failed, and when a menu had more than
`MAX_MENU_ITEMS` items. So a misbehaving app could restart the device instead
of being stopped.

**Reproduce.** `test_reply_callback_failure_is_raised` (new in
`core/tests/test_apps.extapp.ipc.py`) calls `send_crypto_result` and
`send_ui_result` with a callback that raises. Non-frozen T3W1 `--apps`
emulator, with `extapp/bridge-validate-untrusted-input` only:

```
test_reply_callback_failure_is_raised ...Task #1 terminated.
Fatal: unwrap failed at rust/src/crypto/api/firmware_micropython.rs:161
```

**Cause.** `unwrap!` on paths an app's request or state can reach.

**Fix.**
- Those `unwrap!`s become errors; the callback's exception propagates.
- `run.py` stops the app with `DataError` on any failure to process its UI or
  Progress request or to send the UI result, as it already did for the crypto
  result. Before, only `ValueError` was caught, so a `MemoryError` ended `run()`
  and left the app waiting.
- A progress value above the bar's 1000 is clamped instead of overflowing.
- `ipc_cb` becomes a required argument of `send_ui_result`, as it already was
  in practice (`.pyi` updated).
- Paths where Core builds the value (the results `run.py` makes) keep their
  `unwrap!`.

**Tests.** `test_reply_callback_failure_is_raised`: fatal before, passes after
(4/4 in the file).

### Notes for QA
A misbehaving app is stopped with a `DataError`; Core stays up.
