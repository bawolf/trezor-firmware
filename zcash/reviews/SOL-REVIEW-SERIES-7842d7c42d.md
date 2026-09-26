# GPT-6 Sol adversarial review of the extapp series (7842d7c42d)

Model: `gpt-6-sol` via Codex CLI 0.157.0, read-only; 125,379 tokens. Range `zcash/extapp..7842d7c42d` (23 commits).

## Verdict: **Do not ship the platform as reviewed.**

I found **no new path in the Zcash signer to a signature over unreviewed transaction effects**. The app constrains `address_n` to a three-component hardened ZIP-32 path for coin type **133 or 1**, accounts **0–100**; Core independently requires that path to match the app’s declared patterns. The scanner enforces a bounded, exact-length PCZT grammar, rejects transparent inputs and unsupported bundles, verifies actions and amounts, and releases signatures only after output review and totals approval. Chunk replies must echo the 16-byte transfer ID and exact offset and length. The response contains ascending action indices and 64-byte RedPallas signatures.

**Findings**

1. **Must-fix — an extapp can sign outside its declared paths through the existing typed-hash service.** `core/src/apps/extapp/run.py:285–299, 628–675`  
   **Scenario:** An app declaring only ZIP-32 Orchard paths calls `SignTypedHash` with Ethereum definition fields. Core takes the `keychain is None` branch, constructs a secp256k1 keychain from Ethereum schemas rather than the app’s declared `schemas`, and signs the supplied hash without a confirmation screen. This is inherited from the WIP base, not a regression in the Zcash app, but remains a platform release blocker.  
   **Fix:** Require every crypto signing operation to match the calling image’s declared curve and path schemas; do not substitute Ethereum schemas based solely on app-supplied fields. Add a negative test using an Orchard-only app.

2. **Should-fix — “production” does not exclude debug code.** `sdk/crates/modular-xtask/src/args.rs:237–249`  
   **Scenario:** `--production --debug` resolves the `debug` feature and uses the debug profile while skipping dev proofs. The production flag therefore does not guarantee that debug-only code is absent. I found no Zcash runtime use of its empty `dev_keys` feature, and the normal `--production` route omits it.  
   **Fix:** Reject incompatible production/debug or test-feature combinations and assert the resolved production feature set before publishing an artifact.

3. **Should-fix — spending-key consent is bound only to a 32-bit instance ID.** `core/src/apps/extapp/zip32_orchard.py:69–73, 122–144`; `core/src/apps/extapp/load.py:130–134`  
   **Scenario:** If a later, different app receives a colliding instance ID in the same cache lifetime, it can reuse the earlier account approval without the hold-to-confirm screen, provided its manifest permits that Orchard path. This is probabilistic, not a practical one-shot attack.  
   **Fix:** Bind approval to the verified app identity or image hash as well as the account, and use a larger unpredictable instance token.

4. **Should-fix — malformed declared path text escapes the intended error path.** `core/src/apps/extapp/run.py:73–85`  
   **Scenario:** A loaded app with a path string lacking two `/` components makes `pattern.split("/")[2]` raise `IndexError` before Core reaches its `DataError` handling. That fails the wire handler rather than cleanly rejecting and stopping the app. I did **not** establish a firmware-wide crash from this.  
   **Fix:** Validate component count and empty components before indexing; reject malformed manifests at image load where possible.

**Verified by read-only inspection:** the 23-commit range; scanner/session checks, approval token and single-use signing; payment, memo, transparent-output and totals screens; Core’s spending-key consent and weak-backup path; key-wiping wrappers; chunk protocol; checked IPC request access, reply alignment, heap and inbox handling; and production feature selection. The Zcash app does not accept a host-selected path outside its two declared coin types or account limit.

**Not verified:** I did not build, run tests, fuzz malformed IPC/PCZTs, inspect device UI rendering, or independently recompute NU6.3 sighashes against an external implementation. Read-only constraints were observed.
