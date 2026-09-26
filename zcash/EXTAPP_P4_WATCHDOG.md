# Extapp P4: SignPczt "Timeout waiting for message" on the Safe 7

Date: 2026-09-25. Worktree
`/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-demo`. Nothing
has been pushed. This report is not committed. The model was Opus 5.5
(`claude-opus-5-5[1m]`) working alone, with no subagents and no reviews.

## Result

- **Most likely cause: a stack overflow, not the watchdog.** The app's
  deepest call chain needed **40,004 B** of stack (release image: 39,812 B).
  The declared stack is 32,768 B. The chain is `sign_pczt` (a 15,792 B frame),
  then `Body::action` (15,232 B), then `Spend::parse` (3,176 B). It runs on
  every action, so the first action overflows.
  - The applet runs with `PSPLIM`, so the overflow is a `STKOF` UsageFault.
    The kernel kills the task (`systask.c`).
  - `run.py` does not watch for a killed task while it waits. It reports the
    missing message as "Timeout waiting for message" 1 s later.
  - Receive and the viewing key need only ~6.3 and ~7.1 KB, so they worked.
  - The emulator runs apps with a large host stack, so all 57 tests passed.
  - Confidence: high but not observed. Rust allocates the whole frame at
    function entry, so the arithmetic is deterministic. No fault was seen on
    the device, because nothing reports one.
- **The watchdog alone would not have fired.** The Safe 7's own figures for
  this app calibrate the host-to-device ratio at ≈410–440×. On that ratio the
  longest silent stretch before the fix was ≈0.60 s, so the longest silence
  was ≈0.7 s (§2). That is under 1 s, but above the ~0.5 s target.
- **Fixes.** Four commits on the new branch `zcash/extapp-signstart` (§4),
  plus one commit on a new orchard fork branch:
  1. Stack: heavy code moves out of `sign_pczt`'s frame. The worst case is
     now **28,564 B** (release image: 28,460 B), 4.2 KB under the limit.
  2. Stepping, the Trezor way: progress hooks go between the prewarm's calls,
     inside the orchard nullifier check (fork) and between the encryption
     recoveries. The redundant second note commitment of the prewarm is
     removed.
     - Longest step: ≈0.27 s on the device.
     - **Expected worst silence: ≈0.37 s.** The key check before any screen
       stays at the measured 285 ms.
  3. Diagnostics that survive a failure: a debug build sends its counters
     with every PCZT chunk request.
  4. Core (optional firmware): a killed app fails with "Task stopped", not
     "Timeout".
- **No tables in the image; Core's 1 s limit is untouched.** Code shrank by
  1,632 B (release) and 1,376 B (diagnostic).
- **Checks.**
  - Zcash device tests: **58 passed** (57 + 1 new), twice: on the
    `0245f15fd0` emulator and on the new `07a27d1d76` emulator (UDP 21411).
  - ironwood: **126 passed**. SDK checks: exit 0. Clippy: clean.
  - The session script ran against the emulator: all MATCH.
- **Bundle:**
  `copenhagen/.context/product-scaffold/session-images/extapp-07a27d1d76/`.
  - The apps run on the firmware already flashed (`0245f15fd0`).
  - The firmware in the bundle is optional.
- **Hardware: not run.**

## 1. Evidence and what it rules out

**The session log** (`session-logs/2026-09-26-safe7-extapp-session.log`):
- Load: OK.
- Receive: MATCH. Longest silence 285 ms, heap peak 40,076 B.
- Viewing key: MATCH, 259 ms.
- `sign_2_actions` ran in the same app instance. It failed 4 s after the
  weak-backup ButtonRequest.
- `sign_32_actions` ran on a fresh instance. It failed 3 s after the
  ButtonRequest.
- The user took ~3 s per warning elsewhere in the log. So both failures came
  ≈1 s after the tap, early in the sign.

**Static stack analysis.**
- Tool: `/tmp/extapp-p4wd/stack.py`.
- Method: `objdump -d` of the raw app ELF. Each frame is the
  `push`/`stmdb`/`vpush` plus `sub sp`. The call graph uses `bl`/`b.w`.
  Indirect calls are not followed; they are the progress closure, ≈0.5 KB, and
  not on the deepest path.

| Image | Worst from `applet_main` | Deepest path (frames, B) |
|---|---|---|
| Before, release | 39,812 | 288 > 112 > 168 > **15,784** `sign_pczt` > **15,232** `Body::action` > 3,176 `Spend::parse` > … |
| Before, diagnostic | 40,004 | 296 > 200 > 256 > **15,792** > 15,232 > 3,176 > … |
| After, release | 28,460 | 288 > 112 > 2,408 `handle_sign_pczt` > 2,192 `feed` > 15,232 > 3,176 > … |
| After, diagnostic | 28,564 | 296 > 200 > 2,408 > 2,200 > 15,232 > 3,176 > … |
| Receive / viewing key | 6,292–6,340 / 7,132–7,140 | — |

**What inflated `sign_pczt`'s frame.** Fat LTO with `opt-level = "z"`
inlined all of the following, and their stack slots were not shared:
- the prewarm and key setup;
- `payment_address`, with its F4Jumble and Bech32 decode;
- the output, memo and totals screens;
- signing, with a 2,088 B `Signatures` value;
- the whole session feed.

The largest types involved are small (`Session` 208 B, `Event` 344 B, host
figures). The frame came from how many were inlined, not from any one of them.

**Core cannot tell a crash from a silence.**
- A killed task sets `running = false` and signals `SYSHANDLE_APP_ARENA`
  (`app_arena.c` `on_task_killed`).
- `run.py` waits only on `IPC2_EVENT`. It checks `is_running()` only at the
  top of its loop.
- Commit `07a27d1d76` checks it on timeout too.

## 2. Timing: offenders, calibration, before and after

**Harness:** `/tmp/extapp-p4wd/timing`.
- It runs the app's exact call sequence, on the host, with the app's profile:
  `opt-level = "z"`, fat LTO, one codegen unit.
- It uses the same pins, and the series' 32- and 2-action vectors.
- Each measurement is one fresh process, 12–15 runs. The machine was loaded
  (load average 5–12), so the minimum and median are quoted.

**Calibration.**
- **Safe 7, this app:** 285 ms and 259 ms. These are the longest silences of
  receive (cold) and the viewing key (warm). In both requests the longest
  silent stretch is `SpendingKey::from_bytes`, between the crypto reply and
  the warning screen.
  - On the host that call takes 685–862 µs cold and 586–767 µs warm, so the
    ratio is **≈410–440×**.
  - This is an upper bound on the ratio: the silence is at least the call.
- **Safe 5, Core, computed Sinsemilla:** 576-bit `hash_to_point` in 372 ms
  and 510-bit in 328 ms (`SINSEMILLA-DEVICE-TIMING.md`). The same calls on
  the host take 1,410–1,485 µs and 1,151–1,280 µs, so the ratio is
  **≈250–285×**.
- The estimates below use **440×**.

### Before the fix (host µs, min/median; device ≈ ×440)

| Silent stretch (between two IPC messages) | Host | Device |
|---|---|---|
| Prewarm part 3 + the account FVK: `fvk.address` 328 + `Note::from_parts` 457 + a redundant `note.commitment()` 409 + `FullViewingKey::from` 179 (no hook after the prewarm's last part) | **1,373 / 1,437** | **≈0.60–0.63 s** |
| Prewarm part 2: `SpendingKey::from_bytes([1;32])` 586 + FVK 180 + `scope_classifier` 404 | **1,170 / 1,223** | **≈0.51–0.54 s** |
| Nullifier check (one orchard call: the spent note's commitment, the ownership check, the nullifier) | 920–960 (external); ≈+175 for a change note | ≈0.41 s; ≈0.49 s for change |
| Encryption check (up to four recoveries) | 690–770 | ≈0.30–0.34 s |
| `SpendingKey::from_bytes` in `account_keys`, before any screen | 587–702 | **285 ms measured** |
| Per-action parse, cv_net, rk, note commitment | ≤480 | ≤0.21 s |
| Sign: `ask` + one signature | ≈555 | ≈0.24 s |

- The longest silence is at most 100 ms (keep-alive throttle) plus the
  longest stretch: **≈0.70–0.73 s**.
- For this to cross 1 s, the ratio would have to be ≥≈650×. The device's own
  259/285 ms rule that out.
- No single library call exceeds ≈0.42 s on the device. The largest are the
  nullifier check and `from_bytes`.

### After the fix (`timing flow`; median of each run's worst, 15 runs)

| Step | Host (min / median of run max) | Device |
|---|---|---|
| `account_keys` `from_bytes` | 640–699 | 285 ms (measured; unchanged) |
| Longest prewarm step (`from_bytes([1;32])`) | 551–608 | ≈0.26 s |
| Prewarm tail + FVK | 170–187 | ≈0.08 s |
| Longest feed step (32 actions / 2 actions) | 532–616 / 469–501 | ≈0.23–0.27 s |
| Sign (`ask` + signature) | 535–575 | ≈0.25 s |
| Begin (to the first chunk request) | 3–4 | — |

- **Expected longest silence on the device: ≈0.37 s.** That is 100 ms plus
  0.27 s. At the Safe 5 ratio it would be ≈0.26 s.
- A few single host runs had a 0.97–1.35 ms worst step. That is scheduler
  noise on the loaded machine, the same as P2b's 20 ms outliers.

## 3. The fix, and the options not taken

**Stepping, as Core does it: progress between steps.**
- **`ironwood::prewarm_with_progress`** reports after every call:
  `from_bytes`, FVK, classifier, address and note. The second
  `note.commitment()` is gone: `Note::from_parts` already computes the
  commitment, which fills `note_commit_domain`.
- **orchard fork.**
  - Repo: `/Users/bryantwolf/workspace/orchard`, branch
    `ironwood/nullifier-progress`, commit
    `e23177a9dd1c35ee35b5034fc237c4d56de5fabf`. It is `d479291` (the pin
    until now) plus one commit.
  - `Spend::verify_nullifier_with_progress` calls `progress` after the spent
    note's commitment and after the ownership check.
    `verify_nullifier_with_classifier` passes a no-op. The CHANGELOG has an
    entry.
  - **Not pushed; the remote to push to is `bawolf`.**
  - For local builds the commit was pushed into cargo's git cache
    (`~/.cargo/git/db/orchard-56a27abc94c59be9`, ref
    `local/ironwood-nullifier-progress`). Once pushed to `bawolf`, a clean
    checkout builds.
  - The fork's own lib tests do not compile at `d479291`: they need a
    Sinsemilla counters API that is fixed on a later fork commit. So only
    `cargo build`, `clippy` and `fmt` were run there. The ironwood suites
    exercise the new function through the pin.
- **`ironwood::verify_encryption`** takes `progress` and calls it before each
  recovery after the first. The Engine passes a no-op.
- **Pasta and Sinsemilla are unchanged.** No single remaining call needs a
  hook inside them.

**Options not taken.**
- **Tables in the image.** Excluded by the user. The Safe 5 arena is already
  over budget.
- **Prewarm at app start.**
  - `run.py` watches only inside a request, so that phase is not watchdogged.
  - But no progress screen can be shown then. The first request's WireStart
    would also meet a busy app, and Core's 1 s timer starts at that send.
  - So the prewarm would have to finish before the host's first request.
    That is not guaranteed; the session script sends immediately after the
    load.
- **Raising Core's 1 s limit.** Trezor's call; documented, not done.
  - It would remove the need for the orchard hook, but not for the stack fix.
  - A narrower Core alternative: keep the 1 s limit, but let an app declare a
    longer bound per request.
- **A bigger stack instead of the refactor.** 48 KiB would also have covered
  the old 40 KB chain, at +16 KiB of arena, and the Safe 5 is already over.
  The refactor costs nothing, and it removes 11 KB from the deepest chain for
  both models.

## 4. Commits

Branch `zcash/extapp-signstart` is `zcash/extapp-safe7-debug` (`0245f15fd0`)
plus four commits.
- The commits are local. The identity is unchanged, there are no trailers,
  and every commit is `[no changelog]`.
- All four cherry-pick cleanly, in order, onto `zcash/extapp-demo`
  (`1c0a782e01`). This was tried in a temporary worktree, which was then
  removed.

| Commit | Subject | Files |
|---|---|---|
| `e7300b1bf0` | fix(core): keep the Zcash extapp's SignPczt within its 32 KiB stack | `sign_pczt.rs` (`prepare`, `sign`, `feed`, `confirm_payment` out of line; `confirm_transparent_output`, `confirm_totals` `#[inline(never)]`), `Cargo.toml` (stack comment) |
| `fab2c7f77e` | feat(core): report progress inside ironwood's prewarm, nullifier and encryption checks | `ironwood/src/{prewarm,session,lib}.rs`, `Cargo.toml`/`Cargo.lock` (orchard `e23177a9dd`) |
| `9b4743420b` | feat(core): send a debug Zcash app's counters with each PCZT chunk request | SDK `diagnostics::snapshot`, `zcash.proto` (`ZcashPcztRequest.diagnostics = 4`, optional), `main.rs`, `sign_pczt.rs`, shim `on_diagnostics`, test `test_chunk_requests_carry_diagnostics` |
| `07a27d1d76` | fix(core): report an extapp the kernel stopped as stopped, not as a timeout | `core/src/apps/extapp/run.py` |

## 5. Diagnostics that survive a failure (item 3)

**Why the counters were lost.** When Core stops an app, whether for a silence
or a fault, the app's counters die with it. The session script also reloaded
the app after every failure without asking for them.

**Now:**
- **Per chunk.** A debug build puts `diagnostics::snapshot()` into every
  `ZcashPcztRequest`: heap size, used and peak, longest silence and its
  service, and the message count. This does not start new counters.
- **Release builds** send nothing; the field is optional.
- **The shim** passes the counters to `on_diagnostics`.
- **The session script:**
  - prints `DIAG: sign_<name> chunk request: …` whenever the silence or the
    peak grew;
  - asks for `ZcashGetDiagnostics` after a failed step too, before
    reloading, and says when the app is gone;
  - folds the chunk values into the SUMMARY.
- The script's copy before this change is at
  `/tmp/extapp-p4wd/safe7-extapp-session.py.orig`.

**Emulator run of the script** (`/tmp/extapp-p4wd/session-emu-diag.log`):
- All MATCH.
- Chunk lines: heap peak 38,208 → 51,456 → 57,600 B.
- The 32-action sign: 138 IPC messages; longest silence 17 ms (emulator).

**Not exercised:** the script's path for a failure where the app is still
running.

## 6. Sizes

| Image | code_size | data_size | Arena share (384 KiB) | Before |
|---|---|---|---|---|
| `zcash.elf` | 187,456 | 123,040 | 310,496 B, 79.0 % | 312,128 B, 79.4 % |
| `zcash-diag.elf` | 190,464 | 123,072 | 313,536 B, 79.7 % | 314,912 B, 80.1 % |

- **Safe 5 (231 KiB, 236,544 B):** still does not fit (131 %). This change
  makes it neither better nor worse, apart from −1.6 KB of code.
- **Stack, heap, RW:** unchanged, and no new allocations. The emulator heap
  peak is still 57,600 B.
- **Optional firmware:** 2,403,840 B, as for `0245f15fd0`; FLASH 2347.5 KB
  (70.37 %).
- **Build reproducibility:** the app images rebuilt from the clean
  `07a27d1d76` are byte-identical to those built from the pre-commit tree.

## 7. Bundle and next hardware run

**Bundle:**
`/Users/bryantwolf/conductor/workspaces/trezor-ironwood/copenhagen/.context/product-scaffold/session-images/extapp-07a27d1d76/`
- `README.md` and `SHA256SUMS`, checked with the script's own checker.
- App images: `zcash.elf` `b20eac04…`, `zcash-diag.elf` `ea50b39b…`.
- One dev tree and a root packet with timestamp 1790397782, newer than the
  1790385137 the device stores.
- The host shim.
- Optional firmware: sha256 `a65b6ad4…`, header fingerprint
  `000f5428dd8524bf…c8905e`, `verify()` OK. Vendor header, bootloader, BLE
  and secmon are identical to `0245f15fd0`.

**Run:** `safe7-extapp-session.py --image diagnostic`, then `--image release`.
No flash is needed. The session script picks this bundle as the newest.

**What it should show:**
- Load OK; receive and the viewing key MATCH, with ≈285/259 ms as before.
- **Both signs MATCH.**
- `DIAG … chunk request` lines, with the silence rising to **≈0.3–0.4 s**.
- A final longest IPC silence of ≈0.3–0.4 s, or the 285 ms of the key check
  if that turns out longer.
- A heap peak under 73,744 B: the emulator's 57,600 B, or a little less on
  32-bit.
- **The real host-to-device ratio.** Receive's 285 ms and the sign's longest
  step should keep the same ratio of about 1 : 0.9.

**If a sign still fails,** the last chunk line tells how far it got.
- **Timeout with small silences** means a crash, not a slow step. Flash the
  optional firmware: the message then reads "Task stopped".
- In that case, look next at the heap (fragmentation under the real
  73,744 B, never tested because the emulator gives the whole arena) and at
  the RNG syscall. The sign is the first hardware use of
  `SYSCALL_RNG_FILL_BUFFER` from an applet.

## 8. Checks run

| Check | Result |
|---|---|
| Baseline app rebuild at `0245f15fd0` | byte-identical to the bundle (`83370f6c…`, `a6e81787…`); stack 39,812 / 40,004 B |
| Zcash device tests, `0245f15fd0` emulator (`firmware-emu-fix`), app `-e --features debug`, UDP 21411 / Tropic 21417 | **58 passed**, 36 s (`/tmp/extapp-p4wd/dt-all.log`) |
| The same on the new `07a27d1d76` frozen PYOPT=0 emulator | **58 passed**, 35 s (`dt-all-signstart.log`) |
| `cargo test -p ironwood --features test --release` | **126 passed**, 0 failed; `session_equivalence` 308 s (`ironwood-tests.log`) |
| `make sdk_fmt_check sdk_check sdk_clippy sdk_test sdk_doctest` | exit 0; 6 + 40 (1 ignored) + 5 doc tests; only the existing missing-docs warnings |
| `cargo clippy -p zcash --all-features`; `-p ironwood --all-features --all-targets` | only the 2 existing warnings in generated code; ironwood clean |
| rustfmt; ruff 0.15.10 format/check (tests, shim, `run.py`, script at line length 100) | clean |
| orchard fork: `cargo build --release`, `clippy`, `fmt --check` | clean; lib tests not buildable at this base (pre-existing) |
| Session script `--emulator --image diagnostic` against the emulator bundle `/tmp/extapp-p4wd/emu-bundle` | all MATCH, chunk DIAG lines printed |
| Hardware firmware and apps from the clean `07a27d1d76` | exit 0; `verify()` OK; frozen `run.py` has the new check |
| **Not run** | hardware; the "Task stopped" path (the emulator cannot fault-kill an app); Fable/Sol and clarity reviews of the four commits and the orchard commit; pyright; `make style_check` |

## 9. Scratch

`/tmp/extapp-p4wd/` is not a repo and does not survive a reboot. It holds:
- `timing/`: the harness. Modes `seq`, `flow`, `prims` and `sizes`;
  `--features computed` for the Safe 5 calibration. Result files `r32-*`,
  `r2-*`, `flow*-*`, `prims-*`.
- `stack.py`.
- `build-apps.sh` and `build-all.sh`.
- `start_emu.sh` and `run_emu.py`.
- The app builds: `app-base`, `app-exp1`, `app-exp2`, `app-fix1` and
  `app-final`.
- `fw-hw/` and `firmware-emu-signstart`.
- The logs.

The emulator and the Tropic model were stopped; nothing listens on 21411 or
21417. The worktree is left on `zcash/extapp-signstart` and is clean. The
orchard checkout is unchanged: it is still on `ironwood/direct-scope-classifier`,
and the new branch exists only locally.
