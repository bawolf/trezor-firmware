# Keep an extapp alive when the host abandons its request

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches. A bounded, partial fix: a complete one
needs a protocol change (below), which is the owner's call.
Stacked on `extapp/sdk-progress-api` (the SDK half needs its screen tracking);
the Core half applies to sdk-wip alone.
Reproduction status: observed only with a device test of an out-of-tree app
that streams data under a progress screen; no platform-level test.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/host-cancel @ `4a5a41f4c3`
(two commits on `extapp/sdk-progress-api` @ `65166b3176`)

---

If the host sends `Cancel` while Core waits for its answer to an app's
`WireContinue`, `run()` ends but the app is still waiting. When the app then
ends its progress screen during the next request, Core finds no screen and
stops the app until the next `ExtAppLoad`. Suite sends `Cancel` routinely.

- `e6a32ccc0a` fix(core): let an extapp end a progress screen that is already gone
- `4a5a41f4c3` fix(sdk): forget the progress screen when the host abandons a request

**Reproduce.** With an app that shows a progress screen and asks the host for
data with `wire_request_raw`: start its request, answer the first data request
with `messages.Cancel()`, then send the app another request. Observed:

```
before: DataError: Task not running: 224053394
after:  the app answers (the first request after the cancel gets a stale failure, see below)
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
abandoned request's `WireError` (trezorlib shows it as `ButtonExpected`), and
an app that is computing rather than waiting sends its `WireEnd` into the next
`run()`. Nothing tells the app that Core abandoned its request, and nothing
tells Core which request a message answers. A complete fix needs request ids on
the `WireStart`/`WireContinue`/`WireEnd`/`WireError` messages, or an abort
message from Core to the app when `run()` exits abnormally.

**Tests.** No Core unit test (`run()` needs a running app), and no SDK test
(the IPC mock cannot deliver a `WireStart` during a call). The out-of-tree
test, T3W1 emulator: both halves, either half alone: the app survives; neither:
`Task not running`. A second test pins the stale answer as `xfail(strict=True)`.

### Notes for QA
Cancel an app's request mid-flow from the host, then send another. The app
must still answer (the host may need to repeat that request once).
