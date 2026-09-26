# PR drafts for the Trezor extapp contributions

Drafts only. Nothing here has been sent, pushed upstream or opened as a PR. The
user decides whether and how to contact Trezor.

Each draft has a status block for us above a `---` rule, and the text to paste
below it: a lead sentence, then Reproduce, Cause, Fix, Tests and Notes for QA,
in Trezor's shape. Checked on 2026-09-26 against the branches (`git log`,
`git diff <base>..<branch>`) and the recorded receipts of the runs quoted.

Bases: `bieleluk/sdk-wip` @ `4cd93ff4d8` (unchanged upstream on 2026-09-26);
`main` @ `8760d111b6` (the UA fix merges cleanly with `main` @ `3227ea8110`).

## Index

Why sdk-wip: `run.py`'s dispatch, the Rust bridges, `trezor-app-sdk`,
`modular-xtask` and the sample apps exist only on the SDK draft branches
(#7516); on `main`, `run.py` is still a stub. So every sdk-wip draft is a note
for the branch owner, not a PR.

| Draft | Branch @ sha | Target | Kind | Owner / reviewer | Depends on | Reproduction |
|---|---|---|---|---|---|---|
| [run-log-pyopt](run-log-pyopt.md) | `extapp/run-log-pyopt` @ `cf7d5bc6a3` | sdk-wip | note: bug fix | bieleluk; matejcik | — | standalone (PYOPT=1 emulator: Core fatal before, xpub after); no test |
| [killed-not-timeout](killed-not-timeout.md) | `extapp/killed-not-timeout` @ `fc136e637c` | sdk-wip | note: bug fix | bieleluk | — | standalone (scratch Tron panic: "Timeout" before, "Task stopped" after); no test |
| [run-coin-types](run-coin-types.md) | `extapp/run-coin-types` @ `4640bbee69` | sdk-wip | note: 2 bug fixes | bieleluk | — | commit 1 standalone (extracted function); commit 2 test fails (Core fatal) before, passes after |
| [typed-hash-entitlement](typed-hash-entitlement.md) | `extapp/typed-hash-entitlement-v2` @ `cdafac07e0` (local; replaces the pushed `extapp/typed-hash-entitlement` @ `3e668c7caf`) | sdk-wip | note: security bug fix | bieleluk; matejcik | run-coin-types (built on it; same test file) | standalone end to end (scratch Tron app gets an Ethereum signature before, refused after); 4 unit tests after, full Core unit suite passes |
| [sdk-allocator-heap](sdk-allocator-heap.md) | `extapp/sdk-allocator-heap` @ `5f235435ed` | sdk-wip | note: bug fix | bieleluk | — (trivial manifest conflict with ipc-buffer-size) | standalone, deterministic (xtask size report) |
| [emulator-declared-heap](emulator-declared-heap.md) | `extapp/emulator-declared-heap` @ `d502e1ed4d` | sdk-wip | note: feature (+ sample fix) | cepetr (loader); bieleluk | sdk-allocator-heap | standalone (Ethereum suite at declared heaps) |
| [sdk-progress-api](sdk-progress-api.md) | `extapp/sdk-progress-api` @ `65166b3176` | sdk-wip | note: 2 bug fixes + feature | bieleluk; vojczejk (overlaps his modui progress) | — | standalone (Ethereum subset 0 → 8 of 15); SDK test after |
| [host-cancel](host-cancel.md) | `extapp/host-cancel` @ `4a5a41f4c3` | sdk-wip | note: partial bug fix | bieleluk | sdk-progress-api (SDK half) | only an out-of-tree app's device test (stopped before, alive after); no platform test |
| [bridge-validate-untrusted-input](bridge-validate-untrusted-input.md) | `extapp/bridge-validate-untrusted-input` @ `fc12e1b57b` | sdk-wip | note: bug fix (+ SDK API) | bieleluk; matejcik (`core/embed/rust`) | — | test fails (and Core fatal) before, passes after |
| [bridge-raise-not-rsod](bridge-raise-not-rsod.md) | `extapp/bridge-raise-not-rsod` @ `5ae2052a08` | sdk-wip | note: bug fix | matejcik; bieleluk | bridge-validate-untrusted-input | test fails (Core fatal) before, passes after |
| [ipc-buffer-size](ipc-buffer-size.md) | `extapp/ipc-buffer-size` @ `d0bd374a22` | sdk-wip | note: feature + bug fix | bieleluk (his `0be72a42dd` on stabby) | — (trivial manifest conflict with the heap branches) | commit 2 standalone (2 KiB inbox: stale answer before, app stopped after); xtask tests |
| [run-ack-instance-id](run-ack-instance-id.md) | `extapp/run-ack-instance-id` @ `1d0efa2f08` | sdk-wip | note: bug fix (latent) | bieleluk | — | only an out-of-tree app's device test (fails before, passes after) |
| [rng](rng.md) | `extapp/rng` @ `ed9f103db3` | sdk-wip (commit 1 could go to `main`) | note: bug fix + feature | cepetr; TychoVrahe | — | none (code reading; builds only) |
| [xtask-production-rejects-debug](xtask-production-rejects-debug.md) | `extapp/xtask-production-rejects-debug` @ `ef50fa6a04` | sdk-wip (or `cepetr/apptool`) | note: bug fix | cepetr; bieleluk | — | unit test after; not run against the base |
| [xtask-own-workspace](xtask-own-workspace.md) | `extapp/xtask-own-workspace` @ `e3593e829c` | — | reserve, **not proposed** (commit 1 could go alone) | cepetr | — | none |
| [zcash-ua-decode-short-payload](zcash-ua-decode-short-payload.md) | `extapp/zcash-ua-decode-short-payload` @ `935383396b` | **`main`** (code on `main`) | **PR**: bug fix | obrusvit (CODEOWNERS `*`) or matejcik | an issue first (`Fixes #N`) | 2 tests fail before, pass after |
| [key-service](key-service.md) | `zcash/extapp-series` @ `cbce6b97e2`: `f8a8261663`, `5f7e86b229`, `5620cc3c57` | sdk-wip, after design agreement | design proposal (feature) | bieleluk, matejcik; product | bridge-validate, bridge-raise, run-coin-types (in the series) | n/a (15 Core unit tests; end to end via the app) |
| [zcash-app](zcash-app.md) | `zcash/extapp-series` @ `cbce6b97e2`: `1a9708f824`, `ead470546a`, `bc13c3a72d`, `cbce6b97e2` | sdk-wip, after product agreement | feature proposal | bieleluk; product (Hannsek) | the whole platform part of the series, key service included | n/a (77 passed, 1 xfailed on T3W1 and T3T1 emulators) |

Also here: [00-introduction](00-introduction.md) (first contact) and
[DEMO](DEMO.md) (reproducing the demo from a clean clone).

The integrated series is
https://github.com/bawolf/trezor-firmware/tree/zcash/extapp-series @ `cbce6b97e2`.
Its commits carry the same changed lines and messages as the branches, except
`emulator-declared-heap` (the series also covers the local macOS loader).
`extapp/typed-hash-entitlement-v2` is the series commit `b77222b9ae`
cherry-picked onto `extapp/run-coin-types`.

## Fix before sending

- **Push `extapp/typed-hash-entitlement-v2` before linking the draft.** The
  pushed `extapp/typed-hash-entitlement` @ `3e668c7caf` also switches the
  `SignDigest`, `GetAddressMac` and `CheckAddressMac` arms to `"secp256k1"`,
  which its message does not mention. The rebuilt local branch has only the
  series commit's change and passed its checks (see the draft). Pushing it
  over the old name or under the new one is the user's call.
- The older local `fix/*` branches are superseded by these `extapp/*` ones;
  `fix/extapp-progress-variant-id` by `extapp/bridge-validate-untrusted-input`.

## Suggested order

1. **Introduce the work** with an issue or an email (per #6770) and the video:
   [00-introduction](00-introduction.md). Ask how they want contributions to
   the WIP SDK; wait for an answer before sending anything else.
2. **The small fixes that stand alone**, in the form they ask for:
   run-log-pyopt, killed-not-timeout, run-coin-types, then
   typed-hash-entitlement (after run-coin-types, which it builds on; it is
   security-relevant, so perhaps privately to the owner), run-ack-instance-id,
   xtask-production-rejects-debug, sdk-allocator-heap then
   emulator-declared-heap, bridge-validate-untrusted-input then
   bridge-raise-not-rsod, sdk-progress-api then host-cancel, ipc-buffer-size
   and rng (both need an owner decision). The UA fix goes to `main` through
   the normal issue-then-PR route.
3. **The key-service proposal**, which needs their design agreement: whether a
   Core service may give an app the spending key (the direct-key entitlement)
   and what the consent screen says.
4. **The Zcash app** last.

## Trezor's decisions, not ours

- **The app id.** `zcash.trezor.com` is a placeholder; the vendor field says
  "Bryant Wolf".
- **Production root keys.** Apps load only with the dev app root key: on
  emulators and `--bootloader-devel` builds, or with our never-proposed
  test-device commits. Signing an app for production devices is Trezor's.
- **Whether a Core service may hand an app a spending key**, how an app is
  entitled to it (a pseudo-curve or a manifest capability), and the consent
  screen.
- **The fork dependencies.** orchard and sinsemilla come from
  github.com/bawolf forks for APIs no release has; they block merge until
  released crates replace them.
- **The cargo vet exemptions.** 62 new in `sdk/apps/supply-chain` and 149 in
  `sdk/apps/zcash/signer-tests/supply-chain`, none audited.
- Also for the owners: the IPC inbox design (static vs. stabby's heap inbox),
  `ui::Progress` vs. vojczejk's modui progress, the RNG in `trezor_api_v1_t`
  vs. a v2, and the protocol change a complete host-cancel fix needs.
