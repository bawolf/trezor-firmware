# Extapp P2b: key service in the Zcash app, and streamed PCZT signing

Date: 2026-09-25. Phase P2b of `EXTAPP_PLAN.md`. Worktree
`/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-demo`, branch `zcash/extapp-demo`
(started at `zcash/extapp-keys` `91fc95c0ac`). Nothing has been pushed. This report is not
committed.

**Result.** The Zcash app now gets its keys from Core's ZIP-32 Orchard key service (the
`dev-test-seed` stub is gone) and signs streamed Ironwood PCZTs. In the T3W1 emulator, **all 56
Zcash device tests pass**: 17 receive and viewing-key tests, and 39 signing tests ported from the
series' `test_sign_pczt.py` with its fixtures. The emulator cannot show hardware timing, the 1 s
watchdog on a Cortex-M33, or whether the declared heap is enough; those are estimates below.

## 1. Commits (oldest first; identity Bryant Wolf, no trailers, all `[no changelog]`)

| Commit | Subject |
|---|---|
| `5fd51d24a3` | Merge branch 'zcash/extapp-app' into zcash/extapp-demo |
| `7a4c7a80b3` | feat(core): give extapps the device random number generator |
| `9135dc81bd` | feat(core): get the Zcash extapp's keys from Core's key service |
| `e0b8d12350` | feat(core): report progress from inside ironwood's session and prewarm |
| `a7db99df0b` | feat(core): sign streamed Ironwood PCZTs in the Zcash extapp |

**Merge.** The only conflict was `core/src/apps/extapp/run.py`. It keeps the key-service
branch's hunks: the per-pattern coin types, and address MACs refused for an app whose patterns
name several coin types. The app branch's MAC fix merged cleanly on top: the coin type passed to
`get_address_mac`/`check_address_mac` is `paths.unharden(address_n[1])` of the validated path.
Both branches' tests pass on the result: the key-service unit tests (§4) and the app's device
tests.

## 2. What was built

### Key service in the app (`9135dc81bd`)

- `account_keys()` calls `trezor_app_sdk::crypto::get_zip32_orchard_account(coin_type, account)`
  and validates `sk` with `orchard::keys::SpendingKey::from_bytes`. An invalid key is refused
  with "Zcash key derivation failed".
- `AccountKeys` holds the validated `SpendingKey` and wipes it on drop. It uses a volatile
  `wipe`, since the orchard types have no `Zeroize`. The SDK's `Zip32OrchardAccount` is dropped
  (and zeroed) right after.
- Removed: the `dev-test-seed` feature, `src/dev_test_seed.rs` with its seed copy, and the
  `blake2b_simd` app dependency.

### Device RNG for apps (`7a4c7a80b3`): a platform gap found here

`trezor_api_v1_t` has no RNG, and the applet syscall whitelist allows none. RedPallas nonces and
the session id need entropy.
- The table gains `rng_fill_buffer`, **appended after `trezor_crypto_v1`** so existing field
  offsets do not move.
- `syscall_dispatch.c` admits `SYSCALL_RNG_FILL_BUFFER` for applets. Its verifier already
  checks that the buffer is writable by the caller.
- The SDK gains `crypto::random_bytes`. `ffi.rs` is **byte-identical** to the bindgen output of
  the new header (`core/build-xtask/debug/trezor-api.rs`, diffed). The test mock fills zeros.

This is a proposal for Trezor, like the key service.

### Progress hooks in `ironwood` (`e0b8d12350`)

- New entry points: `Session::feed_with_progress`, `Session::sign_with_progress` and
  `prewarm_with_progress`.
- The callback runs between the expensive steps:
  - per action: parse, cv_net, nullifier (the first one also builds the ivk classifier), rk,
    note commitment, encryption;
  - per dummy signature at the review;
  - per signature;
  - between the three parts of the prewarm.
- `feed`, `sign` and `prewarm` pass a no-op and are otherwise unchanged.
- **No rule changed.** The full crate suite passes on the session change: 126/126, listed in §4.

### `SignPczt` (`a7db99df0b`)

**Files.**
- `src/sign_pczt.rs`: the handler.
- `src/layout.rs`: `confirm_output`, `confirm_memo`, `confirm_total`, `keeping_alive`.
- `src/unified.rs`: UA decode, and t-address encoding through `zcash_address`.
- The protobuf.
- The host shim.
- The tests and fixtures.

**Flow**, as in the series' `sign_pczt.py`:
1. Validate the request before any screen: network, account, `pczt_length` 1..65536, and host
   height. Missing fields give "Malformed Zcash request".
2. Get the keys (Core's consent on first use), then show the ZIP-315 warning.
3. Progress "Loading transaction...", `prewarm`, derive the FVK.
4. Start `Session` with policy fee ≤ 1,000,000 zat and expiry window 100.
5. Draw a random 16-byte transfer id.
6. Pull 1 KiB chunks. The session feeds them and returns events:
   - `ConfirmTransparentOutput`: one "zcash_transparent_payment" warning per transaction, then
     the Base58Check address;
   - `ConfirmOutput`: a non-payment is refused (the series' boundary check); then the
     recipient-address rule (below), the output, and a memo screen for text or hash;
   - `Review`.
7. Totals (`confirm_summary`), `approve`.
8. Progress "Signing transaction...": derive `ask`, `sign_with_progress`, wipe `ask`.
9. Return records `0x03 ‖ index ‖ sig64`.

**Recipient-address rule `_payment_address`**, ported exactly:
- No `user_address`: show the Orchard-only UA.
- Otherwise the string must be ASCII alphanumeric and no shorter than the Orchard-only UA, or
  it gets "Invalid unified address.".
- It must decode (`zcash_address`) for this network, or it gets "Invalid unified address.".
- Its Orchard receiver must equal the verified receiver, or it gets "Unified address does not
  match the Orchard receiver.".
- It is shown as given.

**Session lifecycle.**
- The series' native handle, static slot and blind cancel exist because MicroPython cannot own
  a Rust value. Here the handler owns the `Session` on its stack (its big state is boxed on the
  heap). Every exit path (error, cancel, return) drops it, and the session, records and token
  zeroize.
- The `FullViewingKey` and `SpendAuthorizingKey` are in a `Wiped` guard. `ask` exists only
  around `sign`.
- No scratch tier. The app heap is its own and dies with the app.
- Hedge: `Hedged::from_seed(DeviceRng, sk)`. The app never sees the seed, so the hedge secret is
  `BLAKE2b(0x00 ‖ sk)`. That is still a device-only secret, so `hedge.rs`'s argument holds.

**Transport.**
- `ZcashPcztRequest` goes out as an unfinished `ExtAppResponse` through `wire_request_raw`
  (WireContinue); the host answers with an `ExtAppMessage` carrying `ZcashPcztAck`.
- The app checks the transfer id, the offset and the exact length, or answers "Invalid PCZT
  chunk".
- No chunk timer (dropped by design).
- `wire_request` + `wire_request_type!` is **not** used. It decodes any reply as the expected
  type and discards the reply's message id, so a cancel could not be told from a chunk. Fixing
  the SDK's `wire_request` to check the response id is an SDK follow-up.

**Cancel.**
- The platform does not deliver trezorlib's `Cancel` to an app blocked in a WireContinue: Core
  aborts `run()`, and the app stays waiting. The next `ExtAppMessage` then arrives as a
  WireStart, the app's call fails, and that new request is lost.
- So the app adds `ZcashCancel` (id 8). In place of a chunk it ends the request with
  `Failure(ActionCancelled)`. Outside a request it gets the same answer, like Core's
  `handle_Cancel`.
- Any other reply id gives "Unexpected message".

**Errors.** The series' messages are kept, but all are `DataError`, because the SDK has no
ProcessError:
- Malformed → "Malformed PCZT";
- Capacity → "Too many transaction actions (max 32)";
- Amount → "Zcash amount out of range";
- everything else → "Zcash PCZT rejected" (a ProcessError in the series).

**Protobuf.**
- App ids: 4 SignPczt, 5 PcztRequest, 6 PcztAck, 7 SpendAuthSignatures, 8 Cancel.
- The series' request fields are `optional` with "required:" comments, and the app rejects an
  absent field (the P2a rule).

**Host shim.** `tests/zcash_ext.py` gains `sign_pczt`, mirroring the series' trezorlib client:
- the argument checks;
- the latched transfer id, contiguous offsets and exact lengths;
- cancel-and-`ProtocolError` on a violation, with `cancel()` sending `ZcashCancel` and requiring
  `ActionCancelled`, or marking the session invalid;
- `_parse_records`: pool `0x03`, strictly increasing indices, < 32 records.

The shim also checks each response's `finished` flag. It now carries the series' full
Bech32m/F4Jumble helpers, because the tests build UAs.

## 3. Screens compared with the series (T3W1, Eckhart)

| Step | Series (Core layouts) | App (SDK) | Difference |
|---|---|---|---|
| Account consent | none | Core's `zip32_orchard_account` "Zcash account" screen, first use per app instance, network and account | new, from P1 |
| Weak backup | `show_warning` before `get_seed` | same screen, after the keys arrive | order (P2a) |
| Loading/signing progress | `progress(TR.progress__…)`, determinate | `Progress::show` with the same strings, determinate; reported before each feed | none visible |
| Transparent warning | `show_warning("zcash_transparent_payment")` | same text, name and code | none |
| Output | `confirm_output`: address + amount `confirm_value`, "Recipient #n" subtitle, account-info menu, cancel | two `ConfirmValue`s through `interact_with_menu_flow`, same title, subtitle, chunkify, page counter and ButtonRequest (`confirm_output`, ConfirmOutput); menu = "Account info" (Wallet, Derivation path) + "Cancel sign?" | the amount screen has no back button (the SDK `ConfirmValue` cannot set it); no "Sign cancelled" continue-in-app screen after a cancel; the SDK's cancel confirmation sends ButtonRequest code 1 with no name |
| t-address | `coininfo` versions + trezor base58 | `zcash_address` `from_transparent_p2pkh/p2sh` | encoder only; checked against the fixture's independent encoder |
| Memo | `confirm_value("Recipient #n", …, "Memo (text)"/"Memo (hash)", "confirm_memo", ConfirmOutput)` | same, with a cancel menu | none |
| Totals | `confirm_total` → `confirm_summary`: total, "incl. Transaction fee", Account "Zcash Mainnet ZEC #1", path; Fee info: expiry, public amount, outputs, total actions | the same `ConfirmSummary`, `confirm_total`, SignTx | property mono flags are `plain` (the series passed `None`) |
| Session checks | `require_session` after each screen | none | Core owns the session |

## 4. Tests and results

### Device tests (T3W1 emulator, frozen `PYOPT=0`, UDP 21371, Tropic TCP 21377)

Run: `uv run pytest --app=…/t3w1-emu/zcash.elf tests/` in `sdk/apps/zcash`. Log
`/tmp/extapp-p2b/dt-all.log`: **56 passed in 200 s**.

**`test_sign_pczt.py`, 39 tests.**
- `test_streamed_sign`: the 7 vectors of `sign_pczt.json` pass. They are 2/8/16/32 actions,
  `stock_sdk_view_full`, `stock_sdk_view_sdk` and `zip32_derivation_own`. Each asserts the
  exact `real_spend_actions` and 64-byte signatures.
- `test_totals_screen`: new. The consent screen shows total and fee.
- The user_address tests pass:
  - `test_payment_shows_its_user_address`;
  - `test_long_user_address_is_shown_whole`, three receivers and 512-char at budget;
  - `test_user_address_for_another_receiver_is_refused`.
- `test_transparent_outputs`: 4 vectors, with the exact ButtonRequest sequence, t1/t3/tm/t2
  prefixes and chunked addresses.
- `test_memo_is_shown_and_signed`: 7 vectors (short, UTF-8, at budget, over budget, arbitrary,
  supplementary plane, bidi override).
- `test_zip32_derivation_not_the_device_own_is_rejected`: 2 failed fixtures, "Zcash PCZT
  rejected".
- `test_cancel_at_output` and `test_cancel_at_totals_then_sign`.
- New:
  - `test_two_signs_in_one_app_instance`;
  - 4 tampered-chunk tests (offset, transfer id, short, no data);
  - `test_host_cancel_mid_stream`, where `ZcashCancel` is followed by a successful sign;
  - 6 bad-request refusals before any screen.
- Records are checked against the fixtures as in the series. Signatures are not re-verified
  there either: the hedged nonce makes them non-deterministic, and the argument is the crate's
  equivalence suites.

**Receive and viewing key, 17 tests.** Every flow now expects Core's consent. New tests:
- the request names the account and network and is asked once per instance, network and
  account;
- declining gives Cancelled and asks again next time;
- **the "all ×12" mnemonic yields `u1uzslncc…rl26c`**, computed with crates.io `orchard` 0.15.5.

That last test and the unchanged "abandon" vectors prove that the keys come from the loaded
mnemonic through Core.

**First run.** 38/39. `2_transparent_outputs_testnet` failed on a test bug: the consent helper
expected "Mainnet". It was fixed and all 4 transparent vectors passed before the full run.

### Other suites

| Suite | Result |
|---|---|
| `cargo test -p ironwood --features test --release`, on the session hooks | **126 passed, 0 failed**: lib 6, conformance 53, digest_equivalence 9, hedged_nonce 5, receive 13, request_disposal 3, seed_fingerprint 3, session_equivalence 13, stream_equivalence 8, transparent_outputs 13; 36 min wall on a loaded machine |
| The same, after the prewarm hook | lib, receive, hedged_nonce, request_disposal, stream_equivalence: 35 passed (the full suite was not rerun) |
| `cargo clippy --all-features` (app) and `-p ironwood --all-features --all-targets`; `cargo fmt` | clean |
| SDK `make sdk_fmt_check sdk_check sdk_clippy sdk_test sdk_doctest` | exit 0; 4 unit + 40 (1 ignored) + 5 doc tests; only the existing missing-docs warnings |
| Ethereum subset (sign/verify message, public key, typed data), rebuilt on this SDK | **36 passed**, 73 s |
| Key-service core unit tests (`run_tests.sh`, non-frozen `build_unix`, Tropic model TCP 21387, UDP 21381) | `test_apps.extapp.zip32_orchard.py` **16 OK**, `test_storage.cache.py` **7 OK** |
| ruff format/check of `tests/` | clean |

**Not run:** pyright; the SDK `test` feature for the app (it still cannot share `digest` with
the Zcash crates); UI screenshot fixtures (the conftest refuses `--ui`); hardware.

## 5. Sizes, heap, watchdog and timings

### Heap (host, `/tmp/extapp-p2b/heap`)

The measurement is a counting allocator around the app's exact call sequence (`from_bytes`,
`prewarm`, FVK, `Session`, 1 KiB feeds with the two per-chunk copies the app makes, screen
strings and UA decode, approve, sign), over the series' `32_actions` vector.

| | Peak |
|---|---|
| First sign after load | **56,522 B**; 30,186 B of it the Pasta square-root table and orchard domain caches, which stay |
| Later signs | 26,336 B |
| 2-action vector | the same 56,522 / 26,336 B (the per-action state is O(1)) |

This compares with the series' scratch peak of 25,008 B plus a rooted region of ~40 KiB.
Receive peaks at 38,010 B (P2a).

**`heap-size` = 73,728 B (72 KiB).** That is 16 KiB over the peak, for
`embedded_alloc::LlffHeap` fragmentation (for example the square-root table built during an
earlier `GetAddress`) and 32- vs 64-bit differences. **The emulator gives every app the whole
arena**, so this size is not validated by any test.

### Sizes (T3W1 hardware release, `xtask modular build -p zcash -m t3w1 --lang en`)

| Build | code_size | data_size (RW + 32 KiB stack + 72 KiB heap) | Total | T3W1 384 KiB | Safe 5 WIP 231 KiB |
|---|---|---|---|---|---|
| default (table Sinsemilla) | 188,992 | 123,040 | **312,032** | **79.4 %** | 131.9 %, does not fit |
| `--features computed-generators` | 122,848 | 123,040 | **245,888** | **62.5 %** | **104.0 %, 9,344 B over** |

- P2a's receive-only image was 150,016 B.
- Levers to fit the Safe 5:
  - the stack, 32 KiB and unmeasured;
  - the SDK's hard-coded 16 KiB IPC inbox (1 KiB chunks need a few KiB);
  - heap margin.
- Computed generators are much slower: the series measured about 1.5 s per action on the
  Safe 5 in that configuration.

### Watchdog (Core stops an app with no IPC for 1 s)

The keep-alive runs through `keeping_alive`: `Progress::keep_alive` at every crate hook, which
reports if 250 ms have passed.

**Host gaps between hooks** (quiet machine, release build, 32-action vector, 223 hooks):

| | Gap |
|---|---|
| median | 0.40 ms |
| p90 | 0.86 ms |
| **worst** | **1.24–1.36 ms**, always the first action's nullifier step, which builds the ivk classifier (two Commit^ivk) |
| prewarm parts | ≈1.0 and 1.1 ms |
| `SpendingKey::from_bytes` in `account_keys`, before any progress screen | 0.5–0.8 ms |
| one signature | ≈0.33 ms |

The whole verification takes 99–111 ms on the host, or 3.1–3.5 ms per action. Earlier runs on a
loaded machine (load average ≈ 40) showed gaps up to 20 ms; those are scheduler noise.

**Scaling assumption (Cortex-M33, STM32U5 at 160 MHz on both the Safe 5 and the Safe 7): a
factor of 400–650×.** It comes from two sources:
- the project's Safe 5 figure of ≈1.39 s per real-spend action
  (`docs/decisions/2026-09-22-region-vs-heap.md`) against 3.5 ms on the host;
- the crate's "~1.4 s" prewarm against 2.1 ms on the host.

**Estimates on that assumption:**

| | Device estimate |
|---|---|
| worst hook gap | ≈0.5–0.9 s |
| each prewarm part | ≈0.45–0.7 s |
| `from_bytes` | ≈0.3–0.5 s |

The longest IPC silence is bounded by 250 ms plus the worst gap, so **≈0.75–1.15 s: marginal at
the upper estimate.** Before hardware, the cheap fixes are:
- report on every hook rather than every 250 ms (one progress IPC per hook);
- or lower `KEEP_ALIVE_MS` for this app.

Neither is applied yet. The pre-hook design (reporting only between feeds) would have been
seconds per silent call on the device.

### Timings (emulator, debug `-d` app build)

| Test | Wall time |
|---|---|
| 32-action sign | 2.6 s call |
| 16-action | 1.0 s |
| 8-action | 0.5 s |
| 2-action | 0.23 s |
| two consecutive 8-action signs | 3.3 s |
| full Zcash suite | 200 s |

These include the automated screen presses.

| Build | Time |
|---|---|
| emulator firmware, cold | 9 min (`fw-build.log`) |
| non-frozen `build_unix`, incremental | 44 s |
| app emulator build, incremental | ≈70 s |
| hardware release | 35 s |

## 6. What did not port, and why

- **The region test** (`test_two_signs_in_one_boot_leave_the_region_as_they_found_it`) has no
  arenas to measure: the app heap is its own and dies with the app. It is replaced by
  `test_two_signs_in_one_app_instance`. There is still no heap high-water instrument in the app.
- **The 5 s chunk timer** is dropped by design. An abandoned transfer holds the app until Core's
  workflow ends. Only the key service's session state bounds it; that should be checked with a
  host that walks away mid-stream.
- **trezorlib `Cancel`** is replaced by `ZcashCancel`, because of the platform gap in §2. The
  proper fix is in Core's `run.py` (forward an abort to a waiting app) plus the SDK.
- **ProcessError** does not exist: refusals are DataError, with the series' messages.
- **`require_session` and `seed.raise_if_not_initialized`** are the key service's
  responsibility now.
- **The FVK cross-check.** The series checked the exported viewing key against `orchard`. The
  app exports through the ironwood receive port and signs with `orchard`'s FVK, and they are not
  cross-checked at export. The fixture FVK equality in the device tests covers the test seed
  only. Follow-up.
- **`test_capabilities.py`** does not apply, because an extapp has no firmware capability bit.
  **UI screenshot fixtures** are not set up for extapps.

## 7. Follow-ups

1. On hardware: time per hook and prewarm, the IPC-gap bound, the stack high-water, and whether
   the heap fits.
2. The Safe 5 fit, from the levers in §5.
3. SDK: `wire_request` should check the response id. A back button on the output's amount
   screen. A named ButtonRequest for the menu cancel.
4. Platform: Cancel during WireContinue; RNG in the app API (proposal).
5. Reviews: the Fable adversarial pass and a readability pass over these four commits, which
   have had none; P1 and P2a re-reviews are still pending.

## 8. Scratch

These paths are in `/tmp/extapp-p2b/`, which is not a repo and does not survive a reboot:
- `env.sh`, `bin/xtask`, `run_emu.py`, `restart_emu.sh`;
- `heap/`, the measurement;
- `vector/`, the "all" mnemonic vector;
- logs: `fw-build.log`, `fw-build-unix.log`, `zcash-*-build.log`, `zcash-hw-*.log`,
  `dt-keys.log`, `dt-sign-all.log`, `dt-all.log`, `eth-subset.log`, `unit.log`,
  `ironwood-tests.log`, `sdk-checks.log`, `heap-32.txt`;
- `zcash-hw-table.elf` and `zcash-hw-computed.elf`.

The emulator, both Tropic models and all test processes were stopped afterwards; nothing
listens on 21371, 21377, 21381 or 21387. The worktree is clean. `core/build-xtask` now holds the
non-frozen unix build; rerun `build_unix_frozen` before more device tests.
