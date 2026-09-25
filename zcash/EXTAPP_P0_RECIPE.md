# Extapp P0: Trezor's app platform running its sample app in the T3W1 emulator

Date: 2026-09-25. Phase P0 of `EXTAPP_PLAN.md`. Host: macOS 15 (Darwin 24.6), Apple silicon.
Local worktree `/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp`, branch
`zcash/extapp`. Nothing has been pushed.

**Result: yes, the gate is met.** The T3W1 emulator was built with app loading. The SDK's
Ethereum app was built for it, loaded over the wire (`ExtAppLoad`: header, root packet, chunks)
with dev keys, and passed the SDK's own device tests:
- get address and get public key;
- **sign message** (9/9 `test_signmessage` vectors) and verify (8/8 plus the invalid case);
- **sign typed data** (EIP-712, 10/10 vectors plus bad-hash, cancel and show-more flows).

The expected signatures are fixed vectors for the `all all …` test seed. Transaction signing
(`test_signtx*`) does **not** pass yet, because of a bug in the upstream sample app (see
"What failed").

## 1. Branch choice

Upstream WIP branches were fetched on 2026-09-25. `main` is `3c4fdb4926`.

| Branch (upstream) | Head | Base on main | Contents |
|---|---|---|---|
| `bieleluk/extapp-orchestration` (PR #7947) | `ed79bf0f04` 09-25 | `9fb25562e1` | orchestration `8e071d4dc3`, then 8 `fixup!`s; `run.py` is a stub |
| `bieleluk/ipc-update` (PR #7957) | `c8534b4098` 09-24 | `ebd0468e20` | Rust IPC/sysevent bindings; no SDK |
| `bieleluk/extapp-device-tests` | `c6fffaaf95` 09-23 | `9fb25562e1` | orchestration, trezorlib `extapp.py`, `extapp_tool`, modular-xtask, `tests/device_tests/extapp/test_load.py`; **no SDK crate, no apps, stub `run.py`** |
| `bieleluk/refactor-extapp` | `9b847bd496` 09-18 | `9fb25562e1` | only the `trezorapp → extapp` rename |
| `bieleluk/modular-ethereum-wip` | `c5205c5544` 07-27 | `3f19318310` (745 behind) | the original SDK, Ethereum/Tron, `run.py`, T3T1 arena `adb18e1795` |
| `bieleluk/extapp-rust-sdk` | `c75ba3ada9` 2025-11 | 2385 behind | obsolete ELF-loader-era SDK |
| **`bieleluk/sdk-wip`** (not in the brief) | **`4cd93ff4d8`** 09-22 | **`9fb25562e1`** | **`modular-ethereum-wip` rebased onto current main**: see below |
| `cepetr/apptool` | `60d8999028` 09-25 | `9fb25562e1` | `sdk-wip` + 3 commits: `modular-xtask` → `trezor-app-tool`, `xtask apps` (marked "wip") |
| `vojczejk/sdk-wip-modui` | `06cf337807` 09-25 | `9fb25562e1` | `sdk-wip` + 10 commits: new "modui" UI library; **disables the Ethereum app** |
| `bieleluk/stabby` | `0be72a42dd` 09-21 | `62e7f18195` | older rebase; adds "declare IPC inbox size in manifest" and a Rust app entry point |

`bieleluk/sdk-wip` at `4cd93ff4d8` contains the following on top of `9fb25562e1`:
- orchestration `8e071d4dc3`;
- the header_hash → fingerprint rename;
- the `trezorapp → extapp` rename;
- the SDK crate `sdk/crates/trezor-app-sdk`;
- `modular-xtask`;
- the Ethereum and Tron apps;
- the coreapp UI/Crypto/Progress services (`core/src/apps/extapp/run.py` plus the Rust rkyv
  bridges);
- trezorlib `extapp.py` and `debuglink.load_extapp`;
- `extapp_tool.py`;
- the T3T1 arena WIP (`534e35daa9`);
- SDK docs (`sdk/doc/*.md`);
- `build-docker.sh`.

**Chosen: a single upstream branch, `bieleluk/sdk-wip` @ `4cd93ff4d8`, with no merges.** It is
the newest branch that has everything the gate needs, and it is documented.
- Both newer branches are strict descendants of it (`git merge-base --is-ancestor` checked).
  - `cepetr/apptool` only renames and reworks the build tool and is marked wip.
  - `sdk-wip-modui` disables the Ethereum sample.
  - Either can be fast-forwarded onto later.
- **Not included, and why:**
  - the 8 orchestration `fixup!`s after `8e071d4dc3` (`8fd5e4accd`..`ed79bf0f04`, 09-23..25);
  - `ipc-update`;
  - `extapp-device-tests`' `c6fffaaf95`, which is a load-flow test suite built on the stub-era
    tree.

  None of them is needed to load and run an app. They touch the same files as `sdk-wip`, so merging them is
  churn best done when upstream restacks.

**Local commits on `zcash/extapp`** (2 small commits on top of `4cd93ff4d8`):
- `15de3664a6` fix(sdk): build and pack emulator apps on macOS.
  - `sdk/apps/{ethereum,tron}/build.rs`: add `-dynamiclib`.
  - `sdk/crates/modular-xtask/src/binary.rs`: accept an aarch64 Mach-O and tag it
    `target_arch = 1`.
- `1f71e245e4` test(ethereum): `tests/definitions.py` builds `DefinitionPayload(magic=b"trzd", version=b"1")`,
  matching the current trezorlib.

## 2. Environment

Everything runs from `/tmp/extapp-p0/env.sh`, which is scratch and outside the repo:

```sh
source /tmp/ironwood-build-env.sh      # nightly-2026-03-16, ARM GCC 13.3, protoc 31.1, HOST_CC=gcc-15
export PATH=/tmp/extapp-p0/bin:$PATH   # provides `xtask` (below); the env's PATH already puts /opt/homebrew/bin/uv 0.12.15 first
export CARGO_BUILD_JOBS=4
export TREZOR_MODEL=T3W1
export BINDGEN_EXTRA_CLANG_ARGS=-D__float128=double   # see "What failed" #1
export UV_FROZEN=1                                    # see "What failed" #4
```

`/tmp/extapp-p0/bin/xtask` stands in for the nix-shell `xtask` command. `xtask` locates the
workspace through `CARGO_MANIFEST_DIR`, so the working directory does not matter:

```sh
#!/bin/sh
exec cargo run -q --manifest-path /Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp/core/embed/Cargo.toml --profile xtask -p xtask -- "$@"
```

Do **not** prepend `/opt/homebrew/bin` to PATH. That picks Homebrew's stable cargo and protoc 34
over rustup nightly and protoc 31.1.

## 3. Recipe

```sh
# worktree (from the main clone)
cd /Users/bryantwolf/workspace/trezor-firmware
git fetch upstream bieleluk/sdk-wip
git worktree add -b zcash/extapp /Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp upstream/bieleluk/sdk-wip
cd /Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp
git branch --unset-upstream
# the shared config points submodule URLs at the main clone's local checkouts
git -c protocol.file.allow=always submodule update --init --recursive
source /tmp/extapp-p0/env.sh
uv sync --frozen

# 1. T3W1 emulator firmware with app loading.
#    On this branch the Makefile adds --apps automatically for T3W1 (non-BTC-only).
#    PYOPT=0 = the CI test build: --pyopt false --disable-animation --dbg-console vcp --debug-link.
timeout 3600 uv run make -C core build_unix_frozen PYOPT=0
#    -> core/build-xtask/artifacts/T3W1/firmware-emu
#       (features include app_loading, dev_keys, debuglink)

# 2. Sample app. `xtask modular` resolves ../../sdk/apps against the cwd, so run it from core/embed.
cd core/embed
timeout 1800 xtask modular build -p ethereum -m t3w1 --lang en -d -e   # emulator (host dylib)
timeout 1800 xtask modular build -p ethereum -m t3w1 --lang en         # hardware thumbv8m release, for size
#    -> sdk/apps/target/artifacts/t3w1-emu/{ethereum.elf,ethereum.proof,rootpacket_0-timestamped-signed.tmr}
#    -> sdk/apps/target/artifacts/t3w1/...   ("ethereum.elf" is the packed TRZA image, not an ELF)

# 3. Emulator (Tropic01 model starts automatically for T3W1).
cd ../   # core
timeout 3000 uv run ./emu.py --headless --disable-animation --erase --profile /tmp/extapp-p0/profile \
    --output=/tmp/extapp-p0/emu.log &

# 4. Device tests: pytest in the app's own uv project; the venv is kept outside the worktree.
export UV_PROJECT_ENVIRONMENT=/tmp/extapp-p0/eth-venv TREZOR_PATH=udp:127.0.0.1:21324
cd embed && timeout 2400 xtask modular device-tests -p ethereum -m t3w1 -e [-t tests/test_x.py]
#   or directly, with per-test output:
cd ../../sdk/apps/ethereum
timeout 1200 uv run pytest --app=$PWD/../target/artifacts/t3w1-emu/ethereum.elf --lang=en -rA -p no:cacheprovider \
    tests/test_sign_verify_message.py tests/test_getpublickey.py tests/test_sign_typed_data.py
# stop the emulator afterwards (and check that firmware-emu and model_server are gone)
```

The conftest loads the app with `debuglink.load_extapp`, which picks the `.proof` and root packet
next to the image. It also regenerates `tests/generated/messages.py` from the app's `protob/`
using `pb2py`.

## 4. What worked (evidence)

The logs are in `/tmp/extapp-p0/`, which is scratch and will not survive a reboot.

**Firmware.** `fw-build.log` shows `xtask build firmware --emulator --frozen --model T3W1 --apps
--pyopt false --disable-animation --dbg-console vcp --debug-link` finishing with exit 0.

**Emulator.** `emu.stdout` shows "Tropic model ready after 5.4 s" and "Emulator ready after
6.4 s".

**Load.** The emulator log shows the header, root-packet and chunk requests. The test prints
`Requesting ethereum.trezor.com`, then returns `ExtAppLoaded(instance_id)`.

**`dt-sign.log`: `36 passed in 41.77s`.**
- `test_getpublickey` ×4 and `test_slip25_disallowed`.
- `test_signmessage` ×9, `test_verify` ×8 and `test_verify_invalid`.
- `test_ethereum_sign_typed_data` ×10, plus `_bad_show_message_hash`, `_cancel` and
  `_show_more_button`.

**`dt-all2.log`: the full suite, after the test-helper fix.**
- Totals: `132 failed, 55 passed, 3 errors`.
- The passes are:
  - get address (10 of 12);
  - public key;
  - sign/verify message;
  - typed data;
  - 9 definitions tests.
- The first full run (`dt-all.log`, before the fix) gave `277 failed, 59 passed`. 236 of those
  failures were one harness error: `DefinitionPayload.__init__() missing … 'version'`.

## 5. What failed and the workarounds

1. **bindgen and gcc-15 headers.** The new `core/embed/api` crate generates `trezor_api.h`
   bindings using the host C compiler's include directories (`xbuild` `import_cc_compiler_includes`).
   gcc-15's `stddef.h` declares `__float128 __max_align_f128` when `__APPLE__ && __aarch64__`,
   and libclang rejects that ("`__float128` is not supported on this target").
   - Switching compilers made it worse:
     - `HOST_CC=/usr/bin/clang` fails `-Werror,-Wgnu-folding-constant` in trezor-crypto `hash_to_curve.c`;
     - Homebrew clang 22 fails `-Werror,-Wc23-extensions` in `sec/image/boot_header.c`.
   - **Fix:** keep gcc-15 and set `BINDGEN_EXTRA_CLANG_ARGS=-D__float128=double`. This affects
     only bindgen's parse. Upstream uses nix and does not hit this.
2. **The emulator app on macOS cannot be built as shipped.**
   - `build.rs` links the macOS emulator build as an executable with only
     `-Wl,-export_dynamic`. The link fails on missing `_main`, and `dlopen` could not load the
     result anyway. The Linux branch passes `-shared`.
   - `modular-xtask` accepts only ELF ("Unsupported binary format: MachO").
   - The unix loader (`core/embed/io/app_arena/unix/app_loader.c:43`) accepts only
     `target_arch == APP_TARGET_ARCH_X86_64 (1)`. It then simply `dlopen()`s the payload.
   - **Fix:** local commit `15de3664a6`. Also, `is_linux()` in the apps' `build.rs` is true on
     macOS, because `CARGO_CFG_UNIX` is set; this is harmless only because `is_macos()` is
     checked first.
3. **Test-harness drift.** The app's `tests/definitions.py` predates trezorlib's
   `DefinitionPayload` split into magic and version. **Fix:** local commit `1f71e245e4`.
4. **uv re-locks.** The repo's `pyproject.toml` uses a relative `exclude-newer = "30 days"`.
   - A plain `uv run` re-resolved and rewrote `uv.lock`, changing 21 packages. The lock was
     reverted and the venv re-synced with `uv sync --frozen`.
   - **Fix:** `UV_FROZEN=1`.
   - The old `~/.local/bin/uv` cannot parse this `pyproject.toml` at all, so always source the
     env.
5. **First emulator start: "Tropic model process died."** The temporary profile was deleted, so
   the cause was not captured. A rerun with `--profile /tmp/extapp-p0/profile` worked. If it
   recurs, read `<profile>/trezor-tropic-model.log`.
6. **Probing with `timeout 5 trezorctl get-features` is harmful.** The THP handshake on the
   emulator takes longer than that. Aborted handshakes pile up as "retransmitting" channels.
   Let pytest or debuglink connect instead.
7. **Not fixed (upstream sample-app / platform bugs; relevant to Zcash):**
   - **`test_signtx*` fails with `DataError: Progress not initialized` (45×).**
     - `sdk/apps/ethereum/src/helpers.rs` `get_progress_indicator` calls `ui::update_progress`
       and never calls `ui::init_progress`.
     - `sign_tx_eip1559.rs:188` calls `end_progress` without an init.
     - Core's `run.py` enforces init-before-report. The same code is on `cepetr/apptool`,
       `vojczejk/sdk-wip-modui` and `bieleluk/stabby`.
   - **`DataError: Timeout waiting for message` (34×).** `run.py:156`
     (`loop.wait(IPC2_EVENT, timeout_ms=1000)`) **kills the app if it sends no IPC message for
     1 s.** These runs used an unoptimized `-d` emulator app build, which is slower than a release
     build.
   - **`Translation key 'words__cancel_and_exit' not found` (33×)**: the tests' translation JSON
     is out of sync with core.
   - **ETC get-address fails (2×)**, as expected: the app header allows only coin type 60.
   - The full run ended early with `UNALLOCATED_CHANNEL` and a transport timeout, after about
     190 of 336 tests.

## 6. Timings (8-core M-series, `CARGO_BUILD_JOBS=4`)

| Step | Wall time |
|---|---|
| submodules (from local clones) | 16 s |
| `uv sync --frozen` | 2 s |
| firmware emulator, cold: xtask plus all deps, up to the bindgen failure | 231 s |
| firmware emulator, rebuild after the fix | 84 s (`cargo` 73 s) |
| Ethereum emulator app, `-d -e`, first compile | about 34 s; relink and pack 6.5 s |
| Ethereum hardware release app (`-Zbuild-std`, fat LTO) | 46 s |
| emulator start (Tropic model 5.4 s + firmware 6.4 s) | about 12 s |
| sign/pubkey/typed-data tests (36) | 42 s |
| full Ethereum suite (partial, stopped early) | 5 min 22 s |

A clean cold build of everything is roughly 5–6 min of compile.

## 7. Measurements: sample app size (Ethereum, T3W1 hardware, `release-fw`, `opt-level="z"`, fat LTO)

**Image file:** `sdk/apps/target/artifacts/t3w1/ethereum.elf` is **124,160 B**. That is the
512 B TRZA header plus the **123,648 B** payload (`code_size`).

**Payload (armv8m code header of 128 B, decoded):**
- RO segment `ro_va = 0xC0000000`, `ro_size = 0x1DA60` = **121,440 B**. This is `.text`
  110,754 + `.rodata` 10,660, aligned.
- RO relocations 0x804 = 2,052 B.
- RW `rw_va = 0xD0000000`, `rw_size = 0x803C` = **32,828 B**. It is all `.bss`, with no `.data`
  init. Most of it is two SDK statics:
  - the **16 KiB allocator heap** (`app_runtime.rs`, `HEAP_SIZE`, hard-coded);
  - the **16 KiB IPC inbox** (`lib.rs`, `static_service!(CORE_SERVICE, …, 16384)`).
- `stack_size` 0x4000 = **16,384 B**; `heap_size` 0x400 = **1,024 B**.

**Header `data_size` = 50,240 B** (= RW 32,828 + stack 16,384 + heap 1,024 + alignment). xtask
reports "Code segment: 120.6 KB (incl. 2.0 KB relocations, 0 B init data)" and "Data segment:
49.1 KB (incl. 16.0 KB stack, 1.0 KB heap)".

**Arena use:** code + data = **173,888 B**, which is **44 % of the T3W1 384 KiB arena** and would
be 73.5 % of the T3T1 WIP 231 KiB arena.

**How the SDK expresses sizes:**
- `[package.metadata.trezor] stack-size / heap-size` in the app's `Cargo.toml`:
  - Ethereum declares 16384 and 1024.
  - xtask caps each at 256 KiB (`sdk/crates/modular-xtask/src/metadata.rs:91-110`).
  - They are written into the 128 B armv8m code header (`armv8m.rs`, `Armv8mBinaryHeader`).
- The RW size comes from the ELF.
- The header's `data_size` is RW + stack + heap.
- **Caveat:** the Rust global allocator ignores `heap-size`. It uses the fixed 16 KiB static in
  `.bss`, which sdk/doc/development.md itself warns about. A Zcash app needing about 90 KiB of
  heap must either enlarge that static, which counts in RW, or wire `app_get_heap` into the
  allocator.

**Where the arena limit is enforced (device):**
- `core/embed/models/T3W1/memory_secmon.h:112-113` defines `APP_ARENA_RAM_START 0x20200000` and
  `APP_ARENA_RAM_SIZE (384*1024)`.
- `core/embed/models/T3T1/memory.h:86-87` defines 231 KiB at `0x3006A000` (WIP).
- `core/embed/io/app_arena/app_arena.c`:
  - `:133-139` (emulator: `malloc` of 64 MiB instead);
  - `:265-268` (one image takes the whole arena, `APP_ARENA_MAX_IMAGES 1`);
  - `:398-400` `app_image_write_chunk` checks `written + size <= mem_size` (ENOMEM) and
    `<= header.code_size`.
- `core/embed/io/app_arena/stm32u5/app_loader.c`:
  - `:189,206,209`: rw/stack/heap are each `< APP_ARENA_RAM_SIZE`;
  - `:251`: `fit_in_memory` checks `data_size >= rw + stack + heap` for the space left after the
    code;
  - `:275`: the aligned heap end must be within the arena.
- On the unix emulator the app gets all remaining arena memory as heap (`unix/app_loader.c:88`).

## 8. Map for P1 (Zcash key service) and P2 (Zcash app)

**Wire (host ↔ core).**
- `common/protob/messages-extapp.proto` defines IDs 9200–9209:
  - `ExtAppLoad{id, version, fingerprint}` → `ExtAppHeaderRequest` → `ExtAppHeaderAck{header, proof, root_packet_timestamp}`;
  - → (`ExtAppRootPacketRequest{app_ring}` → `Ack{root_packet}`) only if the stored root timestamp differs;
  - → `ExtAppDataChunkRequest{index}` → `Ack{data, hash}` ×N;
  - → `ExtAppLoaded{instance_id}`.
- Then `ExtAppMessage{instance_id, message_id, data}` ↔ `ExtAppResponse{message_id, data, finished}`.
  `finished=false` means the app wants more (a WireContinue round trip); the host answers with
  another `ExtAppMessage`.
- An app error arrives as `Failure{code, message}`.
- Dispatch is in `core/src/apps/workflow_handlers.py:101-105`, to `apps.extapp.load` and
  `apps.extapp.run`.
- The instance id is kept in the sessionless cache `APP_EXTAPP_IDS` (`load.py`).

**Host side (trezorlib).**
- `python/src/trezorlib/extapp.py`:
  - `AppHeader`/`AppImage` construct structs (512 B header, "TRZA");
  - `AppImage.chunks()` builds the reverse SHA-256 hash chain;
  - `load(session, binary, proof, root_packet, min_version)`.
- `python/src/trezorlib/debuglink.py:1850` `load_extapp(session, path)` picks
  `<app>.proof` and `rootpacket_0-…tmr` (ring 0) or `rootpacket_12-…tmr` (rings 1 and 2) next to
  the image.
- The per-app host shim, e.g. `sdk/apps/ethereum/tests/ethereum_ext.py:call_ext`:
  - it serializes the app's own protobuf and wraps it in `ExtAppMessage` with the app's own
    `MessageType` number;
  - it decodes `ExtAppResponse.data` by `message_id`.
- The app's protobuf lives in `sdk/apps/<app>/protob/`: `messages.proto` holds the app-local
  `MessageType` enum. Python classes are generated by `protob/pb2py`.
- Dev signing: `core/tools/trezor_core_tools/extapp_tool.py build-dev-bundle <images>` builds
  Merkle proofs and dev-signed root packets. xtask runs it after every build.

**Coreapp services** (`core/src/apps/extapp/run.py`; IPC function id = `service << 16 | message_id`).

| Service id | Name | Operations |
|---|---|---|
| 0 | WireStart | core → app, carries the request |
| 1 | WireContinue | |
| 2 | WireEnd | |
| 3 | WireError | |
| 4 | UI | |
| 5 | Progress | Init 0, Report 1, Stop 2 |
| 6 | Crypto | GetXpub 0, GetPublicKey 1, SignDigest 2, SignTypedHash 3, GetAddressMac 4, CheckAddressMac 5, VerifyNonceCache 6 |

- The SDK side of these ids is `sdk/crates/trezor-app-sdk/src/service.rs` `CoreIpcService`.
- Payloads are **rkyv** archives of enums shared by the app and core:
  `sdk/crates/trezor-app-sdk/src/structs.rs`.
  - `TrezorCryptoEnum` (:835), whose `id()` is the message_id.
  - `TrezorCryptoResultRef` (:890).
  - `TrezorUiEnum` (:801).
  - `TrezorProgressEnum`.
  - Core links the same crate: `core/embed/rust/Cargo.toml` `trezor-app-sdk`, feature `app_loading`.
- Core-side bridges into MicroPython:
  - `core/embed/rust/src/crypto/api/firmware_micropython.rs` (`deserialize_crypto_message`,
    `send_crypto_result`);
  - `core/embed/rust/src/ui/api/firmware_micropython.rs` (`process_ipc_message`,
    `send_ui_result`, progress).
  - They are registered in `core/embed/upymod/rustmods.c` as `trezorcrypto_api` and
    `trezorui_api`.
- App-side calls go through `sdk/crates/trezor-app-sdk/src/crypto.rs`:
  - `get_xpub`, `get_public_key`, `sign_digest`, `sign_typed_hash`, `get/check_address_mac`,
    `verify_nonce_cache`;
  - each wraps `ipc_crypto_call`, which is a synchronous `services_or_die().call(Crypto, …)`.
- UI is in `sdk/.../ui.rs`: `confirm_value/summary/action/properties`, `show_address`,
  `show_public_key`, `show_warning/danger/success`, `request_number`, menu/info flows and
  `init/update/end_progress`.
- Direct, seedless crypto goes through the `trezor_crypto_v1_t` function table
  (`core/embed/api/trezor_api_v1.h:54-90`): SHA-2, SHA-3, HMAC, ECDSA verify/recover, ed25519
  verify and cosi. **It has no BLAKE2b.**

**App entry and dispatch** (`sdk/apps/ethereum/src/main.rs`).
- `#[no_mangle] fn app()` loops on `wire_receive_wire_start()`.
- `wire_handler!(handler, ProstCodec, Req, MessageType::Resp, fn)` handles one request.
- `wire_request_type!(Req => Ack, MessageType::Req)` sets up mid-flow host round trips (for
  streaming, as in `TxRequest`).
- A recoverable error is sent back as `WireError`; an `Err` returned from `app()` terminates the
  app.

**How an app declares curves and paths.**
- In `Cargo.toml` `[package.metadata.trezor]`:
  - `id`, `name`, `vendor`, `stack-size`, `heap-size`, `app-ring`, `curves = [...]` (≤ 64 B
    packed) and `paths = [...]` (≤ 256 B packed, NUL-separated);
  - these are packed into `app_header_t.curves[64]` and `.paths[256]`
    (`core/embed/io/app_arena/inc/io/app_header.h:75-80`).
- On each `ExtAppMessage`, `run.py:97-115` reads them through `image.allowed_curves()/allowed_paths()`
  (`core/embed/upymod/modextapp/modextapp-image.h`):
  - it requires **exactly one curve** (`:100-101`);
  - it takes the SLIP-44 coin type from path component 2, which must be the same in every
    pattern (`_extract_slip44_id`);
  - it builds `paths.PathSchema`s.
- **There is no entitlement field** yet. The header has reserved bytes (`reserved_1`,
  `reserved_2`, and the padding to 512).

**P1 insertion points** (for a ZIP-32 Orchard `sk`, seed-fingerprint and backup-strength
operation). Each item is a small addition to an existing table:
1. Add `TrezorCryptoEnum::GetZip32OrchardKey { account: u32, coin_type: u32 }` (id 7) and a
   **new tagged** `TrezorCryptoResultRef` variant. The core serializer
   (`crypto/api/firmware_micropython.rs:161-200`) **maps results by length** (32 B → AddressMac,
   65 → Signature, 111 → Xpub), so a 32 B `sk` would be mis-tagged. Add an explicit tag, like
   the `(0, bytes)` path used for PublicKey.
2. Add a deserializer arm (`:47` onward) and a `run.py` branch (`_SERVICE_CRYPTO_…=7`). It should:
   - derive from the seed through `apps.common.seed` or the keychain cache, not `get_keychain`
     (BIP-32);
   - check `m/32'/133'/a'` against the app's `paths` (the `PathSchema` / `_extract_slip44_id`
     code already accepts `m/32'/133'/account'`);
   - check an entitlement.
3. Add an SDK wrapper in `crypto.rs`, and update `core/mocks/generated/trezorcrypto_api.pyi`.
4. Choose where the entitlement lives: a header field (`reserved_*`), or a pseudo-curve string
   in `curves`. The pseudo-curve would run into the one-curve rule once transparent secp256k1
   is also needed.

**Device-test harness patterns.**
- Per app: `sdk/apps/<app>/tests/`, a standalone uv project whose `pyproject.toml` pulls
  `trezor` from `../../../python`.
  - `conftest.py` is a copy of core's device-test conftest: `--app=` path, `setup_client`
    wiping and loading the `all all …` seed, then `debuglink.load_extapp`; there is an
    `instance_id` fixture.
  - Also: `input_flows.py` (Eckhart/Delizia layouts), `<app>_ext.py` (`call_ext`), and
    `ui_tests/` fixtures.
  - Run with `xtask modular device-tests -p <app> -m t3w1 -e [-t …] [--ui]`, against an
    already running emulator.
- Platform load tests: `bieleluk/extapp-device-tests:tests/device_tests/extapp/`, not merged
  here. They build synthetic `AppImage`s, compile an `applet_main` stub with the host cc for
  dlopen, and use `extapp_tool` for proofs. They cover proof, chunk, root-packet and eviction
  failures. That is the model for testing a new crypto op without a full app.

## 9. Top blockers and risks for a Zcash app

1. **No seed or ZIP-32 path for apps (P1).** The Crypto service is BIP-32 only, one curve,
   SLIP-44-from-path. Results are length-tagged. There is no entitlement field and no BLAKE2b in
   the direct crypto table, but the app can carry its own BLAKE2b.
2. **1 s inter-message watchdog.** `run.py:156` kills the app if it sends no IPC message within
   1 s. Orchard/RedPallas signing and PCZT verification on a Cortex-M33 must report progress
   often, or the timeout must change. **Progress is also buggy in the sample app**, so we must
   init/report/stop correctly ourselves.
3. **Heap.** The SDK allocator is a fixed 16 KiB `.bss` static, independent of `heap-size`.
   Zcash needs about 90 KiB. We need an SDK change (use `app_get_heap`) or a bigger static.
   - The 16 KiB IPC inbox caps a single IPC message.
   - Streamed PCZT chunks must fit.
   - `bieleluk/stabby` has "declare IPC inbox size in manifest", which is not on this branch.
4. **Platform hardening we inherit, noted for the review record.**
   - Core deserializes app-supplied rkyv with `access_unchecked`
     (`crypto/api/firmware_micropython.rs:47`, `ui/api/…:1267,1665`), so there is no validation of
     untrusted app bytes.
   - GetXpub and GetPublicKey use `AlwaysMatchingSchema` (`run.py:192-194, 211-213`), so path
     prefixes are not enforced for public keys.
   - `run.py` imports `apps.ethereum.definitions` (Ethereum coupling in the generic service).
   - The ed25519 `sign_digest` result (64 B) matches no length arm.
5. **Churn.** The builder tool is being renamed (`trezor-app-tool`, `xtask apps` on
   `cepetr/apptool`), and the UI layer is being replaced (`modui` on `vojczejk/sdk-wip-modui`,
   which disables Ethereum). Pin `4cd93ff4d8` for P1/P2 and rebase deliberately.
6. **macOS is not an upstream dev platform** (fixes #1 and #2 above). Keep the local commit, or
   build in Docker/nix, when reproducing on another machine.
