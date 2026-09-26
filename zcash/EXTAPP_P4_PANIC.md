# Extapp P4: the Safe 7 "unwrap failed" panic

Date: 2026-09-25. Worktree
`/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-demo`. Nothing
has been pushed. This report is not committed. The model was Opus 5.5
(`claude-opus-5-5[1m]`) working alone, with no subagents and no reviews.

## Result

- **Root cause (confidence: high).** `core/src/apps/extapp/run.py` imports
  `log` only under `if __debug__:` (lines 21–22), but calls
  `log.debug(...)` unguarded as the first line of `crypto_resp_cb` (line 142 at
  `d485d458c5`). The hardware image is frozen with `pyopt` (`PYOPT=1`), so
  `__debug__` is False, `log` is never bound, and the callback raises
  `NameError`.
  - The callback runs inside Rust's `send_crypto_result`, which wraps it in
    `unwrap!(cb.call_with_n_args(...))`
    (`core/embed/rust/src/crypto/api/firmware_micropython.rs:196`).
  - Core's `unwrap!` calls `system_exit_fatal("unwrap failed", file, line)`
    (`core/embed/rtl/src/error.rs:24-31`). The coreapp task dies, and the kernel
    shows the RSOD and reboots (`projects/kernel/main.c:340-350`).
  - It happens on the first crypto reply of any app, right after the user taps
    Allow. The emulator tests are `PYOPT=0`, where `log` exists, so they
    cannot see it.
- **Reproduced in the emulator, by fault injection.** A scratch build of
  `d485d458c5` whose `crypto_resp_cb` raises the same `NameError` ends the same
  way at the same step: `Task #1 terminated. Fatal: unwrap failed at
  rust/src/crypto/api/firmware_micropython.rs:196`, then a reboot. The host saw
  only the `zip32_orchard_account` screen, exactly as on the device.
- **Fixed** on the new local branch `zcash/extapp-safe7-debug`, in 3
  cherry-pickable commits (§4).
  - The same injected fault on the fixed build gives the host
    `DataError: Failed to serialize or send crypto result`, and Core stays up.
- **Test image with file:line.** The test image is built with
  `CARGO_PROFILE_RELEASE_PANIC=abort`, so plain Rust panics also report
  `file:line` on the red screen (§5).
- **Bundle** `copenhagen/.context/product-scaffold/session-images/extapp-0245f15fd0/`:
  - firmware 2,403,840 B, header fingerprint `d02c984a…cd4d5`,
    `verify()` OK;
  - the same app images, proofs and root packet. Rebuilt apps are
    byte-identical.
- **Checks.**
  - Zcash device tests: **57 passed** (emulator, UDP 21401).
  - SDK: `make sdk_fmt_check sdk_check sdk_clippy sdk_test sdk_doctest` exit 0,
    with 6 unit tests (2 new), 40 (+1 ignored) and 5 doc tests.
- **Hardware: not run.** Flashing and a session are the next step.

## 1. Evidence

**Session log** (`session-logs/2026-09-25-safe7-extapp-session.log`).
- 19:49:28: the receive step starts.
- 19:49:29: `STEP: [zip32_orchard_account]` (Core's consent).
- There is no `[zcash_weak_backup]` step, although the test seed is 12 words and
  the app shows that warning before anything else (`get_address.rs:26-28`).
  In the emulator the receive step always prints consent, weak-backup, then
  show_address.
- 19:49:38: `LIBUSB_ERROR_NO_DEVICE`, i.e. about 9 s of reading and tapping,
  then the crash.
- So Core died after Allow and before the app's first screen request reached
  the host. That is the reply to the crypto request.

**The "Zcash screen" the user saw** was most likely Core's own consent screen,
titled "Zcash account". It stays on the display after Allow until something
else draws. The app never received the key, so it cannot have drawn anything.

**The flashed code.** The frozen bytecode of that build
(`core/build-xtask/thumbv8m.main-none-eabihf/release/build/upymod-b45dfcfa89678a16/out/frozen_mpy.c`,
18:12, the `--apps` build, before this work overwrote it):
- `scope apps_extapp_run__lt_module_gt_` has **no** `IMPORT_FROM 'log'`. The
  `PYOPT=0` emulator build has `IMPORT_FROM 'log'` / `STORE_NAME 'log'`.
- `fun_data_apps_extapp_run_run_crypto_resp_cb` starts with
  `LOAD_GLOBAL 'log'; LOAD_METHOD 'debug'`.

**The new image.** No `apps/extapp/run.py` scope references `log`. `load.py`
still does, but it imports `log` at module level, so that is fine.

**Same bug class elsewhere.** An AST scan (`/tmp/extapp-p4panic/unguarded_log.py`)
of `core/src` found 13 unguarded calls, all in `run.py`, at `d485d458c5` lines
142, 171, 253, 287, 313, 340, 351, 385, 389, 514, 530, 533 and 539. The other
hits are false positives: local imports and `if __debug__ and …`.
- Line 171 runs after every app UI screen. Had the crypto reply survived, that
  line would have ended the request with a `NameError` Failure at the first
  tap.
- Lines 533 and 539 break `GetPublicKey` for every app on hardware.
- These calls are upstream code (`345b13da2b9`, Lukas Bielesch). The Ethereum
  sample would hit the same crash on hardware.

## 2. The path after the consent, checked site by site (`d485d458c5`)

| Where | What can stop Core | Verdict |
|---|---|---|
| `run.py:141-143` `crypto_resp_cb` | `NameError` under PYOPT=1 → Rust `unwrap!` | **The crash** |
| `crypto/api/firmware_micropython.rs:194-198` | `unwrap!` on a missing `ipc_cb`, on the callback's result, and on `bytes → Obj` (allocation) | Fatal on any callback exception (e.g. `ipc_send` failing) or on OOM. Fixed |
| same file `:264-268` | `unwrap!(to_bytes_in_with_alloc(...))` into 200 B buffers | **200 B is enough.** Archived sizes: ZIP-32 result 112 B, Xpub 112 B, Signature 112 B, a 65-byte PublicKey 180 B (largest). Now an error anyway |
| same file `:39-48, :62, :70, :204-237` | `unwrap!` on list/int allocation for app-sized `address_n`, `assert!` on the public key length | App-controlled size → OOM → fatal. Fixed |
| `ui/api/firmware_micropython.rs:1601-1603` | the same `unwrap!` on the UI reply callback | Fixed |
| same file `:1630` | `.unwrap()` on `to_bytes`: a plain panic; the release profile compiles it to an abort instruction (fault RSOD, no text) | Cannot fail (`TrezorUiResult` is 8 B); now an error |
| same file `:1266, :1288-1327` | `unwrap!` on `StrBuffer`/`List`/`Obj` allocation for every app string and list | App-controlled size → OOM → fatal. Fixed |
| same file `:1336` | `unwrap!(vec.push(...))` for more than `MAX_MENU_ITEMS` select-menu items | App-controlled → fatal. Fixed (ValueError) |
| same file `:1654` | `unwrap!(get_buffer)` for a progress request | Input is Core's own `bytes`; now `?` |
| Checked `rkyv` access in all three bridges | Validation errors were already mapped to `ValueError` (the crypto request to `False`) | Never fatal, before or after |
| App side: SDK `crypto.rs:39`, `ui.rs:49` (`unwrap!(rkyv::access(...))`) | The app's `unwrap!` ends **only the app task** (`systask_kill`, `sys/task/stm32/systask.c:344-371`; `app_arena` notifies Core, which stops with "Task stopped" or a timeout) | Cannot produce a Core RSOD |

## 3. Alignment (brief item 2)

- **Kernel IPC.** `IPC_DATA_ALIGNMENT` is `sizeof(size_t)` (`sys/ipc/ipc.c:35`).
  - On the device that is 4. The 12-byte item header puts data at 4 mod 8
    whenever the item start is 8-aligned.
  - On the 64-bit emulator it is 8, and data is 16-aligned.
  - This is a real hardware/emulator difference.
- **App inbox.** It is `&mut [usize]` (SDK `ipc.rs:189`), i.e. 4-aligned. The app
  reads Core's replies in place.
  - `Archived<TrezorCryptoResultRef>` and `Archived<TrezorUiResult>` both align
    to **4**, so they are readable at every 4-byte offset. There is no failure
    today.
  - It would break silently, on the device only, if a reply type gained a
    `u64`. It is now pinned by compile-time asserts against
    `DEVICE_IPC_ALIGNMENT = 4`.
- **Core side.**
  - Requests reach the bridges as `bytes(msg.data)`:
    `mod_trezorio_ipc_message_to_obj` → `mp_obj_new_bytes`, a GC copy
    (`upymod/modtrezorio/modtrezorio-poll.h:147-150`).
  - GC blocks are 16 B on the device (`MICROPY_BYTES_PER_GC_BLOCK`,
    `py/mpconfig.h:251`), and the pool is block-aligned (`gc.c:239`).
  - `Archived<TrezorCryptoEnum>` aligns to **8**, because of
    `SignTypedHash.chain_id: Option<u64>` (`structs.rs:854`); UI and progress
    requests align to 4. All are satisfied.
  - A misaligned buffer is refused with `ValueError`, never `unwrap`.
  - Core's own receive buffer is `uint32_t ipc_buffer[8192]`
    (`projects/firmware/src/stm32/main.c:74`).
- **Why no aligned copy.** The brief suggested copying into an aligned buffer
  before checked access. Core never sees an unaligned request, and a copy on the
  app's reply path would add an unwiped copy of the spending key. The static
  guarantee plus refusal-not-panic was preferred.
  - The new test places every request and reply at offsets 0–15. Requests are
    accepted exactly at their alignment and refused elsewhere; replies are read
    at every 4-byte offset.

## 4. Commits (local, `zcash/extapp-safe7-debug` = `zcash/extapp-safe7` + 3)

All three belong on `zcash/extapp-demo` as well. `git merge-tree` shows that
each applies cleanly there in order.

| Commit | Subject |
|---|---|
| `41b3044367` | fix(core): keep extapp run.py logging inside `__debug__` blocks. The 13 calls are wrapped. |
| `def377ad9a` | feat(sdk): validate UI and progress requests, and pin reply alignment. Adds `access_ui_request` and `access_progress_request` (re-exported in the non-app `ui` module), compile-time asserts that reply types align ≤ 4, and the tests `misaligned_requests_are_refused` and `replies_are_read_at_the_device_ipc_alignment`. |
| `0245f15fd0` | fix(core): raise extapp bridge failures instead of stopping Core. Every `unwrap!`/`unwrap()`/`assert!` in the three `app_loading` bridge functions becomes an error. The callback's exception propagates, `run.py` wraps `send_ui_result` like `send_crypto_result` (`die(DataError(...))`), and the bridges use the SDK validators. |

Identity unchanged, no trailers, all `[no changelog]`. The worktree is left on
`zcash/extapp-safe7-debug`, clean.

## 5. File:line on the red screen (brief item 5)

- **Core's `unwrap!`/`ensure!`/`fatal_error!` already carry the location.** They
  pass `file!()`/`line!()`, and `rsod_gui` prints `expr` + `\n` + `file:line`
  (`io/gfx/rsod.c:151-163`; `expr` and `file` are 64 B each). Eckhart's
  `ErrorScreen` label breaks lines at `\n`.
  - The device most likely showed a second line,
    `rust/src/crypto/api/firmware_micropython.rs:196`, under "unwrap failed".
    It was not recorded.
  - The bundle README now asks for a photo.
- **Plain Rust panics** (`.unwrap()`, indexing, `panic!`) did not. The release
  profile is `panic = "immediate-abort"` (`core/embed/Cargo.toml:35-36`), which
  compiles them to an abort instruction: a fault RSOD with no location.
  - The test image is built with `CARGO_PROFILE_RELEASE_PANIC=abort`. The panic
    handler (`sys/src/panic.rs`) is then linked in firmware and kernel (2
    symbols in each, 0 before), and a panic shows `<msg or "rs">\n<file>:<line>`.
  - Cost: FLASH 2347.5 KB (70.37 %), +33.0 KB. Kernel `.data/.bss/.stack`
    unchanged; kernel flash +3.5 KB.
  - This is a build setting, not a commit. It is recorded in `/tmp/extapp-p4panic/build-hw.sh`
    and the bundle README.
- **App-side fatal errors** (the app's own `unwrap!`/panic) still show nothing on
  the device. The kernel prints their `pminfo` only on the debug console, and
  Core reports "Task stopped" or a timeout. Surfacing them needs a kernel/`app`
  API for the task's postmortem info. Not done.

## 6. Other hardware differences checked (brief items 3–4)

- **Consent screen and approval cache.** The layout is plain Python
  `confirm_action`. `APP_EXTAPP_ZIP32_APPROVALS` (44 B) is at the same index in
  `cache_thp.py`/`cache_codec.py` on both targets, with nothing that depends on
  `__debug__`. It ran before the crash (the approval is written before
  `get_seed`).
- **Derivation.** `get_seed()` under THP reads only the session cache. Its
  `assert common_seed is not None` is stripped under PYOPT=1, but the seed was
  present (the session was created with it). BLAKE2b is the same C code. No
  TROPIC access on this path.
- **Kernel stack change (36 KiB).** It affects the kernel only. The crash was
  the coreapp task (`Task #1`).
- **MicroPython heap: an unrelated, real margin issue.**
  - The emulator runs with a **20 MB** heap (`trezorlib/_internal/emulator.py:376`,
    `heap_size="20M"`). The device has **257,276 B** (`_heap_start`
    0x201c1304 … `_heap_end` 0x20200000; the series image had 265,344).
  - Replaying the whole session on the `d485d458c5` emulator with a word-adjusted
    heap (`-X heapsize=<n>w`):

    | Heap | Result |
    |---|---|
    | 257,276 and 245,000 | all MATCH |
    | 235,000 | all MATCH |
    | 228,000 and below | **app load fails** with `MemoryError` (a contiguous ~11–14 KB allocation), a clean Failure |

  - The margin is about 10 % (PYOPT=0, 64-bit; indicative only). The device
    loaded the app in 6.9 s.

## 7. Checks run

| Check | Result |
|---|---|
| Repro: `d485d458c5` bridges + `run.py` with an injected `NameError` in `crypto_resp_cb`, frozen PYOPT=0 emulator, session script receive step | Core fatal `unwrap failed at rust/src/crypto/api/firmware_micropython.rs:196`, reboot; host saw only the consent screen (`/tmp/extapp-p4panic/emu-repro.log`, `session-repro.log`) |
| Same fault on the fixed tree | Each step `DataError: Failed to serialize or send crypto result`; app stopped cleanly; emulator alive (`session-fixinject.log`) |
| Zcash device tests, fixed frozen PYOPT=0 emulator (`firmware-emu-fix`), app rebuilt `-e --features debug`, UDP 21401 / Tropic 21407 | **57 passed** in 55.8 s (`/tmp/extapp-p4panic/dt-all.log`) |
| Session script `--emulator --image diagnostic`, fixed emulator, heap `257276w` | load OK; receive, viewing_key, sign_2 and sign_32 all MATCH (`session-fix-heap257k.log`) |
| SDK `make sdk_fmt_check sdk_check sdk_clippy sdk_test sdk_doctest` | exit 0; 6 unit (2 new) + 40 (1 ignored) + 5 doc tests; no new clippy warnings (`sdk-checks.log`) |
| `rustfmt` (core `rustfmt.toml`) on both bridge files; ruff 0.15.10 format and check on `run.py`; unguarded-log scan | clean; 0 unguarded calls |
| Emulator firmware build warnings | 91, same as the `d485d458c5` build; none in core |
| Hardware build `0245f15fd0` | exit 0; `verify()` OK. Vendor header, bootloader `44fc92ab`, BLE `fb25b6df` and released secmon `464fcb1d` present and identical to `d485d458c5`'s image. New frozen `run.py` has no `log` reference |
| Hardware app rebuild at `0245f15fd0` (release and diagnostic) | byte-identical to the bundle's (`83370f6c…`, `a6e81787…`) |
| **Not run** | hardware flash or session; a true PYOPT=1 end-to-end run (a PYOPT=1 emulator has no debuglink to confirm THP pairing and the consent); core Rust/Python unit suites; pyright; `make style_check`; Fable/Sol and clarity reviews of the three commits (required by AGENTS.md before any hand-off) |

## 8. Uncertain until the next hardware run

1. **Whether this was the only cause.** The line on the device's red screen
   was not captured. Everything else fits: the timing, the missing weak-backup
   request, "unwrap failed", and the deterministic PYOPT=1 bytecode.
2. **Whether the PYOPT=1 path is now clean end to end.** The scan and the
   bytecode say yes for `run.py`, but only a hardware session exercises every
   PYOPT=1 difference: stripped asserts, `if __debug__` paths in the layouts,
   and the load path.
3. **Watchdog.** The longest IPC silence on the Cortex-M33 against Core's 1 s
   limit; P4 estimated 0.45–0.8 s. The diagnostic image measures it.
4. **App heap** (73,728 B; emulator peak 57,600 B) and **Core heap** margin
   (§6). An app-load `MemoryError` would be a clean Failure, not an RSOD.
5. **Kernel ML-DSA stack** of the `d485d458c5` workaround. It already passed
   once on the device (the load verified the root packet).

## 9. Scratch

`/tmp/extapp-p4panic/` (not a repo; lost on reboot) holds:
- `run_emu.py` and `start_emu.sh` (emulator with a chosen heap, private ports
  21401/21407), and `try_heap.sh`;
- `build-hw.sh` and `build-apps.sh`;
- the emulator binaries: `firmware-emu-d485` (original), `-repro`, `-fix` and
  `-fixinject`;
- `unguarded_log.py` and `verify_fw.py`;
- the logs: `session-*.log`, `emu-*.log`, `dt-all.log`, `build-hw.log`,
  `build-apps.log` and `sdk-checks.log`.

The emulator and the Tropic model are stopped; nothing listens on 21401 or
21407.
