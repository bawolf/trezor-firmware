# Keep an extapp alive when the host abandons its request

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches. A bounded, partial fix: a complete one
needs a protocol change (below), which is the owner's call.
Stacked on `extapp/sdk-progress-api` (the SDK half needs its screen tracking);
the Core half applies to sdk-wip alone.
Reproduction status: observed only with a device test of an out-of-tree app
that streams data under a progress screen; no platform-level test, and no run
on this branch's tip, which cannot host that app.
Rerun 2026-09-26 on `zcash/extapp-series` @ `cbce6b97e2` (T3W1 non-frozen
`--apps` emulator and Zcash app built there), with the series' twins of these
commits (`6efe8a111d` Core, `6d574b5bf3` SDK) reverted as uncommitted changes:
`pr-drafts-rerun/logs/cancel-series-{both,core-only,sdk-only,neither}.log`.
The Core half was reverted by hand (`cancel-series-core-half-reverted.diff`),
since `git apply -R` no longer applies at the tip. The series carries the
macOS build fix `15de3664a6` as a commit; not needed on Linux.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/host-cancel @ `856f1d72c7`
(two commits on `extapp/sdk-progress-api` @ `ba03520ba7`)

---

If the host sends `Cancel` while Core waits for its answer to an app's
`WireContinue`, `run()` ends but the app is still waiting. When the app then
ends its progress screen during the next request, Core finds no screen and
stops the app until the next `ExtAppLoad`.

- `d42ae69a5c` fix(core): let an extapp end a progress screen that is already gone
- `856f1d72c7` fix(sdk): forget the progress screen when the host abandons a request

**Reproduce.** We have seen this only with an out-of-tree app: the Zcash app
on github.com/bawolf/trezor-firmware `zcash/extapp-series`, which shows a
progress screen while it asks the host for the PCZT with `wire_request_raw`.
Its `test_sign_pczt_trezor_cancel` starts a signing, answers the first data
request with `messages.Cancel()`, then sends the app another request. On that
branch, with a non-frozen T3W1 `--apps` emulator and the app built with
`xtask modular build -p zcash -m t3w1 --lang en -e`, from `sdk/apps/zcash`:
`pytest --app=../target/artifacts/t3w1-emu/zcash.elf --lang=en tests/test_sign_pczt.py -k trezor_cancel`.
With the series' twins of both commits reverted, and as it is:

```
before: trezorlib.exceptions.TrezorFailure: DataError: Task not running: 224053394
after:  PASSED tests/test_sign_pczt.py::test_sign_pczt_trezor_cancel
```

**Cause.** The next `ExtAppMessage` starts a fresh `run()` with no progress
screen and delivers `WireStart`. The app's pending call returns
`UnexpectedService(WireStart)`, its handler fails, and dropping its progress
sends `End` for a screen Core no longer shows: "Progress not initialized", and
Core stops the app.

**Fix.**
- Core: a progress `End` with no screen is a no-op. `Update` with no screen
  still stops the app.
- SDK: when `wire_request_raw` gets `UnexpectedService` carrying a `WireStart`,
  it forgets the progress screen, so ending it does not reach Core.

Either half prevents the stop; each side should tolerate the other's state.

**Not fixed.** The first request after the cancel is still answered with the
abandoned request's `WireError` (failure code 2, which trezorlib maps to
`ButtonExpected`), and
an app that is computing rather than waiting sends its `WireEnd` into the next
`run()`. Nothing tells the app that Core abandoned its request, and nothing
tells Core which request a message answers. A complete fix needs request ids on
the `WireStart`/`WireContinue`/`WireEnd`/`WireError` messages, or an abort
message from Core to the app when `run()` exits abnormally.

**Tests.** No Core unit test (`run()` needs a running app), and no SDK test
(the IPC mock cannot deliver a `WireStart` during a call). The out-of-tree
test on a T3W1 emulator: with both halves or either half alone the app
survives; with neither, `Task not running`. A second test,
`test_sign_pczt_after_trezor_cancel`, pins the stale answer as
`xfail(strict=True)`.

### Notes for QA
Cancel an app's request mid-flow from the host, then send another. The app
must still answer (the host may need to repeat that request once).
