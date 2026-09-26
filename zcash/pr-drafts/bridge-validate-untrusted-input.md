# Validate extapp IPC requests before reading them

Status: note for the owners of `bieleluk/sdk-wip` (#7516), not a PR. The
bridges and `trezor-app-sdk` exist only on the draft SDK branches.
Reproduction status: a Core unit test fails before (and ends Core) and passes
after.
`extapp/bridge-raise-not-rsod` stacks on this branch.
Rerun 2026-09-26: the tip's test file on a T3W1 non-frozen `--apps` emulator
of sdk-wip and of the pushed tip `fc12e1b57b`, each built at its tree
(`pr-drafts-rerun/logs/bridge-validate-{base,tip}.log`); SDK checks at the tip
(`bridge-validate-tip-sdk.log`); flash with `--apps --bootloader-devel` at
both (`build-hw-{T3T1,T3W1}-{sdkwip,validate}.log`). No local changes. The
`cargo vet` count is from the series at `2839206d35`, which differs from
`cbce6b97e2` only in a Zcash test file; not rerun.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/bridge-validate-untrusted-input @ `fc12e1b57b`
(two commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

Core read a loaded app's Crypto, UI and Progress requests with
`rkyv::access_unchecked`, so a malformed archive from the app was dereferenced
before any check. The progress bridge also trusted the archive to hold the
operation its IPC id named.

- `52a84bf3dd` feat(sdk): add checked accessors for extapp IPC requests
- `fc12e1b57b` fix(core): validate extapp IPC requests before reading them

**Reproduce.** This branch's new `core/tests/test_apps.extapp.ipc.py`, copied
into an sdk-wip checkout and run on a non-frozen T3W1 `--apps` emulator built
from it (`cd core/tests && ./run_tests.sh test_apps.extapp.ipc.py`;
tracebacks and the home directory trimmed):

```
  test_crypto_request ... failed
AssertionError: <class 'ValueError'> not raised
  test_progress_request ... failed
AssertionError: <class 'ValueError'> not raised
  test_ui_request ...Task #1 terminated.
Fatal: rs at …/.cargo/registry/src/index.crates.io-1949cf8c6:41
```

A progress `End` sent under the `Init` id reached the `Init` branch, and
malformed UI bytes were read as an archive until Core stopped with a fatal
error in a dependency (the message truncates the path).

**Fix.**
- The SDK gains `access_crypto_request(bytes, id)`, `access_ui_request(bytes)`
  and `access_progress_request(bytes, id)`, which validate the whole archive
  with rkyv's checked `access` (bounds, alignment, tags, relative pointers,
  UTF-8). The crypto and progress ones also require the archived operation to
  be `id`.
- Core reads all three request kinds through them. A bad crypto request gets
  the service's failure result; a bad UI or Progress request stops the app
  with `DataError` (detail only under `__debug__`).
- rkyv's `bytecheck` is enabled without the SDK's `app` feature, which is how
  Core links the crate. This adds `bytecheck`, `bytecheck_derive` and
  `simdutf8` to `core/embed/Cargo.lock`, with `cargo vet` exemptions.
- Flash: T3T1 +2.5 KB (1576.5 → 1579.0 KB), T3W1 +3.0 KB (2301.0 →
  2304.0 KB), `--apps --bootloader-devel`.

**Tests.**
- SDK: `crypto_request_must_match_the_operation_id`,
  `progress_request_must_match_the_operation_id`,
  `malformed_crypto_requests_are_refused`, `misaligned_requests_are_refused`.
- Core: `test_crypto_request` (an archived `GetPublicKey` under ids 1 and 0),
  `test_progress_request` (`End` under ids 2, 0 and 1), `test_ui_request`,
  each with malformed input. 3/3 after.
- `cargo vet` in `core/embed` fails only on the 101 crates sdk-wip already
  leaves unvetted.

### Notes for QA
Apps built with the SDK send well-formed requests, so nothing changes for
them. A malformed request now fails that request or stops the app; Core stays
up.
