# Adversarial correctness review: `zcash/extapp..zcash/extapp-signstart` (P3 integrated)

- Reviewer model: **Fable 5.1**, model ID `claude-fable-5-1` (in-session Claude Code subagent). This is a genuine Fable run, not a substitute.
- Date: 2026-09-26
- Repo/worktree: `/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-demo`, branch `zcash/extapp-signstart` @ `07a27d1d76`
- Range: `1f71e245e4` (`zcash/extapp`) .. `07a27d1d76`, 29 commits, 109 files, +22,611/-347
- Mode: read-only source review. No build, no test run, no device session. I did not edit anything except this file.

## Verdict

**Approve with changes.** I found no way, tied to code, for a malicious host or a second app to (a) obtain a spend-authorization signature over data the user did not review, (b) make the device show one recipient/amount/fee while signing another, or (c) obtain the viewing key, seed fingerprint or spending key without the consent screens. The streaming protocol, the session's verification chain and the SDK's reply matching hold up under the replay/reorder/truncation/abort scenarios I traced.

One **must-fix** is a build-gating gap: commit `d485d458c5` (in-kernel ML-DSA-44, 36 KiB kernel stack, `MLD_CONFIG_REDUCE_RAM`) is labelled test-device-only but its `#if` gate is *not* excluded from `PRODUCTION` builds, so nothing in the code enforces the "must never reach production" invariant; the sibling commit `ad44c809e8` is gated correctly. The should-fix items are key-material hygiene on the Core side of the key service, the wording/strength of the one consent screen that hands an app a spending key, a robustness defect that kills the app after any host `Cancel` mid-request, and an un-versioned ABI table extension.

## Findings

### Must-fix

**M1. Test-device-only kernel changes are not excluded from production builds.**
- `core/embed/sec/mldsa44/inc/sec/mldsa44.h:28-30`: `#if defined(KERNEL) && defined(USE_SECMON_LAYOUT) && !defined(BOOTLOADER_DEVEL)` → `#define MLDSA44_IN_KERNEL`. `PRODUCTION` is not consulted. `KERNEL`, `USE_SECMON_LAYOUT` and `PRODUCTION` are all model-level defines exported from `core/embed/models/build.rs:133-139,154` to every C library via xbuild metadata, so a `production`-feature T3W1 kernel (secmon layout, no `bootloader_devel`) would define `MLDSA44_IN_KERNEL`, compile `mldsa44.c` into the kernel (`mldsa44.c:24`), drop the smcall stub (`smcall_stubs.c:469`) and take the 36 KiB kernel stack (`core/embed/sys/linker/stm32u5g/kernel.ld:53`). `core/embed/crypto/build.rs:349` adds `MLD_CONFIG_REDUCE_RAM` unconditionally, so it also changes any rebuilt secmon's mldsa-native configuration.
- Scenario: the branch is used as the base of an upstream series or a "production-flavoured" test image; nothing fails the build, the kernel silently verifies app root packets outside the secure world.
- Fix: add `#if defined(MLDSA44_IN_KERNEL) && defined(PRODUCTION)` `#error "test-device-only"` (or gate the whole thing on a dedicated `test_device_mldsa_in_kernel` cargo feature that xtask cannot combine with `production`), make the `kernel.ld` and `build.rs` changes conditional on the same feature, and keep the commit out of any series. Contrast: `core/embed/io/app_arena/root_packet.c:39-40` (`ad44c809e8`) correctly uses `!defined(PRODUCTION)`, and `PRODUCTION` is only defined when the `production` feature is on, so dev root keys are refused in production as the commit message says.

### Should-fix

**S1. Spending-key copies on the Core side of the key service are never wiped.**
- `core/src/apps/extapp/zip32_orchard.py:165-177`: the master `I` and each child `I` are immutable `bytes` from `blake2b(...).digest()`; the returned `memoryview` slices them. Four 64-byte blobs containing `sk‖c` for the master and each hardened level stay in the MicroPython GC heap until overwritten.
- `core/embed/rust/src/crypto/api/firmware_micropython.rs:257`: `Obj::try_from(bytes.as_ref())` allocates a fresh MicroPython `bytes` holding the serialized result (spending key included) for `ipc_cb`; `WipedBuffers`/`WipedResult` (lines 156-183) zero only the Rust-side copies.
- `core/embed/sys/ipc/ipc.c:190`: `ipc_message_free` only sets `item->free = true`; the kernel queue item in the app's inbox keeps the key bytes until overwritten by later messages. The SDK's `Zip32OrchardAccount` doc (`crypto.rs`) acknowledges the inbox copy and Rust move copies.
- Core's own convention for secrets (HDNode `__del__` memzero, bytearray seed) is stricter than this. Exposure requires a memory-disclosure bug, but a spending key crossing a trust boundary is exactly where hygiene matters.
- Fix: derive in Rust (or C `trezorcrypto`) into zeroizing buffers and hand `ipc_send` a `bytearray` that is `memzero`ed after sending; have the SDK zero the received `IpcMessage` payload before `ipc_message_free` for Crypto replies. The author lists this as a follow-up in `notes/zcash/EXTAPP_P1_KEYS.md §7.5`; I am promoting it to should-fix for the branch.

**S2. The account-consent screen understates what it grants and is a single tap.**
- `core/src/apps/extapp/zip32_orchard.py:103-118`: "Allow {app} to use your Zcash account #N (network)? It can see your balance and create transactions for you to confirm.", `verb="Allow"`, `ButtonRequestType.Other`, no `hold`.
- What is actually returned (`run.py:375-388`) is the raw ZIP-32 Orchard spending key. After this tap the app can sign anything; "for you to confirm" is true only because the *app's* code shows screens. This screen is the sole Core-owned gate between the wallet seed and an app, so it should read like a key export (e.g. "Allow {app} to spend from your Zcash account #N (network)?"), use hold-to-confirm, and ideally show the header's `vendor` too, since on non-production firmware (`root_packet.c:39-40`) the header is dev-key-signed and `name` is attacker-chosen. Hardcoded English also needs `TR` strings before any upstream proposal (author already notes this).

**S3. A host `Cancel` mid-request desynchronises app and Core; the next request is lost and, if a progress screen was up, the app is killed.**
- Trace: Core's `run()` (`core/src/apps/extapp/run.py`) is aborted by `ActionCancelled` from `context.call` while the app is blocked in `IpcRemote::call` (`sdk/crates/trezor-app-sdk/src/service.rs:111-126`) for a UI result or a `WireContinue` chunk. The next `ExtAppMessage` starts a fresh `run()` with `progress_obj = None` (`run.py:139`) and delivers `WIRE_START`; the app's `call` returns `UnexpectedService` (correct: no consent bypass), the handler errors, `wire_error_raw` (`sdk/.../wire.rs:51-53`) calls `end_progress`, `PROGRESS_SHOWN` (`ui.rs:83,153`) is still true, so a Progress `STOP` reaches Core, which hits `run.py:475-477` `die(DataError("Progress not initialized"))`, stops the app and answers the host's *new* request with that failure. The host must `ExtAppLoad` again. Without a progress screen the new request is still answered with the stale request's failure.
- Not a security issue (the service check is what prevents a forged `Confirmed`), but Suite sends `Cancel` routinely. The author documents the lost request in `EXTAPP_P2B_SIGN.md` ("Cancel") but not the kill.
- Fix: make Core treat a Progress `STOP` with no progress as an acked no-op (keep `die` for `REPORT`), and/or have the SDK reset `PROGRESS_SHOWN` when a `WireStart` arrives while a call is pending; longer term, deliver an abort IPC to the app when `run()` exits abnormally.

**S4. `trezor_api_v1_t` grew without an ABI version bump.**
- `core/embed/api/trezor_api_v1.h:135` appends `rng_fill_buffer`; `sdk/.../low_level_api/api.rs:325-328` does `unwrap!(get_or_die().rng_fill_buffer)(...)`. An app built with this SDK on a coreapp whose table ends at `trezor_crypto_v1` reads past the static; whatever word follows is not necessarily zero, so the `Option` is `Some(garbage)` and the app jumps to it. `coreapp_api_get(version)` exists for exactly this; use `v2` (or a table-size field) so an old coreapp returns NULL and the SDK fails closed. The appended-field placement itself is correct.

**S5. `xtask modular build --production --debug` is accepted.**
- `sdk/crates/modular-xtask/src/args.rs:255-276`: `production` only removes `dev_keys`; `debug` independently enables `trezor-app-sdk/debug`, i.e. logging plus the `diagnostics` counters and the `ZcashGetDiagnostics` handler (`sdk/apps/zcash/src/main.rs:75-83`, `sign_pczt.rs:291-296`). Nothing secret leaks (heap/timing counters only), but a production-signed image should not carry debug transport; reject the combination in `resolve_features`. Otherwise the debug gating is correct: `diagnostics` is `cfg(all(feature="app", feature="debug", not(feature="test")))` (`lib.rs:47-48`), `PeakHeap` only under `debug` (`app_runtime.rs:224-229`), and release builds send `diagnostics: None`.

### Nits

- N1. `sign_pczt.rs:232` `keys.spending_key.to_bytes()` and `prepare`/`sign` (`FullViewingKey::from`, `SpendAuthorizingKey::from`) leave unwiped `[u8;32]`/64-byte temporaries on the stack; `ironwood/src/hedge.rs:71-76` says "the caller must wipe the stack after session_begin" and no caller does. Consider a stack-scrub after `begin` and `sign`, or accept and document.
- N2. `run.py:63-72` `_coin_type`: `pattern.split("/")[2]` and `component[-1]` raise `IndexError` (not `DataError`) for a two-component or empty pattern. Header patterns are signed, so low impact.
- N3. `run.py:425-432, 506-513`: `ack.instance_id` is never re-checked against the cached instance (pre-existing, now load-bearing for a multi-message protocol). Cheap to check.
- N4. Zcash app declares `m/32'/{133,1}'/account'`; `paths.py` maps `account` to 0..100 while `account.rs:31-36` accepts up to 2^31-1. Accounts 101+ fail with a generic `ApiError(Failed)` text rather than a policy message. Either declare `[0-2147483647]'` (as the P1 note suggests) or map the error.
- N5. 16-byte SLIP-39 seeds get a ZIP-32-shaped fingerprint and derivation outside ZIP 32's 32..252-byte range (`zip32_orchard.py:37-41,180-190`; `ironwood/src/lib.rs:233-244`). Internally consistent and documented, but no other wallet can reproduce it. Flag in the design note as a Trezor product decision.
- N6. `app_arena/stm32u5/app_loader.c` zeroes the app's RW/stack/heap on *prepare* (`memset(data, 0, data_size)`), not on stop/delete (`app_arena.c:507-514, 368-371`). The previous app's residue (including any unwiped key copies) sits in arena RAM until the next app is prepared. Only the kernel can read it, so low priority; a wipe on stop would make N1/S1 moot across instances.
- N7. `run.py:317-322, 343-349`: address-MAC coin type is now `unharden(address_n[1])` of the validated path instead of the pattern's coin type. Equivalent under strict safety checks; under relaxed checks the app's MAC differs from Core's native Ethereum (`defs.network.slip44`). Self-consistent within the app, so only a cross-implementation note.
- N8. `ui.rs` `Progress::drop` swallows the `End` failure; `wire.rs:35` `wire_respond_raw` propagates it and then never sends the response. Pick one behaviour (the drop's is fine).

## What I verified (by reading, with the scenario traced to code)

**Signer / streaming protocol (`sdk/apps/zcash/src/sign_pczt.rs`, `sdk/apps/zcash/ironwood/src/{session,stream,digest,lib,hedge}.rs`)**
- Chunk framing: `request_chunk` (`sign_pczt.rs:254-286`) requires `transfer_id` (16 random bytes from the device RNG per request), exact `offset` and exact `length`; the id of the reply must be `ZcashPcztAck` or `ZcashCancel`. Replay of an old chunk, reordering, short/long data and a foreign transfer id all end the request. Because the device pulls sequentially and verifies every byte as it arrives, "substituting a different PCZT mid-stream" is just a different transaction, all of which is reviewed.
- Truncation/trailing bytes: `Scanner::feed` (`stream.rs:651-746`) fails `State` if fed past the declared total and `Malformed` if the grammar and the declared length do not end together (line 725). `Event::None` with zero bytes consumed is a `State` error in the app (`sign_pczt.rs:147`), so no spin.
- Abort/restart: `Session`, `Review`, `AccountKeys`, `Wiped<FullViewingKey>`, `Wiped<SpendAuthorizingKey>` are locals of `sign_pczt`; every exit drops them; `Session::drop` → `reset` → `Stream`/`Records`/`Scanner`/`IvkCache` zeroize in place. `feed_with_progress` resets the session on any error (`session.rs:407-411`). The only cross-request statics in the app are `PROGRESS_SHOWN` (S3) and the public Pasta/orchard caches from `prewarm` (constants, not secrets).
- Cross-app/cross-instance state: app RAM is zeroed on prepare (N6); IPC inboxes are per (target, remote) queue keyed by the kernel's notion of sender (`ipc.c:212-252`), so an app cannot inject a message that the SDK attributes to CoreApp; `IpcRemote::call` rejects a reply whose service differs from the request (`service.rs:121-125`) and `wire_receive_wire_start` requires `WireStart` (`wire.rs:114-116`). This is what closes the "send `Cancel`, then a crafted `ExtAppMessage` that rkyv-decodes as `TrezorUiResult::Confirmed`" path: it arrives as `WireStart`, not `Ui`.
- What is shown equals what is signed: for every action the session checks (`session.rs:924-1089`) the wire FVK equals the session FVK for real spends, `rk != identity`, `verify_cv_net` (cv_net = ValueCommit(v_in − v_out, rcv)), nullifier ownership under the device FVK/classifier, `verify_rk` (rk = ak + [alpha]G with the device's ak), `verify_note_commitment` (cmx ↔ recipient/value/rseed), and `verify_encryption` (`lib.rs:1142-1235`: enc_ciphertext decrypts under pk_d/esk to the cmx-bound note; memo recovered from the signed ciphertext; OVK recovery rules for payment vs change). The displayed receiver is the verified `recipient`; a host `user_address` is shown only if it decodes for the selected network and its Orchard receiver equals the verified one (`sign_pczt.rs:317-342`, `unified.rs:26-39`). Change is classified by `ScopeClassifier::scope_for_address` (re-derives the address from the device's internal ivk), so a foreign address cannot masquerade as change. Fee = Σspend − Σoutput − Σtransparent, checked against the trailer's `value_sum` which is hashed into the sighash, and capped at 0.01 ZEC (`session.rs:616-626`). Total shown = payments + transparent + fee (`sign_pczt.rs:407-411`).
- Digest: `digest.rs` streams the ZIP-244 tree with fixed V6 header constants; admission enforces `version/group == V6` before `Digest::new` (`session.rs:804`). The anchor's absence from the txid node is consistent with the pinned fork (`orchard e23177a/src/bundle/commitments.rs:98-102,166` gates the anchor per format). I relied on the crate's `digest_equivalence`/`session_equivalence` suites for byte equality with upstream `v6_signature_hash`; I did not run them.
- Consent → sign binding: `approve(token)` then `sign(token, ask)`; both compare the retained token and clear the slot on any exit (`session.rs:691-733`); `sign_records` re-checks `ak` and `rk` against `ask`/`alpha` before signing (`session.rs:743-778`). Signatures are released only when all real spends signed.
- Hedged nonces (`hedge.rs`): `BLAKE2b(secret‖entropy‖counter‖i)` with `secret = BLAKE2b(0x00‖sk)`; the argument that a dead TRNG degrades to a deterministic scheme keyed by a device secret holds. `DeviceRng` is only ever wrapped in `Hedged` (`sign_pczt.rs:232,477-507`).
- Memo display filter (`lib.rs:154-216`): supplementary planes, controls other than `\n`, NBSP, invisible marks, zero-width and bidi controls are hashed instead of shown. Sound.

**Key service (`run.py`, `zip32_orchard.py`, `firmware_micropython.rs`, `structs.rs`, `crypto.rs`)**
- Entitlement: `curve == "zip32-orchard"` from the signed header, one curve per app, so a key-service app cannot also use BIP-32 ops (`run.py:88-91`, `zip32_orchard.py:92-93`).
- Path: only `m/32'/{133|1}'/account'`, `account < 2^31`, plain `schema.match` against the header's patterns with no prompt fallback (`zip32_orchard.py:94-100`).
- Consent: per (instance id, coin type, account), instance id is a fresh `random.uniform(2**32-1)` per load/run (`load.py:132`), approvals live in the session cache (44 B, newest 8), so a reload or a new session re-prompts; policy checks run before any screen; decline → `[_RESULT_CANCELLED]` and nothing cached (`zip32_orchard.py:67-73`, `run.py:389-390`). Tested by `core/tests/test_apps.extapp.zip32_orchard.py` (16 tests, names read, not run).
- What is returned to whom: `[tag 1, sk, fingerprint, weak_backup]` only to the requesting task via `crypto_resp_cb` (`run.py:141-144`); the SDK exposes it only through `get_zip32_orchard_account` and refuses it in the cloneable `TrezorCryptoResult` (`crypto.rs:127-132`).
- Derivation matches ZIP 32 by reading: master `BLAKE2b-512("ZcashIP32Orchard", S)`, child `PRF^expand(c, 0x81‖sk‖I2LEOSP_32(i))`, fingerprint `BLAKE2b-256("Zcash_HD_Seed_FP", len‖S)`; hardened-only enforced (`zip32_orchard.py:157-190`). Reference vectors exist in the unit tests. The app validates `sk` with `SpendingKey::from_bytes` (`account.rs:85-86`).
- Request validation: `access_crypto_request` uses checked `rkyv::access` and ties the archived variant to the IPC message id (`structs.rs:503-518`); UI and Progress requests are checked too; Core turns failures into `ValueError` → `die`, never UB (`firmware_micropython.rs` crypto:51-53, ui:1268-1269, 1657-1658). `DEVICE_IPC_ALIGNMENT` static asserts pin reply alignment (`structs.rs:574-577`). The SDK unit tests cover truncation, misalignment, bad tags, OOB relative pointers, bad bools, non-UTF-8.

**SDK memory/robustness**
- Allocator: `HEAP.init(start, size)` from the loader's `app_get_heap`; a zero-size heap leaves `LlffHeap::empty()` so every allocation fails cleanly (`app_runtime.rs:249-257`). `PeakHeap` wraps `LlffHeap` and only adds atomics (`diagnostics.rs:273-307`).
- RNG: `SYSCALL_RNG_FILL_BUFFER` admitted for applets (`syscall_dispatch.c:128`); the verifier already does `probe_write_access` (`syscall_verifiers.c:856-866`).
- Progress: `update_progress`/`end_progress` fail closed locally when no screen is shown (`ui.rs:131-159`), so the app can no longer trigger Core's `die` by mis-ordering; `keeping_alive` records the first failure and returns it after the work completes (`layout.rs:33-47`). Core clamps the bar at 1000 (`trezor/ui/__init__.py:596-600`).
- run.py changes: logging only under `__debug__`; bridge failures raise into `run()` rather than fatal; a kernel-killed app is reported as "Task stopped" on timeout (`run.py:156-161`). None of these weaken checks for other apps; the address-MAC change is N7. Ethereum/Tron `heap-size` 1024 → 16384 preserves the old static 16 KiB now that the allocator uses the manifest value.

**Debug / test-only leakage**: see M1 (leaks), S5 (allowed combination), and the "correct" list under S5. `ad44c809e8` is correctly gated on `!defined(PRODUCTION)`. Debug-only protobuf messages exist in every build's schema; in release `ZcashGetDiagnostics` maps to `InvalidFunction` and `diagnostics` is `None`.

## What I did not verify

- No build, no `cargo test`, no device tests, no emulator; all numbers in the author's notes (heap peaks, stack depth 28,564 B, 1 s watchdog margins, 56/56 device tests, 126/126 crate tests) are unverified claims.
- The orchard fork at `e23177a9dd` beyond the functions grepped (`scope_for_address`, `scope_classifier`, `wipe`, `verify_cv_net`, anchor gating); in particular `verify_nullifier_with_progress`, `recover_output_bound_with_*` and Ironwood-v3 flag semantics are trusted.
- Byte-equality of `digest.rs` with upstream `v6_signature_hash` (relies on the crate's equivalence tests).
- `ironwood/src/receive/*` (address/FVK derivation for GetAddress/GetViewingKey), `pb2py`, `build.rs`, the Python test shim, translations.
- The claim that `ffi.rs` is byte-identical to bindgen output.
- Whether the design document `docs/common/zcash-ironwood-signing.md` (referenced by the crate) matches the code: it is not in this tree (the crate says so at `lib.rs:6-8`).
- Stack depth of `SignPczt` on hardware; the `#[inline(never)]` split is plausible but unmeasured here.

## Author claims checked against code

| Claim (notes) | Result |
|---|---|
| RNG verifier checks writability (`EXTAPP_P2B_SIGN.md`) | Confirmed, `syscall_verifiers.c:856-866` |
| `rng_fill_buffer` appended so offsets do not move | Confirmed; versioning still missing (S4) |
| Consent shown only after policy checks | Confirmed, `zip32_orchard.py:67-71` |
| Rejection caches nothing | Confirmed |
| Hedge holds with `BLAKE2b(0x00‖sk)` | Confirmed by reading `hedge.rs` |
| "The only key material this module owns is zeroized with the stream" | Confirmed for heap state; stack temporaries excepted (N1) |
| Cancel mid-stream loses the next request | Confirmed, and it also kills the app when progress is shown (S3) |
| Remaining unwiped copies listed in P1 §6 | Confirmed and one more (kernel queue item) (S1) |
| `ad44c809e8` accepts dev keys only in non-production | Confirmed |
| `d485d458c5` is test-device-only | Intent yes; code does not enforce it (M1) |
