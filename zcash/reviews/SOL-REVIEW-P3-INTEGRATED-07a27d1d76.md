# GPT-6 Sol adversarial review of the integrated extapp (07a27d1d76)

Model: `gpt-6-sol` via Codex CLI 0.157.0, reasoning effort medium, read-only sandbox; 121,065 tokens. Range `zcash/extapp..zcash/extapp-signstart`. Prompt: the author's `whole-diff-review/SOL-P3-INTEGRATED-PROMPT.md`.

## Verdict

**Do not ship this range to production as-is.** I did not find a host-controlled chunk replay or reordering path that bypasses the Zcash app’s output and totals review. The release blockers are the test-device kernel-verification change and the security contract of giving an entitled app a raw spending key. This was a **read-only source review**; I did not build, run tests, inspect a built image, or reproduce an attack on hardware.

### Findings

1. **Must-fix — test-device verification path is enabled by the production condition.**  
   `core/embed/sec/mldsa44/inc/sec/mldsa44.h:24–30`, `core/embed/sec/mldsa44/mldsa44.c:24–54`, `core/embed/sys/linker/stm32u5g/kernel.ld:50–54`  
   On a T3W1 kernel using the released secmon layout, `MLDSA44_IN_KERNEL` is selected whenever `BOOTLOADER_DEVEL` is *absent*—including a production build. This puts root-packet signature verification in the kernel and reserves a 36 KiB kernel stack, despite the commit being designated test-device-only. A production image built from this range can therefore contain the test workaround. **Fix:** remove these changes before production, or gate them on an explicit test-only build option that production builds reject; verify the resulting image and stack map.

2. **Must-fix if “no key export without an export consent screen” is a requirement — the direct-key grant permits it.**  
   `core/src/apps/extapp/zip32_orchard.py:49–80,103–118`, `core/src/apps/extapp/run.py:368–388`  
   Once the user allows an entitled app to use an account, Core sends that app the **raw spending key and seed fingerprint**. Approval is cached for that app instance/account, so the app can ask again without another screen. A malicious or compromised *entitled, signed* app can send the key, fingerprint, or a derived viewing key to its host without invoking the Zcash app’s viewing-key-export screens. The grant screen says the app can see balance and create transactions, but does not say it receives an exportable spending key. **Fix:** if per-export consent is promised, keep signing/derivation inside a trusted service rather than returning the spending key. Otherwise explicitly treat this as a full-key trust grant, make the warning unambiguous, and restrict the entitlement to tightly reviewed app identities. An unentitled app is refused by the curve and path checks at `zip32_orchard.py:83–100`.

3. **Should-fix — seed-derived key copies remain in Core and IPC memory after the explicit wipes.**  
   `core/src/apps/extapp/zip32_orchard.py:157–177`, `core/embed/rust/src/crypto/api/firmware_micropython.rs:224–257`, `sdk/crates/trezor-app-sdk/src/crypto.rs:281–305`  
   ZIP-32 derivation creates immutable BLAKE2b digest objects containing parent/child secret material. The result serializer wipes its local arrays, but `Obj::try_from(bytes.as_ref())` makes a MicroPython bytes copy before IPC, and the SDK documentation acknowledges that its IPC receive buffer and move copies are not wiped. This is residual-secret exposure after the request, not a demonstrated cross-app read. **Fix:** minimize immutable secret copies, use wipeable buffers throughout the bridge where feasible, and clear the IPC receive storage after extracting the key; document any unavoidable residuals in the threat model.

4. **Should-fix — production app build options do not fail closed against debug/dev features.**  
   `sdk/crates/modular-xtask/src/args.rs:255–299`, `sdk/apps/zcash/src/main.rs:75–102`, `sdk/apps/zcash/src/sign_pczt.rs:288–300`  
   Normal `--production` feature resolution omits `dev_keys`, and normal non-debug builds omit diagnostics. But arbitrary `--features` are appended afterward, so `--production --features debug,dev_keys` is accepted by this resolver. That can put the diagnostics handler and chunk counters into a purported production app; whether `dev_keys` changes the signed app artifact also needs image-level verification. **Fix:** reject `debug`, `dev_keys`, and test-only features with `--production`, and check the final artifact’s feature manifest in release CI.

5. **Should-fix — an untrusted app can break Core’s progress bridge with a valid payload under the wrong operation ID.**  
   `core/src/apps/extapp/run.py:445–482`, `sdk/crates/trezor-app-sdk/src/structs.rs:1017–1019`  
   `access_progress_request` validates the archive but does not bind its enum variant to the IPC `message_id`. For example, an app can send a valid serialized `End` with the `Init` ID. Core then treats the decoded value as a four-field tuple; the resulting assertion/type failure is outside the bridge’s `ValueError` handler. This is an availability/cleanup issue for Core and other apps, not a Zcash signature bypass. **Fix:** validate variant-to-ID equality as the Crypto bridge does, and handle all progress-processing exceptions by stopping the offending app cleanly.

6. **Should-fix — RNG writes assume alignment that the SDK’s byte-slice API does not require.**  
   `core/embed/sys/rng/stm32/rng.c:67–79`, `sdk/crates/trezor-app-sdk/src/crypto.rs:337–340`  
   The exported RNG accepts `&mut [u8]`, including an unaligned subslice, while the STM32 implementation casts its pointer to `uint32_t*` and writes words. An app passing such a slice may fault or encounter undefined C behavior. The Zcash app’s current stack arrays may happen to be aligned; the public API does not guarantee it. **Fix:** make the RNG implementation byte-safe for arbitrary destinations, or enforce/check alignment and use an aligned temporary.

### What the source review supports

- **Streaming/signing:** each requested chunk is checked against a fresh 16-byte transfer ID, exact offset, and exact length (`sign_pczt.rs:97–116,254–285`). A wrong/replayed ack is rejected; a mid-stream cancel returns an error. The session is local to one signing call, resets on feed errors, and binds review/approval to the finished byte stream and signing context (`ironwood/src/session.rs:401–412,637–705,718–746`). The app confirms payment outputs as encountered, then totals before `approve` and `sign` (`sign_pczt.rs:118–161`). I found no source-level path for a malicious host to substitute unreviewed PCZT bytes after approval.
- **Entitlement/build guards:** the ZIP-32 service checks the `zip32-orchard` curve, hardened account path, allowed coin type, and declared path schema before derivation (`zip32_orchard.py:83–100`). The dev root key addition is excluded when `PRODUCTION` is defined (`root_packet.c:38–40`), and the model build script defines it for production (`core/embed/models/build.rs:137–139`). That is a **source/configuration conclusion**, not a built-image verification. Diagnostics are feature-gated; `run.py` logging additions are inside `__debug__` blocks.
- **Limits of verification:** the requested `docs/common/zcash-ironwood-signing.md` is absent in this checkout. I reviewed the checked-in Ironwood session/streaming code and the app integration, but did not exhaustively re-prove the cryptographic implementation or validate the reported 32-action stack/heap measurements. I also did not verify the actual firmware/app images, linker output, production flags, or hardware behavior.
