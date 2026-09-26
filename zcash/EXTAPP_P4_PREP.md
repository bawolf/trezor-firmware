# Extapp P4 prep: the Zcash app, ready for a Safe 7 session

Date: 2026-09-25. Worktree
`/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-demo`. Nothing
has been pushed. This report is not committed. The model was Opus 5.5
(`claude-opus-5-5[1m]`) working alone, with no subagents and no reviews.

## Result

- **Watchdog margin.**
  - Progress is now reported at most 100 ms apart (it was 250 ms).
  - The first action's ivk-cache build is now a step of its own.
  - The longest host gap during verification fell from 1.24–1.36 ms to
    0.83–1.09 ms.
  - The ~250 ms target is **not reachable** at the app level on the P2b
    scaling estimate: the floor is the longest single orchard/pasta call,
    ≈0.35–0.7 s on a Cortex-M33 (§2).
  - The new diagnostic build measures the real value on the device.
- **Heap.** A debug-feature build now reports:
  - the heap high-water mark;
  - the longest IPC silence (plus its service and the message count).

  Release builds do not contain it (§3). In the emulator a 32-action sign
  peaks at 57,600 B; the declared heap is 73,728 B.
- **Blocker, with a proposed workaround.** No hardware image the P0/P1 recipe
  describes can load an app on this Safe 7 (§4):
  - `BOOTLOADER_DEVEL=1` images do not install on a production bootloader;
  - a non-devel image embeds the released secmon, which lacks the ML-DSA
    smcall that app loading needs.

  A test-device-only kernel change works around it. That image is built and
  staged, emulator-checked only. **Flashing it is a user decision.**
- **Bundle.**
  `/Users/bryantwolf/conductor/workspaces/trezor-ironwood/copenhagen/.context/product-scaffold/session-images/extapp-d485d458c5/`
  holds the firmware `.bin/.elf/.map` (plus the kernel ELF and map), the
  release and diagnostic app images with one dev Merkle tree and root packet,
  the host shim, `SHA256SUMS` and a `README.md` with the exact commands and
  the flash procedure.
- **Session script.** `copenhagen/.context/product-scaffold/session-logs/safe7-extapp-session.py`.
  Tested end to end in the emulator in both modes: debuglink auto-confirm, and
  the hardware path (`get_default_client`, with a scratch debuglink presser
  standing in for the user). Every step matched.
- **Device tests.** **57/57** twice: the 56 existing tests plus a new
  diagnostics test.

## 1. Commits

The commits are local; the identity is unchanged, there are no trailers, and
every commit is `[no changelog]`.

**Branch `zcash/extapp-demo`**, on `a7db99df0b`:

| Commit | Subject |
|---|---|
| `cd971229d2` | fix(core): keep an extapp's progress reports at most 100 ms apart |
| `a54c4c3758` | feat(core): report progress between ironwood's ivk cache and nullifier check |
| `863bf79dfd` | feat(core): count an extapp's heap peak and IPC silence in debug builds |
| `1c0a782e01` | feat(core): let a debug build of the Zcash extapp report its diagnostics |

**Branch `zcash/extapp-safe7`**, new and local, stacked on `1c0a782e01`. The
session firmware is built from it:

| Commit | Subject |
|---|---|
| `ad44c809e8` | feat(core): accept the dev app root key in non-production firmware |
| `d485d458c5` | feat(core): verify app root packets in the kernel with the released secmon (**test devices only; not for upstream**) |

The worktree is left on `zcash/extapp-demo` and clean.
`core/build-xtask` holds the frozen `PYOPT=0` emulator and the hardware
release build, both from `d485d458c5`.

## 2. Watchdog

Core stops an app that sends no IPC message for 1 s while it handles a
request (`run.py`, `loop.wait(..., timeout_ms=1000)`). The app's longest
silence is at most the report interval plus the longest step between two
progress calls.

**Changes.**
- **`Progress::KEEP_ALIVE_MS` 250 → 100** (SDK).
  - The longest step may now be 900 ms.
  - An app sends at most 10 reports a second.
  - On the device, where steps are ≈0.1–0.5 s, this is close to "report at
    every hook". Where hooks are dense (the 51 Sinsemilla ticks of each
    `commit_ivk` in receive), it keeps the IPC rate bounded.
  - The development guide states the bound.
- **`ironwood` `Body::action`.** It builds the scope classifier (two
  `Commit^ivk`) before the nullifier check, followed by a progress call. The
  later uses find it built, so the checks are unchanged. There are now 224
  hooks per 32 actions (was 223).

**Host gaps.** The harness is `/tmp/extapp-p2b/heap` (P2b's, rebuilt against
this tree): the series' 32-action vector, release build, quiet machine (load
2.2), two process runs of two rounds each.

| | Before (P2b) | Now |
|---|---|---|
| Median gap | 0.40 ms | 0.35–0.38 ms |
| p90 | 0.86 ms | 0.78–0.82 ms |
| Worst gap, verification | 1.24–1.36 ms (first action's nullifier + classifier) | **0.83–1.09 ms** |
| Prewarm parts (first sign only) | ≈1.0 / 1.1 ms | 1.0–1.6 / 1.0–1.4 ms |
| `SpendingKey::from_bytes` (before any progress screen) | 0.5–0.8 ms | 0.5–0.9 ms |

**Device estimate.** P2b's factor is 400–650×, from the Safe 5 series timing.
It may be pessimistic for the table-Sinsemilla app build.

| | Estimate |
|---|---|
| Worst verification step | ≈0.33–0.71 s |
| Prewarm parts | ≈0.4–0.65 s (up to ≈1.0 s on the cold 1.6 ms outlier) |
| `from_bytes` | ≈0.2–0.6 s |
| Longest silence (100 ms plus the longest step) | ≈0.45–0.8 s |

That is **below 1 s**, but not ≈250 ms. Each of those steps is one call into
`orchard`/`pasta_curves`:
- `verify_nullifier_with_classifier`;
- `SqrtTables` construction;
- `scope_classifier`;
- `SpendingKey::from_bytes`.

The app cannot split them without progress hooks inside the orchard fork
(`bawolf/orchard`, pinned by rev), or without Core raising its limit.

One cheap lever is not applied: prewarm could fill the `commit_ivk` domain
cache with one `fvk.to_ivk(External)` instead of `scope_classifier()`'s two
commitments. That halves that part.

**The session measures the real value.** With `--image diagnostic` the script
prints each step's longest silence and the service that ended it.

## 3. Heap and IPC diagnostics (debug builds only)

- **SDK `diagnostics` module.** It is compiled only with
  `all(feature = "app", feature = "debug", not(feature = "test"))`.
  - `PeakHeap` wraps the global `LlffHeap` and records `used()` after each
    allocation.
  - `IpcRemote::receive` and `IpcRemote::send` record the time from the last
    received message to the next send. This is Core's watchdog window: a
    `call`'s wait for its reply resets it, and so does a WireStart.
  - `diagnostics::take()` returns
    `{heap_size, heap_used, heap_peak, max_ipc_silence_ms, max_ipc_silence_service, ipc_sent}`.
    It resets the peak to the current use and the rest to zero.
- **Without `debug`,** the allocator is the plain `LlffHeap` and nothing is
  counted.
  - The release image has no `ZcashGetDiagnostics` handler; the app stops on
    that id, like on any unknown one.
  - Checks: the release build log shows features without `debug`; the release
    image is 2,752 B smaller than the diagnostic one.
- **App.**
  - `ZcashGetDiagnostics` (id 9) is answered with `ZcashDiagnostics` (10).
  - Host: `zcash_ext.get_diagnostics`.
  - Test: `test_diagnostics_measure_a_sign` checks:
    - after a 2-action sign, `0 < used < peak <= size`;
    - the silence is under 1000 ms;
    - an immediate second request reports `ipc_sent == 1` and a lower peak.
- **Hardware diagnostic image.** A release profile with `--features debug`.
  The SDK's `debug` feature also brings its panic handler, error context and
  logging; timing differences against release should be negligible.
- **Emulator numbers.** These are 64-bit, and the heap is the whole 64 MiB
  arena, so they bound the size only.

  | Step | Heap peak | Longest silence |
  |---|---|---|
  | Receive | 40,088 B | — |
  | Viewing key | 38,240 B | — |
  | 2-action sign | **57,600 B** | 3–16 ms |
  | 32-action sign | **57,600 B** | 3–16 ms |

  In use after the first sign: 32,240 B (the persistent caches). The 32-action
  sign sent 138 IPC messages.

  On hardware the heap is 73,728 B. The peak does not show fragmentation: an
  allocation failure would still abort the app.

## 4. Blocker: app loading on a production-bootloader Safe 7

**Why the recipe's image does not work here.** The P0/P1 recipe used
`make build_firmware BOOTLOADER_DEVEL=1` because only devel/emulator builds
carried the dev root-packet keys. That image:
- uses the `dev_DO_NOT_SIGN_signed_dev` vendor header, which a production
  bootloader rejects;
- embeds `bootloader_T3W1_devel.bin` (`firmware/build.rs`), `trezor-ble-dev`,
  and a source-built, dev-signed secmon (`kernel/build.rs`). A production
  bootloader verifies the secmon against the production keys
  (`fw_check.c` `check_secmon_header_sig`).

It is meant for devel-bootloader boards. The Safe 7 runs the production
bootloader: our `7864a22444` image carries it, and installs only because its
vendor header is prod-signed.

**What a non-devel `--apps` build hits.**
1. **It did not compile.** `ROOT_PACKET_KEYS` has no dev keys outside
   devel/emulator builds, and no model defines `MODEL_ROOT_PACKET_KEYS`.
2. **It embeds the released secmon** `models/T3W1/secmon/secmon.bin`
   (`464fcb1d…`, commit `6af91f1f32`, 2026-08-31). That predates
   `SMCALL_MLDSA44_VERIFY` (`0846f20419`, 2026-09-11).
   - In the disassembly, the released secmon's dispatch is
     `cmp r1, #65; tbh`, i.e. ids 1..66.
   - A secmon built from this tree has `cmp r1, #66`, i.e. ids 1..67; the
     ML-DSA smcall is 67.
   - The first root-packet verification (every first app load) would reach
     the secmon's `default: system_exit_fatal("Invalid smcall")`.
   - No newer signed T3W1 secmon exists on any upstream ref.

**Consequence.** Upstream app loading cannot yet run on any production-
bootloader T3W1 until Trezor ships a signed secmon with that smcall.

**Workaround on `zcash/extapp-safe7`.**
1. `ad44c809e8`: dev root keys in every non-production build (as for dev
   translations and definitions).
2. `d485d458c5`: `MLDSA44_IN_KERNEL`, defined in `sec/mldsa44.h` for
   `KERNEL && USE_SECMON_LAYOUT && !BOOTLOADER_DEVEL`.
   - `mldsa44.c` compiles in the kernel, and the smcall stub is left out.
     `nm`/`objdump`: the kernel's `mldsa44_verify` calls `mldsa_verify`, with
     no smcall.
   - mldsa-native is built with `MLD_CONFIG_REDUCE_RAM`.
   - Stack: static worst case of `mldsa_verify` 24,736 B (GCC 13.3
     `-fcallgraph-info=su` with the kernel's flags; 44,568 B without
     `REDUCE_RAM`). The stm32u5g kernel stack goes from 12K to 36K; kernel
     MAIN_RAM is 63,080 of 65,536 B. This `kernel.ld` also serves D002.
   - What changes: the signature check leaves the secure world. The keys, the
     check and its caller are unchanged.
   - Emulator coverage: the emulator runs the same C with `REDUCE_RAM` (57/57
     tests, each verifying a root packet; the session script). It cannot test
     the kernel stack. If the stack is short, the first load ends in a
     UsageFault; recover by reflashing.

**Alternatives, for the user.**
- Wait for a Trezor-signed secmon that has the smcall. Asking Trezor for one
  is upstream communication, which is not authorized.
- Use a devel-bootloader T3W1 board. We have none, and converting this device
  is a factory-security change.
- Keep the Safe 7 on the built-in series image for now.

## 5. Hardware images

### Firmware

`firmware-T3W1-extapp-d485d458c5.bin`:
- 2,370,048 B, sha256 `1fe52a24…7936`;
- header fingerprint `06214309…d9bbe`;
- `trezorlib.firmware.parse().verify()`: OK.

Build: `uv run xtask build firmware --model T3W1 --frozen --emit-memory-analysis --apps`
at a clean `d485d458c5`. Features: `universal_fw`, `dev_keys`, `frozen`,
`pyopt`, `app_loading`. There is no `debuglink`, `production` or
`bootloader_devel`.

| Image | FLASH (of 3336.0 KB) |
|---|---|
| Same tree without `--apps` (stock-equivalent: no app loading, no Zcash) | 2265.5 KB, 67.91 % |
| **Session image (`--apps`)** | **2314.5 KB, 69.38 %**, i.e. +49.0 KB |
| Built-in series image `7864a22444` (the coordinator's figure) | 2371.5 KB, 71.09 %, i.e. −57.0 KB against it |

The following are byte-identical to the `7864a22444` image now on the device:
- the vendor header;
- the embedded bootloader;
- the BLE app;
- the secmon.

### Apps (T3W1 hardware, release-fw)

| Image | File | code_size | data_size | Total | Share of the arena |
|---|---|---|---|---|---|
| `zcash.elf` (release) | 189,600 B | 189,088 | 123,040 | 312,128 B | 79.4 % |
| `zcash-diag.elf` | 192,352 B | 191,840 | 123,072 | 314,912 B | 80.1 % |

- **Dev bundle.** One Merkle tree over both images: proofs of 32 B each, and
  one ring-0 root packet of 4,900 B, timestamp 1790385137.
- **Why one bundle.** The device refuses a root packet older than the one it
  stores (`app_root_update`). Two separately built bundles would lock one
  image out after the other was loaded.
- **The script loads with the fingerprint** (`force_reload=True`). Otherwise
  `load.py` reuses a loaded image with the same id and version.

## 6. Session script

`safe7-extapp-session.py`. Run it with the worktree's
`.venv/bin/python`, whose trezorlib has `extapp`.

**Options.**
- `--bundle` (default: the newest `session-images/extapp-*`);
- `--image diagnostic|release`;
- `--path`;
- `--code-file` (default `/tmp/t7code.txt`);
- `--recover`;
- `--emulator`.

**What it does.**
1. Checks `SHA256SUMS`.
2. Pairs with `get_default_client(..., code_entry_callback=<poll the code file>, button_callback=...)`.
3. Prints the features: model, version, revision, vendor, `bootloader_locked`,
   `bootloader_mode`, init/PIN/passphrase, capabilities.
4. Loads the app and times it.
5. Runs:
   - testnet account 0 address #3 (expected `utest1qkmc…xqn0y7f5`);
   - the testnet UFVK prefix `uviewtest1uap6…gfv0q`;
   - `2_actions` and `32_actions` mainnet (expected `real_spend_actions` [1]
     and [29], with 64-byte signatures).
6. Prints `MATCH`/`MISMATCH`/`FAIL` for each.
7. Prints `STEP: [br_name] <what to do>` for every screen, the wall time and
   screen count per step (user time included), and, with the diagnostic
   image, `DIAG` lines and a `SUMMARY`.
8. After a failure it reloads the app for the next step.

**Emulator runs.** Private ports UDP 21391 and Tropic 21397; emulator bundle
`/tmp/extapp-p4/emu-bundle`, with emulator-arch images built the same way.

| Run | Result |
|---|---|
| `--emulator` diagnostic (demo emulator, then the Safe 7 emulator) | all MATCH |
| `--emulator` release | all MATCH |
| Hardware path against the emulator, with a scratch debuglink presser, diagnostic and release | all MATCH. The emulator offers SkipPairing, so no code was needed; the device showed `thp_pairing_request`. |
| 1 of 7 `--emulator` runs | `sign_32_actions` failed with "layout deadlock detected" after a debuglink `DebugLinkGetState` UDP timeout. This is harness only; the hardware path does not use debuglink. |

**Not exercised.**
- `--recover`. It is copied from `/tmp/safe7_session.py`, which recovered this
  device.
- The pairing-code file path; the emulator skipped pairing.

## 7. Flash procedure

The details are in the bundle `README.md`.

1. Quit Suite.
2. Run `/tmp/safe7_to_bootloader.py`: pair, confirm the reboot, and expect
   `locked: False`.
3. Run `.venv/bin/trezorctl firmware update -f <bundle>/firmware-T3W1-extapp-d485d458c5.bin`
   and check the fingerprint `0621 4309 …`. Confirm on the device ("UNSAFE,
   DO NOT USE!", 2.12.6). `-s` is not needed.

**Seed: expect it to be kept.**
- The vendor header hash is identical and the version equal, so
  `detect_installation` sets `keep_seed` (`wf_firmware_update.c`). That is
  this tree's source; the device's bootloader is the prebuilt
  `bootloader_T3W1.bin`, the same blob this image embeds.
- If the device does come up empty, use `--recover` (typed on the device).

**First boot.**
1. The red vendor screen needs a tap and a 1 s wait.
2. The device boots to the homescreen `ironwood-test`.
3. Each host connection asks for THP pairing.

**Other.**
- The 1-hour iwdg runtime limit applies.
- Rollback: flash `firmware-T3W1-2.12.6-7864a22444.bin` (same vendor, seed
  kept).

## 8. Checks run

| Check | Result |
|---|---|
| Zcash device tests, `-d -e` app, demo-branch frozen `PYOPT=0` emulator | **57 passed** in 143 s (`/tmp/extapp-p4/dt-all.log`) |
| The same on the `d485d458c5` emulator (`REDUCE_RAM`), with the `-e --features debug` app | **57 passed** in 35 s (`dt-all-safe7.log`) |
| SDK `make sdk_fmt_check sdk_check sdk_clippy sdk_test sdk_doctest` | exit 0: 4 unit + 40 (1 ignored) + 5 doc tests; only the existing warnings |
| `cargo clippy -p zcash --all-features`; `-p ironwood --all-features --all-targets`; rustfmt; ruff on the tests and the script | clean; the only warnings are the 2 existing ones in generated code |
| `clang-format` (repo config, Homebrew 22.1.1) on the changed C | clean |
| `cargo test -p ironwood --features test --release` | **126 passed** (§8a) |
| Hardware | **not run**: no flash, no device session |

**Not run:** reviews (Fable, Sol or clarity) of these six commits; pyright;
`make style_check`.

### 8a. ironwood suite

`cargo test -p ironwood --features test --release` at `1c0a782e01` (with the
classifier split): **126 passed, 0 failed**, exit 0, finished 18:39.

| Test | Passed |
|---|---|
| lib | 6 |
| conformance | 53 |
| digest_equivalence | 9 |
| hedged_nonce | 5 |
| receive | 13 |
| request_disposal | 3 |
| seed_fingerprint | 3 |
| session_equivalence | 13 (288 s) |
| stream_equivalence | 8 |
| transparent_outputs | 13 |

Log: `/tmp/extapp-p4/ironwood-tests.log`.

## 9. Decisions for the coordinator and the user

1. **Flash the `d485d458c5` image?** It carries the kernel ML-DSA workaround
   (§4). The alternative is no extapp on this Safe 7 until Trezor ships a
   secmon with the smcall.
2. **Session order.** First `--image diagnostic`, which answers the heap and
   watchdog questions; then `--image release`.
3. **If the 32-action sign stops** with "Timeout waiting for message", the
   `DIAG` line of the step before it and the device timings decide between:
   - hooks in the orchard fork;
   - `to_ivk` prewarm;
   - a Core-side limit change.
4. **Reviews.** Six new commits have had no review.

## 10. Scratch

`/tmp/extapp-p4/` is not a repo and does not survive a reboot. It holds:
- the build scripts `build-emu.sh`, `build-hw.sh`, `stage_host.sh`;
- `run_emu.py` and `restart_emu.sh` (ports 21391/21397);
- `emu_presser.py`;
- the logs: `fw-*.log`, `zcash-*.log`, `dt-*.log`, `session-emu-*.log`,
  `sdk-checks.log`, `ironwood-tests.log`;
- `mldsa-su/` (the stack analysis, `worst.py`);
- `emu-bundle/`, `app/` and `out-apps/`.

The emulator and the Tropic model were stopped; nothing listens on 21391,
21392 or 21397.
