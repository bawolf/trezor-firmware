# fix(core): guard extapp run.py logging with __debug__

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. `run.py`
exists only on the draft SDK branches: `cepetr/apptool` has the identical
file; `bieleluk/stabby` has the same bug with two more unguarded calls, and
`vojczejk/sdk-wip-modui` too; this patch is against sdk-wip
(`pr-drafts-rerun/logs/scan-branches.log`). No committed test.
Rerun 2026-09-26 on T3T1 frozen PYOPT=1 emulators without debuglink, built at
sdk-wip and at the pushed tip `cf7d5bc6a3`, on a profile seeded by a PYOPT=0
build of sdk-wip: `pr-drafts-rerun/logs/pyopt-{seed,base,tip}.log`. Local macOS
build fix only (`15de3664a6` cherry-picked for the emulator app); not needed on
Linux. Land this first: the other `run.py` branches carry the base's unguarded
calls until it does.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/run-log-pyopt @ `cf7d5bc6a3`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`run.py` imports `log` only under `if __debug__:`, but called it unguarded in
13 places, so on a PYOPT=1 build the first crypto reply to any app ends Core:
the `NameError` is raised inside `send_crypto_result`, which `unwrap!`s the
callback's result. Each `log.*` call now goes under `if __debug__:`; nothing
changes under PYOPT=0.

Tested on a T3T1 emulator built with
`xtask build firmware --emulator --model T3T1 --apps --frozen --pyopt true --debug-link false --disable-animation`,
seeded first through a PYOPT=0 build ([repro/run-log-pyopt/seed.py](repro/run-log-pyopt/seed.py)).
[repro_pyopt_getpublickey.py](repro/run-log-pyopt/repro_pyopt_getpublickey.py)
loads the Ethereum sample (`xtask modular build -p ethereum -m t3t1 --lang en -d -e`)
and sends it `GetPublicKey(m/44'/60'/0')`, which shows no screen:

```
before: GetPublicKey got no answer: Timeout: Timeout reading UDP packet (20s)
        GetFeatures: TransportException: Error opening udp:127.0.0.1:21570
after:  GetPublicKey: xpub6CNFa58kEQJu...
        GetFeatures: Core answers
```

Before, the emulator console shows
`Fatal: unwrap failed at rust/src/crypto/api/firmware_micropython.rs:157`.

T3T1 because a T3W1 needs THP pairing, which without debuglink needs someone
to read the screen; the bug does not depend on the model. A committed test
would need a PYOPT=1 device-test harness without debuglink, which the repo
does not have.
