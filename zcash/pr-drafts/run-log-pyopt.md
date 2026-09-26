# fix(core): guard extapp run.py logging with __debug__

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. `run.py`'s
dispatch exists only on the draft SDK branches (identical on `bieleluk/stabby`,
`cepetr/apptool`, `vojczejk/sdk-wip-modui`); on `main` it is still a stub.
Reproduction status: standalone repro on a PYOPT=1 emulator; no committed test.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/run-log-pyopt @ `cf7d5bc6a3`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`run.py` imports `log` only under `if __debug__:`, but calls it unguarded in 13
places. On a PYOPT=1 build, which every hardware build is, the first crypto
reply to any app takes Core down.

**Reproduce.** T3T1 emulator without debuglink, device already seeded (with a
PYOPT=0 build of the same tree), Ethereum sample loaded over the normal wire:

```sh
uv run xtask build firmware --emulator --model T3T1 --apps --frozen --pyopt true --debug-link false --disable-animation
xtask modular build -p ethereum -m t3t1 --lang en -e
```

Then load the app with `trezorlib.extapp.load(...)` and send the sample's
`GetPublicKey(m/44'/60'/0')` in an `ExtAppMessage`. It shows no screen, so no
debuglink is needed. Observed:

```
RESULT: GetPublicKey got no answer: Timeout: Timeout reading UDP packet (20s)
AFTER: Core does not answer: TransportException: Error opening udp:127.0.0.1:21490
Fatal: unwrap failed at rust/src/crypto/api/firmware_micropython.rs:157
```

With the fix, the xpub comes back and Core still answers `GetFeatures`. T3T1
because a PYOPT=1 T3W1 speaks only THP, whose pairing needs someone to read the
screen; the bug does not depend on the model.

**Cause.** `crypto_resp_cb` calls `log.debug`, which is unbound under PYOPT=1.
The `NameError` is raised inside `send_crypto_result`, which `unwrap!`s the
callback's result. The call after each UI interaction ends the request with a
`NameError` too.

**Fix.** Each `log.*` call goes under `if __debug__:`. Nothing changes under
PYOPT=0.

**Tests.** None committed: a PYOPT=1 device test needs a harness without
debuglink, which the repo does not have. A scan of `run.py` finds 13 unguarded
calls before and 0 after.

### Notes for QA
On a PYOPT=1 build, load any app and make it request a key. Before the fix Core
stops with "unwrap failed"; after it the app answers.
