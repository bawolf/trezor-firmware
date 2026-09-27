# PR drafts for the Trezor extapp contributions

Drafts only. Nothing here has been sent, pushed upstream or opened as a PR. The
user decides whether and how to contact Trezor.

Each draft has a status block for us above a `---` rule, and the text to paste
below it. One-commit fixes are two or three sentences plus the test, as
Trezor's own are; the multi-commit notes add Reproduce, Cause, Fix, Tests and
Notes for QA.

Evidence: on 2026-09-26 every before/after result was rerun on macOS on the
pushed branch tip and on its base (or the parent commit or branch, where the
status block says so); host-cancel's only on the integrated series, whose app
it needs. Receipts, each with its argv, sha, date and any local change:
`.context/product-scaffold/pr-drafts-rerun/logs/` in the trezor-ironwood
workspace. Emulator apps need our local macOS build fix `15de3664a6`
cherry-picked on top of both base and tip (not needed on Linux; for
emulator-declared-heap adapted, see its status); nothing else differs besides
the named scratch fixtures, which are in [repro/](repro/).

Bases: `bieleluk/sdk-wip` @ `4cd93ff4d8` (unchanged upstream on 2026-09-26);
`main` @ `8760d111b6` (the UA fix merges cleanly with `main` @ `3227ea8110`,
the current `upstream/main`; quote that one in anything sent).

## Index

Why sdk-wip: `run.py`'s dispatch, the Rust bridges, `trezor-app-sdk`,
`modular-xtask` and the sample apps exist only on the SDK draft branches
(#7516). `run.py` does not exist on `main`; #7947 proposes a stub. So every
sdk-wip draft is a note for the branch owner, not a PR.

| Draft | Branch @ sha | Target | Kind | Owner / reviewer | Depends on | Reproduction |
|---|---|---|---|---|---|---|
| [run-log-pyopt](run-log-pyopt.md) | `extapp/run-log-pyopt` @ `cf7d5bc6a3` | sdk-wip | note: bug fix | bieleluk; matejcik | — | standalone (PYOPT=1 T3T1 emulator: Core fatal before, xpub after); no test |
| [killed-not-timeout](killed-not-timeout.md) | `extapp/killed-not-timeout` @ `fc136e637c` | sdk-wip | note: bug fix | bieleluk | — | standalone (scratch Tron panic: "Timeout" before, "Task stopped" after); no test |
| [run-coin-types](run-coin-types.md) | `extapp/run-coin-types` @ `7b126357ca` | sdk-wip | note: 2 bug fixes | bieleluk | — | commit 1 standalone (sdk-wip's function run on the host); commit 2 test fails (Core fatal) before, passes after |
| [typed-hash-entitlement](typed-hash-entitlement.md) | `extapp/typed-hash-entitlement` @ `23599d27b6` | sdk-wip | note: security bug fix | bieleluk; matejcik | run-coin-types (built on it; same test file) | standalone end to end (scratch Tron app gets an Ethereum signature before, refused after); 4 unit tests after, full Core unit suite passes |
| [sdk-allocator-heap](sdk-allocator-heap.md) | `extapp/sdk-allocator-heap` @ `5f235435ed` | sdk-wip | note: bug fix | bieleluk | — (trivial manifest conflict with ipc-buffer-size) | standalone, deterministic (xtask size report); Ethereum subset: 3 allocator panics before, none after |
| [emulator-declared-heap](emulator-declared-heap.md) | `extapp/emulator-declared-heap` @ `d502e1ed4d` | sdk-wip | note: feature (+ sample fix) | cepetr (loader); bieleluk | sdk-allocator-heap | standalone (Ethereum suite at declared heaps) |
| [sdk-progress-api](sdk-progress-api.md) | `extapp/sdk-progress-api` @ `ba03520ba7` | sdk-wip | note: 2 bug fixes + feature | bieleluk; vojczejk (overlaps his modui progress) | — | standalone (Ethereum subset 0 → 8 of 15); SDK test after |
| [host-cancel](host-cancel.md) | `extapp/host-cancel` @ `856f1d72c7` | sdk-wip | note: partial bug fix | bieleluk | sdk-progress-api (SDK half) | only an out-of-tree app's device test, on the series (stopped before, alive after); no platform test |
| [bridge-validate-untrusted-input](bridge-validate-untrusted-input.md) | `extapp/bridge-validate-untrusted-input` @ `fc12e1b57b` | sdk-wip | note: bug fix (+ SDK API) | bieleluk; matejcik (`core/embed/rust`) | — | test fails (and Core fatal) before, passes after |
| [bridge-raise-not-rsod](bridge-raise-not-rsod.md) | `extapp/bridge-raise-not-rsod` @ `5ae2052a08` | sdk-wip | note: bug fix | matejcik; bieleluk | bridge-validate-untrusted-input | test fails (Core fatal) before, passes after |
| [ipc-buffer-size](ipc-buffer-size.md) | `extapp/ipc-buffer-size` @ `d0bd374a22` | sdk-wip | note: feature + bug fix | bieleluk (his `0be72a42dd` on stabby) | — (trivial manifest conflict with the heap branches) | commit 2 standalone (2 KiB inbox: app left waiting before, stopped after); xtask tests |
| [run-ack-instance-id](run-ack-instance-id.md) | `extapp/run-ack-instance-id` @ `1d0efa2f08` | sdk-wip | note: bug fix (latent) | bieleluk | — | standalone (scratch Ethereum test: answer taken before, refused after); no committed test |
| [rng](rng.md) | `extapp/rng` @ `ed9f103db3` | sdk-wip (commit 1 could go to `main`) | note: bug fix + feature | cepetr; TychoVrahe | — | none (code reading; builds only) |
| [xtask-production-rejects-debug](xtask-production-rejects-debug.md) | `extapp/xtask-production-rejects-debug` @ `ef50fa6a04` | sdk-wip (or `cepetr/apptool`) | note: bug fix | cepetr; bieleluk | — | build command accepted before, refused after; unit test after |
| [xtask-own-workspace](xtask-own-workspace.md) | `extapp/xtask-own-workspace` @ `e3593e829c` | — | reserve, **not proposed** (commit 1 could go alone) | cepetr | — | none |
| [zcash-ua-decode-short-payload](zcash-ua-decode-short-payload.md) | `extapp/zcash-ua-decode-short-payload` @ `935383396b` | **`main`** (code on `main`) | **PR**: bug fix | obrusvit (CODEOWNERS `*`) or matejcik | an issue first (`Fixes #N`) | 2 tests fail before, pass after |
| [key-service](key-service.md) | `zcash/extapp-series` @ `c74600c911`: `8bd8923160`, `3b178fdebd`, `f9aace94f8` | sdk-wip, after design agreement | design proposal (feature) | bieleluk, matejcik; product | bridge-validate, bridge-raise, run-coin-types (in the series) | n/a (15 Core unit tests; end to end via the app) |
| [zcash-app](zcash-app.md) | `zcash/extapp-series` @ `c74600c911`: `1f22828ea3`, `c040431244`, `407da85e06`, `c74600c911` | sdk-wip, after product agreement | feature proposal | bieleluk; product (Hannsek) | the whole platform part of the series, key service included | n/a (77 passed, 1 xfailed on T3W1 and T3T1 emulators) |

Also here: [00-introduction](00-introduction.md) (first contact) and
[DEMO](DEMO.md) (reproducing the demo from a clean clone).

The integrated series is
https://github.com/bawolf/trezor-firmware/tree/zcash/extapp-series @ `c74600c911`.
Its commits carry the same changed lines and messages as the branches, except
`emulator-declared-heap` (the series also covers the local macOS loader).
`extapp/typed-hash-entitlement` is the series commit `b77222b9ae`
cherry-picked onto `extapp/run-coin-types`.

## History

- Already done: `extapp/typed-hash-entitlement` was rebuilt from the series
  commit on 2026-09-26 and force-pushed (`3e668c7caf` → `cdafac07e0`). The
  earlier push also changed the `SignDigest`, `GetAddressMac` and
  `CheckAddressMac` arms, which its message did not say.
- The older local `fix/*` branches are superseded by these `extapp/*` ones;
  `fix/extapp-progress-variant-id` by `extapp/bridge-validate-untrusted-input`.
- run-coin-types commit 2 was reworded (`4640bbee69` → `7b126357ca`, same tree), and
  typed-hash-entitlement was rebased onto it (`cdafac07e0` → `23599d27b6`, same tree).

- sdk-progress-api commit 1 was reformatted with rustfmt (one call wrapped; `65166b3176` →
  `ba03520ba7`), and host-cancel was rebased onto it (`4a5a41f4c3` → `856f1d72c7`). The fork's
  CI format check caught it. Receipts from before the reformat still apply: only whitespace
  changed.

## Daily sync

From 2026-09-26 a daily job (launchd `com.bawolf.trezor-daily-sync`, 06:17; script and
reports in the author's `…/ca/work/daily-sync/`) keeps the fork current:
- the fork's `main` gets upstream `main` merged in;
- `extapp/zcash-ua-decode-short-payload` and `zcash/ironwood-upstream-v2` are rebased onto
  upstream `main`;
- every sdk-wip branch is rebased when `bieleluk/sdk-wip` moves, in dependency order;
- the private Suite branches are rebased onto `develop`.

It pushes only clean rebases and reports conflicts instead. The shas quoted in these drafts are
therefore the ones the evidence ran on. Link branches by name. On 2026-09-26 the UA fix moved
`935383396b` → `d4ee82c0dc` and v2 moved `7864a22444` → `3e4018eacc` (both clean). v2 was not
re-gated after its rebase.

## CI on the fork's review PRs

The review PRs on `bawolf/trezor-firmware` (#2–#16) run Trezor's own GitHub workflows. Most red
checks there are inherited from `bieleluk/sdk-wip` itself: Trezor's PR #7516, at the same base
commit `4cd93ff4d8`, fails the same jobs. They are:
- Build firmware (T3W1, universal, normal): `prodtest_nfc_backup.c` signedness errors;
- Python test: a trezorlib dataclass `TypeError` ("non-default argument 'code_size'");
- Rust dependencies check: `cargo vet` is missing `thiserror`;
- Style check: pyright;
- Changelog check;
- the device-test matrix.

For each PR the set of failing checks was compared with #7516's (2026-09-26). The only one that
was ours was the extapp format check on sdk-progress-api, now fixed. `modular-xtask / Format
check` also fails on the base (`postbuild.rs`). #16 is based on `main` and must be judged on its
own.

- 2026-09-27: the series and the demo were reformatted the same way (one call in
  `sdk/apps/ethereum/src/helpers.rs`; `cbce6b97e2` → `c74600c911`, `7a438a4399` → `a93365d0c0`).
  The Safe 7 bundle `extapp-7a438a4399` and the first Suite send from a Safe 7 (block 4,401,482)
  were built at the old demo commit, kept as tag `archive/extapp-safe7-suite-send-7a438a4399`.
  The firmware and app sources are unchanged.

## Suggested order

1. **Introduce the work** with an issue or an email (Hannsek's reply on #6770
   names two contacts) and the video:
   [00-introduction](00-introduction.md). Ask how they want contributions to
   the WIP SDK; wait for an answer before sending anything else.
2. **The small fixes that stand alone**, in the form they ask for:
   run-log-pyopt first (every other `run.py` branch carries the base's
   unguarded `log` calls until it lands), killed-not-timeout, run-coin-types,
   then typed-hash-entitlement (after run-coin-types, which it builds on),
   run-ack-instance-id,
   xtask-production-rejects-debug, sdk-allocator-heap then
   emulator-declared-heap, bridge-validate-untrusted-input then
   bridge-raise-not-rsod, sdk-progress-api then host-cancel, ipc-buffer-size
   and rng (both need an owner decision). The UA fix goes to `main` through
   the normal issue-then-PR route.
3. **The key-service proposal**, which needs their design agreement: whether a
   Core service may give an app the spending key (the direct-key entitlement)
   and what the consent screen says.
4. **The Zcash app** last.

## Security handling

typed-hash-entitlement, bridge-validate-untrusted-input and
bridge-raise-not-rsod fix security-relevant bugs, all in unreleased WIP code
with no users. The fixes, their commit messages and tests are already public
on github.com/bawolf/trezor-firmware, so they are handled alike, as ordinary
notes. You may still prefer to email typed-hash-entitlement to the branch
owner before posting it anywhere.

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
