# Extapp P2a: SDK fixes and the Zcash app (receive and viewing key)

Date: 2026-09-25. Phase P2a of `EXTAPP_PLAN.md` (track B, part 1). Worktree
`/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-app`, branch `zcash/extapp-app`,
on `bieleluk/sdk-wip` `4cd93ff4d8` plus the two P0 fixes (`15de3664a6`, `1f71e245e4`).
Nothing has been pushed. This report is not committed.

**Result.** The Zcash app loads in the T3W1 emulator. It shows and returns Unified Addresses and
exports Orchard-only UFVKs that match the firmware series' fixture and upstream `orchard`.
**All 14 device tests pass.** The app gets its keys from a test-only stub (`dev-test-seed`), so
the tests prove the app's half only. The real proof needs track A's key service. The T3W1
hardware image is **150,016 B** after the review (code 51,584 + data 98,432): 38.2 % of the
384 KiB T3W1 arena and 63.4 % of the 231 KiB Safe 5 WIP arena. The review changes are in §13;
§§2–12 describe the state before the review.

## 1. Commits (oldest first, after the review rewrite; all `[no changelog]`, scopes from `docs/git/hooks/commit-msg`)

| Commit | Subject |
|---|---|
| `7c6288c374` | fix(core): back the extapp SDK allocator with the loader's app heap |
| `0a2896f577` | fix(core): make the extapp SDK progress API safe to use |
| `0cf2f7d907` | fix(core): show the Ethereum extapp's data progress before reporting it |
| `0f11bbb082` | feat(core): add the ironwood crate for the Zcash extapp |
| `3b3abc23b6` | feat(core): derive Zcash receivers and viewing keys from a spending key |
| `76d6f5d642` | fix(tools): let xtask modular build an app that is its own workspace |
| `91adf53f95` | feat(tools): pass extra app features to xtask modular build |
| `595f7644f8` | fix(core): accept extapp path patterns with different coin types |
| `9b10fd2bfa` | feat(core): add a Zcash extapp with receive and viewing-key export (with its device tests) |

- The SDK, xtask and app commits use `core` or `tools`, because the hook allows no `sdk` scope.
  Upstream's own WIP uses `feat(sdk)`/`feat(extapp)`.
- The git identity is unchanged, and no commit has an AI trailer.
- The pre-review series was 11 commits (`df552aa9f2`..`4830a3cfa8`). §13 lists what the review
  changed. Sections 2–12 describe the pre-review state wherever §13 says otherwise.
- **Review status:**
  - the Opus clarity review is applied (§13);
  - the adversarial GPT-6 Sol review is pending;
  - no Fable review has run.

## 2. SDK fixes

### a. Heap (`df552aa9f2`)

`applet_main` used to hand the global allocator a fixed 16 KiB `.bss` array. It now passes the
region from `app_get_heap`:
- on hardware, exactly `heap-size` rounded up for alignment (`stm32u5/app_loader.c`
  `fit_in_memory`);
- on the emulator, the rest of the 64 MiB arena (`unix/app_loader.c`).

An app with a zero heap does not initialize the allocator, so every allocation fails. A failing
`app_get_heap` exits with an error.

`low_level_api::app_get_heap` now returns `(ptr, len)` instead of a `&'static [u8]` that was
then written through.

The Ethereum and Tron samples declared `heap-size = 1024` and relied on the static, so both now
declare 16384, the heap they actually had. The `development.md` heap warning is replaced.

**Measured effect on the Ethereum T3W1 release image:**

| | Before (P0) | After |
|---|---|---|
| Data segment | 49.1 KB (RW 32.1 KB, stack 16 KB, heap 1 KB) | 48.1 KB (RW 16.1 KB, stack 16 KB, heap 16 KB) |
| Code segment | 120.6 KB | 121.0 KB (+0.4 KB is the progress init) |

**Caveat:** the emulator still gives every app the whole arena, so the emulator cannot catch a
`heap-size` that is too small. The development guide says so.

### b. Progress and the 1 s watchdog (`2705e15583`; sample fix `88dd0db25b`)

**`ui.rs` tracks whether Core shows a progress screen** (`PROGRESS_SHOWN`), mirroring
`progress_obj` in `run.py`:
- `update_progress` without a screen returns `DataError("Progress not initialized")` locally;
  before, Core killed the app.
- `end_progress` without a screen is a no-op.
- `wire_respond_raw` ends a screen the app left open, because Core drops it when the request
  ends.

**New `ui::Progress` guard:**
- `Progress::show(title, description, indeterminate)`;
- `report(value)` on the 0..1000 scale;
- `keep_alive()` re-reports at most every `KEEP_ALIVE_MS = 250`, and otherwise costs one
  `systick_ms` read;
- the screen ends on `Drop`.

`sdk/doc/development.md` gains a "Long computations" section. Core stops an app that sends no
IPC for 1 s while it handles a request. Waiting on Core (a screen, a host round trip) does not
count. An app must report from inside the long loop, and the emulator is too fast to reveal a
missing report. The watchdog itself is unchanged.

**Ethereum sample:** `get_progress_indicator` now calls `init_progress` first. The T3W1
emulator run of `tests/test_signtx.py` (log `/tmp/extapp-p2a/eth-signtx.log`) gave:
- 32 passed, 204 failed, 3 errors;
- **0 "Progress not initialized" and 0 "Timeout waiting for message"** (P0 had 45 and 34);
- Core processed 74 progress inits and 104 reports without stopping the app.

The failures are harness drift, not app bugs:
- 158 × translation key `words__cancel_and_exit` (known from P0);
- 9 × ImportError `compact_size` from `trezorlib.testing.common`;
- 4 × assertions, 3 × timeouts, and 1 × missing `EthereumTxRequest` in the generated messages.

## 3. The `ironwood` crate (`fd62bbe758`, `f9cbf8c2a5`)

### Import

The crate was copied **unchanged** from `zcash/ironwood-upstream-v2` at
`7864a22444932d6504da55df538b468cedff7db3` (`core/embed/ironwood`) into
`sdk/apps/zcash/ironwood`.

**Why it is not in the `sdk/apps` workspace.** It is the second member of a new
`sdk/apps/zcash` workspace:
- `zcash_transparent`'s `bip32` pins the pre-release `digest =0.11.0-pre.9`;
- the SDK's `test` feature, which Ethereum and Tron enable, needs `sha3 0.11`, and so the stable
  `digest ^0.11`;
- Cargo allows only one `digest 0.11.x`.

A second blocker: Ethereum's build script pins `serde =1.0.145`, which rules out every
`serde_json` that `pczt`/`serde_with` accept.

**Pins.** The workspace carries:
- the series' `[workspace.dependencies]` pins;
- `[patch.crates-io]` for `bawolf/orchard` `d4792911986deda16cc3d689a97f316655f08e84` and
  `bawolf/sinsemilla` `6ca88ff52835899c87ff83489b95c5cda887e8a4`;
- the `sdk/apps` profiles.

At the import commit, every package in `sdk/apps/zcash/Cargo.lock` had the same version and
source as in the series' `core/embed/Cargo.lock` (29 packages were pinned down with
`cargo update --precise`). The app commit then added its own new packages: prost, the SDK, rkyv,
ufmt, and others.

**Builds checked:**
- `cargo build -p ironwood --target thumbv8m.main-none-eabihf -Zbuild-std=core,alloc
  --profile release-fw`, with and without `computed-generators`;
- host `cargo test -p ironwood --features test --test receive --test seed_fingerprint`: 11 + 3
  passed at import, 13 after the change below.

**Four tests cannot build here:** `rooted_tier*` and `scratch_tier` include the firmware's
`core/embed/rust/src/ironwood/arena.rs` by `#[path]`, which does not exist in this tree.
- They are behind `required-features = ["test", "computed-generators"]`, so they are skipped by
  default.
- The same `#[path]` makes `cargo fmt -p ironwood` fail, so the changed files were formatted
  with `rustfmt` directly.
- These are MicroPython-region traces, moot in the app model. Delete them or port them to the
  SDK heap.

### Change 1: spending-key entry points (`f9cbf8c2a5`)

The app receives `sk`, not the seed, so the crate gains
`receive::derive_external_receiver_from_spending_key(sk, index, progress)` and
`derive_full_viewing_key_from_spending_key(sk, out, progress)`.
- **Validation** is the same as `orchard::SpendingKey::from_bytes`: `ask ≠ 0` and valid external
  and internal ivk.
- **Tests:** they match the seed functions for mainnet and testnet at all six diversifier
  indices. The key comes from upstream-style `SpendingKey::from_zip32_seed`.
- **The seed functions are unchanged**; they pass a no-op callback.

### Change 2: progress hook

`commit_ivk` now calls `progress()` after each of its 51 Sinsemilla chunks:
- 153 calls per receiver (external ivk ×2 plus internal ivk);
- 102 calls per FVK.

This is the only way to report from inside the Pallas work. The receiver path computes the
external ivk twice (once to validate, once to use it); that costs one extra `commit_ivk`, kept
for simplicity.

## 4. xtask changes (`a4c920cc89`, `acb2b106f3`, `4b0a20948f`)

- **App with its own workspace.** `run_cmd` changes into `sdk/apps/<project>` when that
  directory holds its own workspace root (`helpers::is_own_workspace`, via `cargo metadata`).
  So `xtask modular build|device-tests -p zcash` works from `core/embed` as the recipe
  describes.
- **Project directories.** device-tests, py-style and translation-style now use the package's
  manifest directory, not `<root>/<project>`.
- **Proof tool lookup.** Proof generation finds `extapp_tool.py` in the nearest ancestor. It no
  longer assumes the tool is exactly two levels up, and it still skips with a warning outside a
  firmware checkout.
- **Extra features.** `--features a,b` adds app features to the ones xtask derives.
  `resolve_features` now borrows from `self`. There is a new unit test, and 36 unit + 5 doc tests
  pass.

## 5. Coin types in `run.py` (`78d0344b56`): **overlaps track A**

`run.py`'s `_extract_slip44_id` refused, on **every** message, an app whose path patterns carry
different coin types. The agreed manifest (`m/32'/133'/account'` and `m/32'/1'/account'`)
therefore could not run at all.
- Each pattern is now parsed with its own coin type.
- The address MAC is refused when the coin types differ.
- Track A's in-progress `run.py` (worktree `extapp-keys`, uncommitted) does not handle this yet.
  **Track A should adopt or replace this commit.** It touches the same file.

## 6. The Zcash app (`cd68b87780`)

### Layout

`sdk/apps/zcash`:

| Path | Contents |
|---|---|
| `Cargo.toml` | package + workspace |
| `build.rs` | protobuf, the `tr!` translations and the macOS dylib link, from Ethereum; English only |
| `protob/{messages,zcash}.proto` | messages |
| `protob/{pb2py,messages.py.mako}` | copied from Ethereum |
| `translations/en.json` | the series' strings |
| `src/main.rs` | entry and dispatch |
| `src/account.rs` | validation, labels, `account_keys`, warnings, `with_progress` |
| `src/get_address.rs`, `src/get_viewing_key.rs` | handlers |
| `src/unified.rs` | ZIP-316 encoding through `zcash_address` 0.13 |

`zcash_address` 0.13 was already in the dependency graph, which avoids a hand port of
`f4jumble`/bech32m. The tests check its output against the series' fixture.

### Manifest

| Field | Value | Note |
|---|---|---|
| `id` | `zcash.trezor.com` | placeholder for Trezor to decide |
| `vendor` | `Ironwood (experimental)` | placeholder |
| `curves` | `["zip32-orchard"]` | |
| `paths` | `m/32'/133'/account'`, `m/32'/1'/account'` | `account` is the SDK/PathSchema keyword for a hardened account index |
| `stack-size` | 32768 | twice the samples; **not measured** |
| `heap-size` | 49152 | from the host peak in §8 |
| `app-ring` | 0 | not 2; see below |

**`app-ring` is 0, not 2.** The emulator rejected the dev-signed ring-1/2 root packet: EBADMSG at
`app_root.c:88` in `app_root_update`, from `root_packet_verify`. Ethereum and Tron also use
ring 0. This needs a platform follow-up before a third-party ring.

### Protobuf

The file is the series' `messages-zcash.proto` with one change: the request fields the series
marks `required` are `optional`, with "required:" comments. prost does not enforce proto2
`required`, so an absent network or account would silently decode as 0. The app now rejects an
absent field as `Malformed Zcash request`. The wire encoding is identical.

The app-local `MessageType` ids are 0..7, in the order GetAddress, Address, GetViewingKey,
ViewingKey, SignPczt, PcztRequest, PcztAck, SpendAuthSignatures. `ZcashSignPczt` is answered with
the Failure `"ZcashSignPczt is not supported yet"`, which keeps the app alive.

### Keys

`account::account_keys(network, account)` is the single source of key material. It returns
`AccountKeys { spending_key, seed_fingerprint, weak_backup }`; the spending key is zeroized on
drop.
- **Default build:** returns `DataError("Zcash account keys are not available")`. The SDK call is
  not on this branch.
- **`dev-test-seed` (test only, never default):** hard-codes the BIP-39 seed of "abandon … about"
  with an empty passphrase (`5eb00bbd…9e38e4`, checked with PBKDF2). It runs ZIP-32 Orchard
  master and hardened-child derivation with BLAKE2b, takes the fingerprint from
  `ironwood::seed_fingerprint`, and sets `weak_backup = true` (12 words).
- **At merge:** replace the default arm with `trezor_app_sdk::crypto::get_zip32_orchard_account(
  network.coin_type(), account)`, which matches track A's current SDK diff, and delete the stub
  and the feature.

### Errors

The series' messages are kept, but everything is a DataError. The SDK has no ProcessError
variant, so a policy violation is DataError, not ProcessError. Malformed input or an unknown
message id still terminates the app, as in Ethereum, because `wire_handler!` propagates decode
errors. Only the checks in the handlers send a Failure.

## 7. Screens compared with the series

| Step | Series (core, Eckhart) | App (SDK) | Difference |
|---|---|---|---|
| Weak backup | `show_warning(br_name="zcash_weak_backup", Warning)`: "Important", "Continue anyway", danger | `ShowWarning` with the same title, text, button and `danger=true`, `allow_cancel=false`, `br_name` "zcash_weak_backup" | SDK `show_warning` discards the UI result, so a cancel (which cannot happen without a cancel button) would not abort. The app fetches the keys **before** this warning; the series showed it before `get_seed` |
| Progress | `progress(indeterminate=True)`, `report(0)` | `Progress::show(None, None, true)`; `keep_alive` from the 153/102 Sinsemilla ticks | None visible. On the emulator no keep-alive fired (22 inits, 22 stops, 0 reports), because the work is too fast. **On hardware it is unmeasured** |
| Address | `show_address(address, network="Mainnet" as description, account "ZEC #1", path "m/32'/133'/0'", case_sensitive=False, br_name "zcash_receive", Address, chunkify)` | `ShowAddress` (title "Receive", `subtitle="Mainnet"`, account, path, `case_sensitive=false`, chunkify, Address) | SDK has no description field (subtitle used) and a fixed ButtonRequest name `show_address`, not `zcash_receive`. No "continue in app" screen after it |
| Viewing key consent | `confirm_action("zcash_export_viewing_key", "Export Zcash viewing key?", action "Zcash Mainnet\nZEC #1\nm/32'/133'/0'", description warning, SignTx, prompt_screen=True, hold=True)` | `ConfirmAction` with the same title, action, description, br_name and SignTx, `hold=true`, `cancel=false`, `external_menu=false` | The SDK bridge hard-codes `prompt_screen=false`, so there is no separate hold prompt screen. On Eckhart `press_yes` confirmed it in the tests |
| Seed fingerprint | `show_warning("zcash_seed_fingerprint")` after consent | same, same order | none |
| Session checks | `helpers.require_session` between screens | none | not applicable in the extapp model; core owns the session |

## 8. Tests and results

### Files

`sdk/apps/zcash/tests/`:
- `conftest.py`: a reduced copy of Ethereum's; it refuses `--ui`, which has no fixtures;
- `zcash_ext.py`: the `call_ext`/`call_raw` shim plus the bech32m, `convert_bits` and `f4jumble`
  ported from the series' `trezorlib/zcash.py`;
- `common.py`: the vectors, `screen_text` and `address_pieces`;
- `test_get_address.py`, `test_get_viewing_key.py`.

### Vectors

On the ABANDON mnemonic, upstream crates.io `orchard` 0.15.5 + `zcash_address` 0.13 +
`zip32` 0.2.1 (scratch program `/tmp/extapp-p2a/vector`) reproduce the series' fixture exactly:
- the mainnet FVK `4c9c066f…49f883e`;
- the seed fingerprint `21ed3d78…d27815` (vector "2_actions").

The series has no address vector, so the addresses and the testnet FVK come from upstream only.
For example, mainnet account 0 index 0 is `u1y2z9wqt9du4stq2keex78l4vvl…vxjtk5g`, the address
the project docs already record.

### Run

The emulator was started on private ports (UDP 21361, Tropic model TCP 21367) with the scratch
launcher `/tmp/extapp-p2a/run_emu.py`, not the default ports.

Command (from `core/embed`):
```sh
source /tmp/extapp-p2a/env.sh
xtask modular build -p zcash -m t3w1 --lang en -d -e --features dev-test-seed
TREZOR_PATH=udp:127.0.0.1:21361 UV_PROJECT_ENVIRONMENT=/tmp/extapp-p2a/zcash-venv \
  xtask modular device-tests -p zcash -m t3w1 -e
```

Result: **14 passed in 108.5 s** (log `/tmp/extapp-p2a/dt-all.log`).

| Test | Checks |
|---|---|
| `test_receive_address` ×4 (mainnet a0 i0, a0 i1, a1 i0; testnet a0 i0) | exact UA, ButtonRequests `[(Warning, zcash_weak_backup), (Address, show_address)]`, address on screen in order, subtitle equals the network |
| `test_receive_address_chunkify` | same address, same ButtonRequests, chunked lines are multiples of 4 and narrower |
| `test_refuses_bad_requests_before_any_screen` ×5 | a missing network, account or index, a 10-byte index, and account 2³¹ are refused with the series' messages |
| `test_refuses_an_unknown_network` | raw-encoded `network=2` gives the policy violation |
| `test_device_fvk_matches_fixture` (mainnet, testnet) | ButtonRequests; the UFVK's Orchard item equals the FVK; no fingerprint; the path is on the consent screen |
| `test_seed_fingerprint_export_is_opt_in` | the extra warning, in order, mentions "recovery seed"; the fingerprint equals the fixture |

**What these tests do not show:**
- that coreapp derives the same `sk`, fingerprint and weak-backup bit; that needs track A;
- the hardware timing and keep-alive behaviour;
- whether the heap or stack is enough on the device. The emulator heap is unbounded.

Python style: `uvx ruff format`/`ruff check` are clean, and so is flake8 apart from the generated
file. `xtask py-style` expects `ruff`/`flake8`/`pyright` on PATH (nix), which this environment
lacks.

## 9. Sizes: T3W1 hardware release

The measured build is `xtask modular build -p zcash -m t3w1 --lang en --features dev-test-seed`
(`release-fw`, opt-level z, fat LTO).

**Why the stub build is measured.** The default build has no key source, and LTO then removes
all the Pallas code: code 16.7 KB. The stub adds only BLAKE2b, which `ironwood` links anyway,
and a 64 B constant. The real key service call replaces it with one IPC.

**Image** `target/artifacts/t3w1/zcash.elf`: 52,224 B, the 512 B header plus `code_size` 51,712.

| Segment | Size |
|---|---|
| `.text` | 47,666 B |
| `.rodata` | 3,292 B |
| relocations | 0.6 KB |
| `.bss` | 16,488 B, almost all the SDK's 16 KiB IPC inbox |

xtask reports "Code 50.4 KB, Data 96.1 KB (32.0 KB stack, 48.0 KB heap)".

**Header `data_size`:** 98,432 B = RW 16.1 KB + stack 32 KiB + heap 48 KiB.

| | Bytes | Share of T3W1 384 KiB (393,216 B) | Share of Safe 5 WIP 231 KiB (236,544 B) |
|---|---|---|---|
| This app (receive and viewing key) | 150,144 | **38.2 %** | **63.5 %** |
| Ethereum sample, for comparison (after the heap fix) | 121.0 KB + 48.1 KB | ~44 % | ~73 % |

### Heap

The host measurement (`/tmp/extapp-p2a/heap`, a counting allocator around the same `ironwood`
entry points and `zcash_address` encoding):

| Operation | Peak |
|---|---|
| First receiver + UA | 38,010 B |
| Later receivers + UA | 215 B |
| Later FVK + UFVK | 446 B |

**29,802 B of the first-use peak stays allocated for the rest of the app's life.** It is
`pasta_curves`' lazy `SqrtTables`, enabled through feature unification. The rest is transient.
Hence `heap-size` 48 KiB. The measurement is on a 64-bit host; the 32-bit target should need
about the same or less.

**Signing will need about 90 KiB.** With the same code and stack, the data segment then grows to
about 140 KB and the image to about 190–200 KB plus the signing code: about half the T3W1 arena,
and tight for 231 KiB.

The SqrtTables cost (about 30 KB of RAM) is worth checking: is `sqrt-table` really needed?

### Stack

32 KiB, not measured. It needs a stack-painting or `-Z emit-stack-sizes` pass on hardware.

## 10. Findings for track A and the platform

1. **`run.py` coin types**: §5. Without it, the agreed manifest cannot run.
2. **Ring 1/2 root packets are rejected by the emulator** (EBADMSG, `app_root.c:88`), so the app
   uses ring 0.
3. **Key validation semantics.** ZIP-32 in `orchard` and in the series validates the key at every
   child level (`ExtendedSpendingKey::validate`). Coreapp can only do BLAKE2b, so it cannot. The
   app validates the final `sk` as `orchard` does. For the rare seed where an intermediate level
   is invalid (probability about 2⁻²⁵⁰), the result differs from the series' error. Record this
   as an accepted difference.
4. **ZIP-315 order.** The weak-backup bit arrives with the keys, so the app reads the keys before
   showing the warning. Nothing is exported before consent.
5. **SDK `show_warning` returns no result** (`ipc_ui_call_void`). If the user could cancel, the
   app would carry on.
6. **SDK `ShowAddress`** has no `br_name` or description, and the ConfirmAction bridge hard-codes
   `prompt_screen=false`. See §7.
7. **SDK `test` feature**: its mock `sha3 0.11` blocks sharing `sdk/apps` with the Zcash crates,
   and so blocks host unit tests of the app itself. The app has no `test` feature; its logic is
   covered by the `ironwood` host tests and the device tests.

## 11. What remains for signing (P2b)

- **Handler.** Add `ZcashSignPczt`: stream the PCZT through `ZcashPcztRequest`/`ZcashPcztAck`,
  using `wire_request` round trips (`WireContinue`), into `ironwood::Session`.
  - Review screens: the approval rules and the recipient-address rule `dcc602fd48` from
    `docs/common/zcash-ironwood-signing.md`.
  - Then `approve`, then `sign` with `ask` derived from `sk`, returning
    `ZcashSpendAuthSignatures`.
  - The 16 KiB IPC inbox bounds chunk size; the series uses 1 KiB chunks, which fit.
- **Heap.** Size it for signing (about 90 KiB). Decide the `computed-generators` question:
  - the table Sinsemilla costs flash and RO (arena RAM);
  - computed generators cost time, which makes the 1 s keep-alive more important.
  - Keep-alive hooks are needed in the signing core too (per action, and inside RedPallas and
    `hash_to_curve`-heavy steps), not only in `commit_ivk`.
- **Crate cleanup.** Fix `prewarm()`'s relinking of FF1, `num_bigint` and `libm` (the plan's
  warm-up regression), and remove or port the four arena-trace tests.
- **Key service.** Swap `account_keys` to the SDK call once track A merges, delete the
  `dev-test-seed` stub, and rerun these device tests without it.
- **Hardware.** Measure stack, heap and timing (keep-alive cadence) on a Safe 7, or on the Safe 5
  alpha arena.
- **Reviews.** Run the adversarial (Fable) and readability reviews of this branch.

## 12. Scratch environment

These paths are not in any repo and will not survive a reboot.

- `/tmp/extapp-p2a/env.sh`: the P0 environment, with `xtask` pointed at this worktree
  (`/tmp/extapp-p2a/bin/xtask`).
- `core/tools/uv.lock` (gitignored) was copied from the P0 worktree so that the `UV_FROZEN=1`
  proof step can run.
- Logs:
  - `fw-build*.log`: emulator firmware; the last run finished with exit 0, 68 s incremental;
  - `zcash-emu-build.log`, `zcash-hw-build.log`, `zcash-hw-build.nokeys.log`, `eth-hw-build.log`;
  - `dt-all.log`, `eth-signtx.log`;
  - `emu.log`.
- The emulator was stopped afterwards: no `firmware-emu` or `model_server` is left on ports
  21361/21367.

## 13. Review resolutions (clarity review, 2026-09-25)

**Review:** `copenhagen/.context/product-scaffold/whole-diff-review/CLARITY-REVIEW-P2A-APP.md`.
The reviewer model was `claude-opus-5-5[1m]` (Opus 5.5, **not** Fable). It raised 16 should-fix
items and 34 nits. The adversarial GPT-6 Sol review follows separately.

**History.** The history was rewritten with fixup and `amend!` commits and a scripted rebase
(`git rebase -i` with a prepared todo). The final tree is byte-identical to the pre-rebase fixup
HEAD.

**Per-commit build check.** For every commit in a scratch worktree:
- `cargo check` of the SDK, modular-xtask and the Ethereum app all pass;
- `cargo check -p ironwood --features test --all-targets` passes at the two ironwood commits;
- the Zcash workspace passes at the app commit.

### Should-fix items

| # | Resolution |
|---|---|
| A1 | The Ethereum progress fix is now commit 3, right after the SDK progress change. |
| A2 | The py-style/translation-style fix is squashed into the xtask workspace commit, whose message now names all three lookups. |
| A3 | Removed `rooted_tier*`, `scratch_tier`, `device_arena/` (which `#[path]`-include the missing `arena.rs`) and `region_budget.rs` (a model of the firmware's MicroPython-era arenas), plus their `[[test]]` stanzas. The import commit says so. `cargo fmt` and `cargo clippy --all-features --all-targets` are now clean for the whole Zcash workspace. |
| A4 | **Not applied, per the coordinator:** the whole crate stays, because signing lands next. The commit message says the app uses `receive` now and the signing core next. |
| A5 | Dropped `links` and `build.rs`. Neutralised the firmware-context comments with minimal edits: the Cargo feature and version notes, the `prewarm` module doc (heap, not MicroPython GC or `install_region`), "the region"→"the heap" in `session.rs`/`stream.rs`, `hedge.rs` (no `sign_pczt.py`), `error.rs`, the `receive` module doc, "stack analysis", the test helper doc, and "firmware"→"device" in `lib.rs`. The design document is cited as `docs/common/zcash-ironwood-signing.md` of bawolf/trezor-firmware@7864a22444, "not in this tree"; the § references stay. |
| B1 | `wire_error_raw` now also ends a leftover progress screen, through a shared `end_request_progress`. Its failure is ignored, so the original error is still reported. The commit message and the guide say "a response or error". |
| B2 | `Progress::show(description, title, indeterminate)` now matches `init_progress`. |
| D1 | No more swallowed `raise`. The address MAC takes its coin type from the validated path (`paths.unharden(address_n[1])`), so `slip44_id` is gone. The commit message is updated. |
| E1 | ButtonRequest codes come from `protob/common.proto` (copied from Tron) via `ButtonRequestType::…​.into()`. |
| E2 | Screens (`show_warning`, `show_weak_backup_warning`, `with_progress`) moved to `src/layout.rs`. The stub moved to `src/dev_test_seed.rs`, one file to delete later. |
| E3 | Comments no longer name the absent SDK call. The curve comment says "a coreapp operation that is not available yet; the app makes no Crypto calls today". `account_keys` says the same. |
| E4 | Vendor changed to `Bryant Wolf` (**the user should confirm, or pick another**; the `id` `zcash.trezor.com` is also a placeholder). The signing-heap and ring-2 narration is gone from the manifest; the ring-2 issue stays recorded here, in §10.2. |
| E5 | The PCZT messages, the `allow(dead_code)` and the `ZcashSignPczt` arm are gone. Ids 0..3 remain. |
| E6 | The proto header now states only the `optional`/"required:" rule. The PCZT and SLIP-39 prose is dropped. |
| F1 | The refusal tests run under a `no_screens` input flow that fails on any ButtonRequest (renamed `…_before_any_screen` for the unknown network too). A throw-away sanity test confirmed that `no_screens` fails on a real address flow: "unexpected screen zcash_weak_backup". |
| F2 | `MNEMONIC` now says that under `dev-test-seed` the app derives from its own copy of the test seed and ignores the device seed, so the tests cover the app's half only. The app commit message says the same. |

### Nits applied

| Area | Change |
|---|---|
| A6 | No "firmware series" in the messages, the proto or the tests; the source is named by commit. |
| A7 | Commit message updated. |
| A8 | The device tests are squashed into the app commit. |
| B3 | `#[must_use]` on `Progress`. |
| B4, B5 | Comments reworded. |
| B6 | The guide's paragraph is split into a list and says that `keep_alive` repeats the last value. |
| B7 | The heap arm is now `Ok((_, 0))`. |
| C1 | `sdk/apps/zcash` example dropped. |
| C2 | A single `set_current_dir` via `own_workspace`. |
| C3 | The translation-style message is fixed. |
| C4 | Shorter TODO. |
| D2, D3 | The comment is generic, and schemas are built in one comprehension. |
| E7 | The app uses `ironwood::receive::Network`, whose `coin_type` is now `pub`. |
| E8 | The stub says it mirrors `ExtendedSpendingKey`, without per-level validation. |
| E9, E10 | `uformat!` (from `strutil.rs`, as in Ethereum). |
| E11 | `Zeroizing<[u8; 96]>`. |
| E12 | `with_progress(derive, error)`. |
| E13 | Keys are dropped right after use in both handlers. |
| E14 | Argument markers added. |
| E15 | Blank lines fixed. |
| E16 | Comment on why there is no `test` feature. |
| E17 | `zcash_address` is a workspace pin. |
| E18 | A short `README.md` with the two xtask commands. |
| F3 | Provenance wording. |
| F4 | Docstring. |
| F5 | `orchard_fvk` helpers cut to the inverse-only, private forms. |
| F6 | `AddressFlow` NamedTuple. |
| F7 | `pytest.param` ids. |
| F8 | Exact path asserted per network. |
| G1 | `validate` returns the external ivk, so the receiver path makes **2 × 51** Sinsemilla ticks, down from 3 × 51: one commitment fewer. The test is updated. |
| G2 | The exact call counts are removed from the docs. |

**Not applied.** A4, as above. Items that the review said disappear with A4/E5 are handled
through E5 and the A5 edits. `postbuild.rs`'s pre-existing `rustfmt` diff is upstream code and
was left alone.

### Reruns after the rewrite

All on HEAD `9b10fd2bfa`, with the emulator on private ports UDP 21361 and Tropic TCP 21367. The
emulator was stopped afterwards.

| Check | Result |
|---|---|
| SDK | `cargo fmt --check` clean. `cargo check` and `clippy` show no new warnings (the existing missing-docs set only). `cargo test --features test`: 2 unit tests + 40 + 5 doctests pass (1 ignored). |
| modular-xtask | build and clippy clean; 36 unit tests + 5 doctests pass. |
| ironwood | `cargo fmt`/`clippy --all-features --all-targets` clean. Full `cargo test -p ironwood --features test --release`: **126 passed, 0 failed** (lib 6, conformance 53, digest_equivalence 9, hedged_nonce 5, receive 13, request_disposal 3, seed_fingerprint 3, session_equivalence 13 in 602 s, stream_equivalence 8, transparent_outputs 13). |
| Zcash device tests (`xtask modular device-tests -p zcash -m t3w1 -e`, app built with `--features dev-test-seed`) | **14 passed** in 110 s. |
| Ethereum subset (P0's set: sign/verify message, public key, typed data) | **36 passed**. |
| Ethereum `test_signtx.py` | 33 passed, 206 failed (harness drift: 160 × `words__cancel_and_exit`, 9 × `compact_size` import, 4 assertions, and others). **0 "Progress not initialized", 0 "Timeout waiting for message"**; Core processed 53 progress inits, 106 reports and 9 stops. |
| Size (T3W1 release, `--features dev-test-seed`) | code_size **51,584 B** (.text 47,588, .rodata 3,252, relocations 0.6 KB) + data_size **98,432 B** (RW 16.1 KB, stack 32 KiB, heap 48 KiB) = **150,016 B**: 38.2 % of the 384 KiB T3W1 arena and 63.4 % of the 231 KiB Safe 5 WIP arena. |
