# Size an extapp's IPC inbox from its manifest

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The SDK
and xtask exist only on the draft SDK branches. Needs an owner decision:
bieleluk's `0be72a42dd` on `bieleluk/stabby` (#7882) adds the same manifest key
with the inbox on the heap and its size in the header. That commit cannot be
applied to sdk-wip (its consumer is stabby's Core-side runtime), so this branch
keeps a static inbox and follows its key and rules.
Kind: feature (commit 1) and bug fix (commit 2).
Reproduction status: commit 2, standalone (scratch Ethereum build with a 2 KiB
inbox); commit 1, xtask unit tests.
Rerun 2026-09-26 with `run.py` at commit 1 `f9a863cf2b` (before) and at the
pushed tip `d0bd374a22` (after), T3W1 non-frozen `--apps` emulator built at
each, same app: `pr-drafts-rerun/logs/ipc-oversize-{commit1,tip}.log`; at the
tip, xtask tests (`ipc-tip-xtask-test.log`), SDK checks
(`ipc-tip-sdk.log`) and Core unit tests (`ipc-tip-unit-full.log`). Local macOS
build fix only (`15de3664a6` cherry-picked for the emulator app); not needed on
Linux.
Conflicts trivially with `extapp/sdk-allocator-heap` in the sample manifests
(adjacent lines).

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/ipc-buffer-size @ `d0bd374a22`
(two commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

Every app got a 16 KiB static inbox for Core's messages, counted against its
RAM arena, whatever the largest message it receives.

- `f9a863cf2b` feat(sdk,xtask): size the static IPC inbox from the app manifest
- `d0bd374a22` fix(core): stop an extapp whose host message does not fit its inbox

**Reproduce (commit 2).** Build the Ethereum sample with
`ipc-buffer-size = 2048` ([repro/ipc-buffer-size/manifest.patch](repro/ipc-buffer-size/manifest.patch))
and run the scratch [test_zz_oversize.py](repro/ipc-buffer-size/test_zz_oversize.py)
from `sdk/apps/ethereum/tests/`. It starts a `SignTx` with 3000 bytes of data,
answers its `TxRequest` with a `TxAck` carrying a 4 KiB chunk, then sends
`GetAddress`:

```
before: TxAck -> TrezorFailure: FirmwareError: Failed to send IPC message.
        GetAddress -> TrezorFailure: ButtonExpected:
after:  TxAck -> TrezorFailure: DataError: Failed to send IPC message
        GetAddress -> TrezorFailure: DataError: Task not running: 224053394
```

Before, the app is still waiting in the old request when `GetAddress`
arrives.

**Cause.** `run.py` forwards the host's answer to an app's `WireContinue`, and
the request after an app's `WireError`, with an unguarded `io.ipc_send`. The
exception ended the host request but left the app waiting for a message that
never came.

**Fix.**
- Optional `ipc-buffer-size` under `[package.metadata.trezor]`: a power of two
  from 256 B to 64 KiB; absent or 0 means 1 KiB, which holds the replies of
  Core's services. xtask validates it and passes it as
  `TREZOR_APP_IPC_BUFFER_SIZE`; the SDK sizes its static inbox from it.
- The Ethereum and Tron samples receive host requests and declare 16 KiB: THP's
  host buffer is 8,704 B.
- The two unguarded sends stop the app with `DataError`, as the guarded sends
  already do.

**Tests.** `ipc_buffer_size_defaults_to_1_kib`,
`ipc_buffer_size_is_a_power_of_two_in_range`; modular-xtask tests (37 + 5
doctests) and SDK checks pass; Core unit tests 135/135.

### Notes for QA
An out-of-tree app that does not declare the key now gets 1 KiB instead of
16 KiB, and must declare its largest host message. The size is read at build
time per cargo invocation, so build one app per invocation.
