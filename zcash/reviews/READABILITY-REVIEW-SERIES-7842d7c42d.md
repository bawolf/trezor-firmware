# Readability review: `zcash/extapp..7842d7c42d` (23 commits)

- Reviewer: Claude Opus 5.5, exact model ID `claude-opus-5-5[1m]`. This is **not** a Fable run.
- Two sub-reviews ran as in-session subagents on the same model (`claude-opus-5-5[1m]`, both self-reported):
  - `sdk/apps/zcash/signer-tests/` (Rust);
  - `sdk/apps/zcash/tests/` plus the Python tooling files (pyproject, uv.lock, ui_tests).
- The coordinator reviewed:
  - every commit message;
  - all 20 platform commits, diff by diff;
  - the signer crate `sdk/apps/zcash/signer/src/*` (every file);
  - the app `sdk/apps/zcash/src/*`, `protob/`, manifests, README and translations.
- The coordinator re-checked these subagent claims against the tree:
  - the `#[ignore]` test (`bundle.rs:147-148`);
  - "the fork" (`actions.rs:362`);
  - `Policy as Refused`;
  - the dead constants in `zcash_ext.py:42-44`;
  - the `MNEMONIC_ABANDON` comment.
- Repo: `/Users/bryantwolf/conductor/workspaces/trezor-firmware/series`, HEAD `7842d7c42ddc190c252fbb5e3fc7f7183cfd0015`, clean tree.
- What ran:
  - `git log/show/diff/grep`, `diff`/`cmp` against `sdk/apps/{ethereum,tron}`, and awk line counts;
  - a `grep` over the local crates.io registry copies of `orchard-0.15.3` and `orchard-0.15.5`;
  - `git log` over `remotes/upstream/{bieleluk,cepetr,vojczejk}/*`;
  - one `rustfmt --check` by the tests subagent, which writes nothing.
- Nothing was built, tested, vetted, edited, committed or pushed. `cargo vet` was **not** run, so the vet finding below comes from reading `sdk/apps/supply-chain/`.

Severity key: **MUST** = fix before showing Trezor, **SHOULD** = should fix, **NIT** = nit. Paths are relative to the repo root unless noted. Line numbers are at HEAD.

---

## Verdict

**Mostly yes now, with nine specific exceptions.** This is a different series from the one reviewed in P3.

- **Platform commits.** The 20 platform commits read like Trezor's own fixes:
  - correct scopes;
  - `[no changelog]` everywhere;
  - short "why" bodies with no sessions, forks or timings;
  - tests at the right layer (`core/tests/test_apps.extapp.{run,ipc,zip32_orchard}.py`, inline Rust tests in `structs.rs`/`ui.rs`/`metadata.rs`);
  - `raise ValueError  # comment` in the style of `exceptions.md`.
- **Signer crate.** It now reads like a small, purpose-built Trezor crate:
  - one `Error` enum;
  - one `Network`;
  - no reference engine;
  - no `§N`;
  - no FFI residue;
  - comments are present-tense "why", at 11.6% density (core/embed/crypto is 15.5%).
- **The app.** It follows Ethereum/Tron's shape:
  - `build.rs` is identical except for its proto list;
  - `common.proto`, `pb2py` and `messages.py.mako` are copies;
  - `ui_tests/` is Tron's tree;
  - the protos are `hw.trezor.zcash` with unprefixed names and `address_n`.

What would still give it away, or draw a first-round objection:

1. **Dependency provenance is misdescribed.** The manifests and commit say the orchard fork exists because "Orchard with Ironwood support is not released yet". Released `orchard` 0.15.3 and 0.15.5 already have Ironwood. The fork is there for APIs that exist nowhere upstream (`*_with_progress`, `ScopeClassifier`, `recover_output_bound_with_*`, `wipe`). The new crates also have no `cargo vet` entries, and the separate `signer-tests` workspace is outside fmt, clippy, vet and CI.
2. **Overlap with the owners' in-flight work.**
   - `020122b2a2` has the same subject as bieleluk's `0be72a42dd` on `upstream/bieleluk/stabby`, with a different design.
   - `ui::Progress` overlaps vojczejk's `9ee009365b` "add library-owned progress to modui" (2026-09-25).
3. **A few leftovers:**
   - an `#[ignore]`d test that documents a known deviation;
   - "the fork" in a test doc;
   - dead wallet-format constants in `zcash_ext.py`;
   - `vendor = "Bryant Wolf"` on `id = "zcash.trezor.com"`;
   - two commits that refer to code arriving only in a later commit;
   - Zcash-prefixed Core translation strings that land in bitcoin-only builds.

---

## Must-fix list

| # | Where | Exact quote | Fix |
|---|---|---|---|
| M1 | `sdk/apps/Cargo.toml:11`, `sdk/apps/zcash/signer-tests/Cargo.toml:34`, commit `3d69ea103d` body | `# Orchard with Ironwood support is not released yet.` / "Orchard with Ironwood support is not released yet, so orchard and sinsemilla are patched to forks." | **Factually wrong.** The registry copies of `orchard-0.15.3` (`src/lib.rs:126-127` `/// The Ironwood value pool. Ironwood,`) and `orchard-0.15.5` (`IronwoodDomain`, `BundleVersion::ironwood_v3`, `cross_address_enabled`) already ship Ironwood. The signer calls APIs that 0.15.5 does not have: `ScopeClassifier`/`scope_classifier_with_progress`, `recover_output_bound_with_{pkd_esk,ock,ovk}`, `address_with_progress`, `SpendingKey::from_bytes_with_progress`, `Spend::parse_with_progress`, `verify_{nullifier,note_commitment}_with_progress`, `Note::from_parts_with_progress` and `ScopeClassifier::wipe`. Say so in one line, e.g. `# Forks that add progress callbacks (for Core's 1 s IPC limit), a cached ivk classifier and bound output recovery; proposed upstream as <link>.` Say the same in the commit body. List the `github.com/bawolf/*` git dependencies in the PR description as a merge blocker. |
| M2 | commit `020122b2a2` "feat(sdk): let an app declare its IPC inbox size in its manifest" | (whole commit) | `upstream/bieleluk/stabby` has `0be72a42dd` by Lukas Bielesch (2026-09-15), subject "feat(extapp): let an app declare its IPC inbox size in its manifest", with a different design: an `ipc_buffer_size` header field, `app_get_ipc_buffer_size` syscall, and an inbox allocated from the heap after `allocator::init`. Ours passes `TREZOR_APP_IPC_BUFFER_SIZE` at compile time. Showing a second implementation under the owner's own subject is the "duplicate PR" pet peeve. Drop `020122b2a2` and state the dependency on his commit; the manifest key `ipc-buffer-size = 2048` is the same. Or offer ours as a comment on his branch. Rebase `52ddf0d4f0`'s last paragraph accordingly. |
| M3 | `sdk/apps/supply-chain/` (unchanged); `sdk/apps/Cargo.lock` (+647 lines); `sdk/apps/zcash/signer-tests/Cargo.lock` (1,507 lines); `Makefile:331-333` `extapp_vet: cd sdk/apps ; cargo vet --locked` | `exclude = ["zcash/signer-tests"]` | Not run, but by inspection `supply-chain/config.toml` has no entries for `orchard`, `pasta_curves`, `sinsemilla` or their dependency tree, so `make extapp_vet` cannot pass. `signer-tests` is outside `xtask modular fmt`, `extapp_clippy`, `extapp_vet` and `.github/workflows/sdk.yml` (matrix `app: [tron]`); only the README's manual `cargo test --manifest-path` runs it. Add the vet exemptions/audits in the commit that adds the dependencies. Add a Makefile target and an `sdk.yml` job for `signer-tests` (test, fmt --check, clippy), or say plainly in the PR that it is not wired yet. |
| M4 | `core/translations/en.json:3583-3584`; `core/embed/rust/librust_qstr.h` (qstrs placed before `#if !BITCOIN_ONLY`); `common/tools/cointool.py:148-162` `ALTCOIN_PREFIXES` | `"zcash__spending_key": "Spending key"`, `"zcash__spending_key_template": "{0} will receive the spending key of Zcash account #{1} ({2}) and can spend all of its funds."` | `zcash` is not in `ALTCOIN_PREFIXES`, so these qstrs are generated into the always-built section, i.e. bitcoin-only firmware. Reviewers reject btc-only leakage on sight (PR #7947 "let's not"). The consent screen belongs to the platform's key service, not a coin app, so rename the keys to `extapp__…`. Or add `zcash` to `ALTCOIN_PREFIXES` and check that the key service itself is excluded from btc-only. |
| M5 | `sdk/apps/zcash/signer-tests/tests/bundle.rs:145-148`; `tests/mutations.rs:9-11` | `#[ignore = "the Session admits binding keys and a Sapling anchor that upstream refuses"]` / "The binding keys are skipped: the Session does not parse them (`bundle.rs`, `test_unused_fields_parse`)." | An ignored test naming a divergence from upstream reads as a known bug shipped with the PR. Either validate these fields in `Session` and un-ignore the test, or delete it. In the latter case, add one line to the `Session`/scanner docs ("`bsk` and the Sapling anchor are outside the sighash and unused by signing, so they are skipped") and shorten the `mutations.rs` sentence to match. |
| M6 | `sdk/apps/zcash/signer-tests/tests/actions.rs:362` | `/// \`ask\` as a scalar: the fork encodes a signing key as its scalar, and` | "the fork" is history the maintainers never saw. `/// \`ask\` as a scalar: a signing key encodes as its scalar, and randomizing by zero leaves it unchanged.` |
| M7 | `sdk/apps/zcash/tests/zcash_ext.py:42-44` | `# A wallet stores each signature as a record: pool u8 \| action_index u8 \| signature[64].` / `POOL_IRONWOOD = 0x03` / `RECORD_BYTES = 66` | Unused anywhere, and describes an external wallet's storage format that is not in the tree. Delete all three lines. |
| M8 | commits `4ba6e777d8` and `3d69ea103d` (self-containment) | In `4ba6e777d8`'s `sdk/apps/zcash/Cargo.toml:12-17`: `# Signing keeps ~30 KiB of Pallas and Orchard caches and needs a deep stack to verify an action.` and `# The largest message the app receives is a PcztAck with a 1 KiB chunk.` | These refer to signing and `PcztAck`, which arrive only in `7842d7c42d`. Declare address-only sizes in `4ba6e777d8` and raise `stack-size`/`heap-size`/`ipc-buffer-size` (with these comments) in `7842d7c42d`. Similarly, `3d69ea103d` adds `signer-tests/src/fixtures.rs`, which writes to `sdk/apps/zcash/tests/fixtures` before the app exists. Move it, with `pub mod fixtures;`, `serde` and the `zcash_address` dev-dependency, to `4ba6e777d8`. Move the `sapling`/`transparent` dev-dependencies (used only by `fixtures_sign.rs`) to `7842d7c42d`. |
| M9 | `sdk/apps/zcash/Cargo.toml:9-11` | `id = "zcash.trezor.com"` … `vendor = "Bryant Wolf"` | Still unresolved from P3. A `trezor.com` id asserts SatoshiLabs ownership, while the vendor names an individual; Ethereum and Tron have `vendor = "SatoshiLabs"`. Pick one (`vendor = "SatoshiLabs"` if the app is offered to Trezor to own, or a non-trezor.com id), and say which in the PR. |

---

## Previous review's must-fixes (P3), status

| P3 must-fix | Status | Evidence at `7842d7c42d` |
|---|---|---|
| Series: drop the merge `5fd51d24a3` | Resolved | Linear 23 commits, no merges |
| Series: keep the test-device (`ad44c809e8`, `d485d458c5`) and diagnostics (`863bf79dfd`, `1c0a782e01`, `9b4743420b`) commits out | Resolved | No `diagnostic`, root-key or secmon changes in the range |
| Series: fold the ironwood progress commits and the stack fix into their feature commits | Resolved | Three Zcash commits: signer crate, app, signing |
| Scopes (`sdk`, `xtask`, `extapp`, `core`), not `core` everywhere | Resolved | `core`, `sdk`, `xtask`, `extapp`, `core,xtask`. See the table below for two NITs. |
| `[no changelog]` on every commit | Resolved | All 23 end with `[no changelog]` |
| History in bodies (Safe 5/7 sessions, timings, `bawolf/…@hash`, "not yet pushed") | Resolved | None in any body; the fork is described (inaccurately, M1) |
| `§N` design-document references (ironwood `lib.rs:6-8`, ~25 sites; `sign_pczt.rs`; tests) | Resolved | `grep '§'` over the range: none |
| FFI/firmware vocabulary (`session_begin`/`session_feed`, "future trusted UI", "application adapter", DEDUP LEVER 3, CAP-sized, stock SDK, the series, the spike) | Resolved | None in `signer/src`, `src`, tests |
| Manifest measurement/history comments (`Cargo.toml:12-17, 72-74, 89-90, 112-118`) | Resolved | One-line "why"s now; `computed-generators` is wired to `model_t3t1` |
| `sign_pczt.rs:165-168` stack-overflow narration | Resolved | `sign_pczt.rs:140-141` "out of line so that their frames are not on the stack while the session verifies an action." |
| Debug diagnostics on the release wire schema | Resolved | `zcash.proto` has 9 messages, no diagnostics field |
| Reference signer (`Engine`/`validate`/`verify_bundle`/`wire::scan`/`effects`) interleaved in `lib.rs`; production docs linking to it | Resolved | `signer/src/lib.rs` is 278 lines of production API; no `Engine` anywhere |
| Seed-based derivation and `seed_fingerprint` in the crate | Resolved | `keys.rs` has only `external_receiver(fvk, …)`; Core computes the fingerprint |
| `Hedged::from_seed` fed the spending key | Resolved | `Hedged::new(inner, key)`, `hedge.rs:30-48`, 11-line module doc |
| `account.rs:94-104` safe generic `wipe<T>` (unsound) | Resolved | `src/keys.rs:32-71`: `unsafe trait FlatKey` + `zeroize::zeroize_flat_type` with per-type `// SAFETY:` |
| `#[path = "../src/…"]` test includes; `cfg_attr(not(test), no_std)` | Resolved | `signer/src/lib.rs:9`; inline `mod tests` in every module; no `#[path]` |
| Dead `common/mod.rs` leftovers (`Signing`, `sign_streamed`, `CHUNK_BYTES`, `build_wide_deshield`, region_budget/8→32 comments) | Resolved | Gone (tests subagent: every `pub fn` in `signer-tests/src/lib.rs` has a caller) |
| Tests built only on the reference signer (`conformance.rs`, `action_bounds`, `randomness`, `request_disposal.rs`, `seed_fingerprint.rs`) | Resolved | All tests drive `zcash_signer::Session` and check against upstream `pczt` `Verifier`/`Signer` |
| History/private references in tests (Design §7, "profile 1", "the spike", "the adapter", stale "nine actions", stale `lib.rs:783-790`) | **Partly** | All gone except `tests/actions.rs:362` "the fork" (M6) |
| Personal `bawolf/{orchard,sinsemilla}` patches: disclose; one-line comment | **Partly** | One-line comment exists but is wrong (M1); no vet coverage (M3) |
| `ironwood-pasta-curves`/`ironwood-sinsemilla` package aliases | Resolved | Plain `pasta_curves`/`sinsemilla` (`signer/Cargo.toml:15,18`) |
| Python: `zcash_ext.py` "mirrors trezorlib's … firmware series (bawolf/…)", "helpers of the series' trezorlib" | Resolved | Tests subagent grep: none |
| Python: `test_sign_pczt.py` "firmware series' vectors", "Stands in for the series' region test", "which the series had in Core", "(design §7)" | Resolved | None |
| Python: diagnostics tests and shim | Resolved | None |
| Test-device-only files (`mldsa44.h`, `kernel.ld`, `root_packet.c`) | Resolved | Not in the range |
| Changelog: `c549ab5f18`/`91fc95c0ac` lacked `[no changelog]` | Resolved | Their successors `2bec506689`/`3273d5681d` carry it |

Previous SHOULDs that are still open or only partly done: `vendor`/`id` mismatch (now M9); `computed-generators` wiring (resolved); `structs.rs` duplicate id table (now `ArchivedTrezorCryptoEnum::id`; a second table remains, but the test at `structs.rs` `crypto_request_must_match_the_operation_id` pins them together, so acceptable); `// SAFETY:` on touched `unsafe` blocks (partly, see S14); `hw.trezor.zcash` package, unprefixed names, `address_n`, `repeated SpendAuthSignature`, `ButtonRequestType::PublicKey` (all resolved); `end_request_progress` wrapper (resolved, inlined); `slip44_id` naming (still the name, now a real value again; acceptable); `zip32_orchard.py` `_ENTITLEMENT`, the double seed check, the unreachable hardened check and the 15/21-word refusal (all resolved).

---

## Commit messages: conventions check

Subjects: 36-67 characters (median 56), lowercase imperative, no period. Every body ends with `[no changelog]`, and no body mentions devices, sessions, forks' history or earlier designs.

| sha | Subject | Scope | Body | Notes |
|---|---|---|---|---|
| e0bbd36a70 | fix(core): guard extapp run.py logging with __debug__ | OK | 6 lines, why | OK |
| 32531d2342 | fix(core): report a stopped extapp instead of a timeout | OK | 5 lines | OK |
| 367455203e | fix(core): allow extapp path patterns with different coin types | OK | 6 lines | OK; adds `core/tests/test_apps.extapp.run.py` |
| e65622262f | fix(sdk): back the global allocator with the app heap | OK | 7 lines | OK. Targets the runtime that `stabby`'s `c328a4bdb7`/`app_runtime2` replaces; expect a rebase. |
| f122e92a7b | fix(extapp): give the ethereum app the heap its tests need | OK | 3 lines | OK |
| dfbb8392d9 | feat(core,xtask): give an emulator app only the heap it declares | OK | 10 lines | NIT: arguably `fix`. The body is good ("Emulator images built before this change … must be rebuilt"). |
| 70be287a0f | fix(extapp): show the ethereum data progress before reporting it | OK | 3 lines | OK |
| 7b56a3384e | fix(sdk): track whether a progress screen is shown | OK | 6 lines | OK |
| e7ab6d223e | feat(sdk): add ui::Progress with a keep-alive | OK | 5 lines | SHOULD: overlaps vojczejk `9ee009365b` (S2) |
| 78c8bac166 | feat(sdk): add checked accessors for extapp IPC requests | OK | 8 lines | SHOULD: `rkyv/bytecheck` is now compiled into Core firmware (the `core/embed/Cargo.lock` +33 lines are in `3273d5681d`). State the flash delta; reviewers quote KB. |
| 3273d5681d | fix(core): validate extapp IPC requests before reading them | OK | 12 lines | NIT: trim to the first paragraph plus one sentence |
| 8df4fd2825 | fix(core): raise extapp bridge errors instead of unwrapping | OK | 8 lines | NIT: the `min(value, 1000)` clamp is a separate fix bundled in (it is mentioned); either split it or leave it |
| 020122b2a2 | feat(sdk): let an app declare its IPC inbox size in its manifest | NIT: touches `modular-xtask` too; `dfbb8392d9` used `core,xtask`, so `sdk,xtask` is consistent | 8 lines | **M2** (duplicate of bieleluk `0be72a42dd`) |
| 52ddf0d4f0 | fix(core): stop an extapp whose host message does not fit its inbox | OK | 9 lines | Depends on `020122b2a2`; reword after M2 |
| 421a5f600d | fix(core): let rng_fill_buffer fill unaligned buffers | OK | 4 lines | OK |
| df8953aafd | feat(core): allow extapps to use rng_fill_buffer | NIT: also changes `sdk/crates/trezor-app-sdk` (4 files); `core,sdk` | bullets | OK |
| 61b5009529 | fix(xtask): name translation-style in its error message | OK | none | OK |
| f6afc7a253 | feat(sdk): check that reply types fit the device IPC alignment | OK | 5 lines | OK |
| b46a40d286 | feat(sdk): add crypto::get_zip32_orchard_account | OK | 6 lines | OK |
| 2bec506689 | feat(core): derive ZIP-32 Orchard account keys for extapps | OK | 12 lines | NIT: long. The pseudo-curve design choice belongs in the issue or PR text; **M4** for the strings |
| 3d69ea103d | feat(extapp): add zcash signer crate | OK | 6 lines | **M1** (fork reason wrong), **M8** (`fixtures.rs` placement) |
| 4ba6e777d8 | feat(extapp): add zcash app with address and viewing key | OK | 3 lines | **M8** (manifest refers to signing) |
| 7842d7c42d | feat(extapp): sign PCZTs in the zcash app | OK | 3 lines | OK |

---

## Per-file findings (beyond the must-fix list)

### Core Python and Core Rust

| Sev | Where | Quote | Fix |
|---|---|---|---|
| NIT | `core/src/apps/extapp/run.py:317, 340` | `raise ValueError  # an address MAC binds one coin type` | This uses an exception for control flow inside `try/except Exception: result = False`. `if slip44_id is None: result = False` before the `try` reads more directly. |
| NIT | `core/src/apps/extapp/run.py:173-177` (after `8df4fd2825`) | `except Exception as e: … log.error(__name__, f"Invalid UI request: {e}")` | Since `8df4fd2825` widened the catch to `Exception`, this also covers allocation failure, so "Invalid UI request" can be wrong. Use `"Failed to process UI request"`, or catch `ValueError` separately. |
| NIT | `core/src/apps/extapp/zip32_orchard.py:437-446` vs app screens | `"… Zcash account #{1} ({2}) …"` | Core names the account "Zcash account #1 (Mainnet)", while the app uses "ZEC #1" (`src/helpers.rs:31-33`) and "Zcash Mainnet ZEC #1" (`sign_pczt.rs:348`). Pick one label. |
| NIT | `core/tests/test_apps.extapp.zip32_orchard.py:28-29, 36-37` | `# … (as vendored in orchard 0.15.5 src/test_vectors/zip32.rs)`, `equals orchard 0.15.5 SpendingKey::from_zip32_seed` | The signer pins `orchard = "=0.15.3"`. Cite the same version, or say the vectors are version-independent. |
| NIT | `core/embed/rust/src/crypto/api/firmware_micropython.rs:218` | `// SAFETY: \`result\` is not modified while this function runs.` | Good. The neighbouring touched `unsafe { get_buffer(obj) }?` and `unsafe { list.as_slice() }` lines still have none; they match the file, so leave them. |

### SDK and xtask

| Sev | Where | Quote | Fix |
|---|---|---|---|
| SHOULD | `sdk/crates/trezor-app-sdk/src/ui.rs` `Progress` doc (≈16 lines) and `KEEP_ALIVE_MS` doc | "The longest silence is then `KEEP_ALIVE_MS` plus the longest stretch between two calls." / "It leaves 900 ms of Core's 1 s limit for the longest stretch between two calls, and costs at most ten reports a second." | The same point is made three times (struct doc, const doc, `development.md` "Long computations"). Keep it in the development guide. Cut the struct doc to the 1 s rule plus the example, and the const doc to one line. |
| NIT | `sdk/crates/trezor-app-sdk/src/structs.rs` `ArchivedTrezorCryptoEnum::id -> u8` vs `ArchivedTrezorProgressEnum::id -> u16` | — | This mirrors the non-archived `id()` types, so it is fine. The crypto test builds bytes with `archive()` and the others call `rkyv::to_bytes` directly; use one helper. |
| NIT | `sdk/doc/development.md:53` | "Heap use on the 64-bit emulator is close to the device's, so emulator tests are a fair measure: declare the measured need plus a margin." | This is an unsupported empirical claim. Use "Measure the app's peak heap on the emulator and declare it with a margin." |
| NIT | `sdk/crates/modular-xtask/src/metadata.rs` `IPC_BUFFER_DEFAULT_SIZE` + `sdk/crates/trezor-app-sdk/src/lib.rs` `None => 16384` | "as in the SDK's `IPC_BUFFER_SIZE`" | These are two defaults to keep in sync. This goes away under M2. |
| NIT | `sdk/crates/trezor-app-sdk/src/crypto.rs` (`b46a40d286`) | `// TODO: proper error type` | It copies the file's existing convention, so leave it, but it is a TODO in new code. |

### Signer crate `sdk/apps/zcash/signer`

| Sev | Where | Quote | Fix |
|---|---|---|---|
| SHOULD | `signer/src/ff1.rs` (whole file, 1.2% comments) | (no module doc) | This is a hand-written FF1-AES256 where orchard uses the `fpe` crate. The only reason appears in `prewarm.rs:43-44` ("`address_at` would link FF1 from `fpe`"). Add `//! FF1-AES256 for Orchard diversifiers (radix 2, 88 bits, no tweak), in place of the \`fpe\` crate that \`address_at\` would link.` Give `test_orchard_ff1_vectors` a `// source:` line. |
| SHOULD | `signer/src/hedge.rs:18`, `session.rs:36`, `session.rs:527` | `b"TrezorIrnwdNonce"`, `b"IWStreamBytesV1"`, `b"IWApprovalV1"` | Codename abbreviations. The two session personalizations are internal (the token never leaves the device), so rename them freely (`b"TrezorZcashStream"`…). The hedge constant changes pinned vectors; rename it or leave it and justify it once. |
| NIT | `signer/src/scanner.rs:171` | `/// Checked using upstream's versioned flag implementation by the caller.` | "upstream" is our vocabulary. `/// Checked by the caller with orchard's \`Flags\`.` |
| NIT | `signer/src/scanner.rs:38-80` | `pub const HEADER_BUDGET`, `…_BUDGET` | These are `pub` in a private module, while `limits.rs` uses `pub(crate)`. Use `pub(crate)`. P3 suggested `MAX_*_BYTES`; "budget" is acceptable if used consistently. |
| NIT | `signer/src/prewarm.rs:15` | `/// A public key and note seed used only to fill the caches.` | `WARM_KEY` is a spending key, and "public" means "publicly known". `/// A publicly known spending key and note seed, used only to fill the caches.` |
| NIT | `signer/src/lib.rs:128-133` | `pub enum OutputKind` (no doc) | One line: `/// Whether an output pays someone else or returns change to the account.` |
| OK | `session.rs`, `memo.rs`, `sighash.rs`, `scanner.rs` | e.g. `sighash.rs:26` "Private in zcash_primitives::transaction::txid; retyped here." | Terse, present-tense "why"s. Nothing to change. |

### App `sdk/apps/zcash`

| Sev | Where | Quote | Fix |
|---|---|---|---|
| SHOULD | `src/helpers.rs:23-28` | `Network::Mainnet => "Mainnet", Network::Testnet => "Testnet",` | These are English literals on screens, while the same series adds `words__mainnet`/`words__testnet` to Core. Put them in `translations/en.json` and use `tr!`. |
| NIT | `sdk/apps/zcash/Cargo.toml:45` | `# The Safe 5's app arena has no room for the Sinsemilla generator table.` | Trezor code names models by id, and T3T1 has an app arena only on cepetr's WIP `534e35daa9`. `# T3T1's app arena has no room for the Sinsemilla generator table.` |
| NIT | `src/paths.rs:23-24` | `let account = account ^ HARDENED; if purpose != PURPOSE \| HARDENED \|\| account > MAX_ACCOUNT` | The XOR relies on the range check to reject non-hardened indices. `account & HARDENED == 0` as an explicit check reads directly. |
| NIT | `src/sign_pczt.rs:186-188` | `if signatures.records().is_empty() { return Err(Error::DataError("Zcash signing failed")); }` | `review_stream` already refuses a PCZT without a real spend. Drop this, or comment it as a defensive check. |
| NIT | `src/proto.rs:17-41` | `// Optionally, assert it's within u16 if that's your requirement:` | A verbatim copy of Ethereum's test. Leave it for template parity. |
| OK | `src/keys.rs` (27.9% comments) | `// SAFETY: two Pallas field elements and a RedPallas verification key …` | The density comes from required `// SAFETY:` lines. It is fine. |
| OK | `protob/zcash.proto` | `// prost reads an absent required field as 0, which both refuse.` | Tron-style one-liners. Fine. NIT: `messages.proto` could carry a `// Zcash` group comment as Ethereum's does. |

### Rust tests `signer-tests` (subagent, spot-checked)

These must-fixes are listed above: M3 (tooling and vet), M5 (`#[ignore]`), M6 ("the fork") and M8 (`fixtures.rs` placement). Should-fixes:

- **S-T1 Duplicated helpers**, each to go into `src/lib.rs` once:
  - `fn payment` (`actions.rs:20`, `outputs.rs:10`, inlined at `memos.rs:163`);
  - the sighash closure (`actions.rs:413-417`, `approval.rs:170-174`);
  - the memo BLAKE2b (`memos.rs:95-97`, `fixtures_sign.rs:148-150`);
  - the ZIP-317 fee (`fixtures_sign.rs:89`, `signing.rs:28, 123`);
  - `two_notes()` re-inlined three times;
  - `session.begin(len, &wallet.fvk, &wallet.seed_fingerprint)` ×8;
  - `sign_using` (`signing.rs:206`) duplicating `lib::sign_with`.
- **S-T2** `use zcash_signer::Error::{Malformed, Policy as Refused};` in 7 files. No variant is called `Refused`; use `Policy`.
- **S-T3** Large table tests: `actions.rs:34-201` (40 cases), `memos.rs:18-69` (32), `bundle.rs:19-113` (18), `transparent.rs:151-236` (16). Split them by concern (PR #7911: "create multiple tests and have the vectors separated").
- **S-T4** "sdk" meaning librustzcash: `signing.rs:93` `/// What the librustzcash SDK hands over:` and `fixtures_sign.rs:374` `let sdk = …` / vector `"view_sdk"`. In this repo `sdk/` is Trezor's app SDK. Use `zcash_client_backend` and `view_wallet`.
- **S-T5** `lib.rs:45-47` `/// The app's limits.` duplicates `src/sign_pczt.rs:37-38` silently, and `header.rs`/`bundle.rs` use a literal `100` for `EXPIRY_WINDOW`.
- **S-T6** `blake2b_simd` is used only by `tests/`; make it a dev-dependency.

### Python tests `sdk/apps/zcash/tests` (subagent, spot-checked)

Must-fix M7 is listed above. Should-fixes:

- **S-P1 Test mnemonic.** `common.py:90-91` has `# The wallet of the fixtures; its 12 words get the ZIP-315 weak-backup warning.` / `MNEMONIC_ABANDON = " ".join(["abandon"] * 11 + ["about"])`, plus `pytestmark = pytest.mark.setup_client(mnemonic=MNEMONIC_ABANDON)` in all three test files. Every Ethereum and Tron fixture uses `all ×12` (the conftest default), and that 12-word seed would get the same warning. Regenerate the fixtures from `signer-tests` on the all-all seed; the Rust `fixtures.rs:12` changes with them.
- **S-P2 Error vectors in the success test.** `test_sign_pczt.py:78-81` does `if "error" in result: with pytest.raises(...) … return`. Ethereum separates these (`sign_tx_error.json` + `test_signtx_error`). Do the same.
- **S-P3 `zcash_ext.py:145-264` carries wallet-client logic.** It has `_cancel_and_fail`, `session.is_invalid = True` inside `except Exception`, `_signatures` shape checks, and `MAX_ACTIONS`/`SIGNATURE_BYTES`. Replace these with plain asserts. `SpendAuthSignature` NamedTuple (`55-63`) duplicates the generated message; return `response.signatures`. The `call_raw(session, id, message_id(msg), encode(msg))` call is repeated 5×; have `call_raw` take the message.
- **S-P4** `test_get_address.py:121-126`: `# Computed with \`orchard\` 0.15.5 and \`zcash_address\` 0.13 from this seed.` This is a hand-computed vector with no generator. Have `fixtures_keys.rs` emit it.
- **S-P5** `input_flows.py:43-45`: `"""Whether the address is shown in groups of four characters."""` / `return max(len(piece) …) % 4 == 0`. The code does not check what the docstring says. `return all(len(p) == 4 for p in pieces[:-1])`.
- **S-P6** `input_flows.py`: `get()` diverges from Ethereum's `InputFlowBase` (`input_flow_common`/`_delizia`/`_eckhart`). `assert "Zcash account" in text` should be built from `TR.zcash__spending_key_template` (or its M4 rename). `read_all_pages` reimplements `trezorlib.testing.common.get_text_possible_pagination`.
- **S-P7** `"payment_total"` in every sign fixture is never read by a device test. `TRANSPARENT_PREFIXES` (`test_sign_pczt.py:41, 71-73`) checks fixture data rather than the device.
- NITs:
  - one blank line removed from the copied `conftest.py:401` and `ui_tests/reporting/common.py:223`; restore them so `diff -r` against Tron shows only the URL and `fixtures.json`;
  - ambiguous vector ids (`at_limit`, `swapped`, `view_sdk`, `1_output`);
  - magic `65_537`, `bytes(16)`, `bytes(11)`.

---

## Comment density

Comment lines (`//`, `///`, `//!`) over non-blank lines, whole files including tests:

| Code | Non-blank | Comment | Density |
|---|---|---|---|
| **Zcash app `sdk/apps/zcash/src`** | 1,109 | 99 | **8.9%** |
| Ethereum `sdk/apps/ethereum/src` | 7,479 | 383 | 5.1% |
| Tron `sdk/apps/tron/src` | 2,146 | 101 | 4.7% |
| **Signer `sdk/apps/zcash/signer/src`** | 2,682 | 310 | **11.6%** |
| `core/embed/crypto/src` (Trezor library crate) | 2,350 | 364 | 15.5% |
| `sdk/crates/trezor-app-sdk/src` (public API, `missing_docs`) | 5,606 | 1,250 | 22.3% |
| `signer-tests` (subagent) | 3,740 | 238 | 6.4% |
| Zcash Python, non-copied (`#` + docstrings) (subagent) | 827 | 66 | 8.0% (Ethereum tests 7.8%) |

Per-file outliers in the app:
- `src/keys.rs` 27.9%, from required `// SAFETY:` lines;
- `src/sign_pczt.rs` 10.5%;
- `src/proto.rs` 13.5% and `src/unified.rs` 13.5%, both on small bases;
- everything else 2-7%.

Per-file outliers in the signer:
- `limits.rs` 53% and `error.rs` 38%, which are doc-only files;
- `ff1.rs` 1.2%, too low (see the `ff1.rs` finding);
- the rest 7-14%.

The app sits at 1.7-1.9× Ethereum/Tron, down from 2.6× in P3. That is justified by the security-relevant "why"s in `sign_pczt.rs` and `keys.rs`. The signer is below Trezor's own crypto crate.

---

## Top 10 should-fixes

1. **Coordinate `ui::Progress`** (`e7ab6d223e`) with vojczejk's `9ee009365b` "feat(sdk): add library-owned progress to modui" (2026-09-25). Say in the PR that it may be superseded, or rebase onto modui. Likewise, `e65622262f` targets the runtime that `stabby`'s `c328a4bdb7` replaces.
2. **State the flash cost** of `rkyv/bytecheck` now compiled into Core (`78c8bac166`, `core/embed/Cargo.lock` +33), and the app's size per model.
3. **De-duplicate the `signer-tests` helpers** (S-T1) and drop the `Policy as Refused` alias (S-T2).
4. **Split the large table-driven tests** by concern (S-T3; PR #7911).
5. **Move fixtures to Trezor's `all ×12` mnemonic** in both the Rust generator and the Python tests (S-P1).
6. **Slim `zcash_ext.py`** to an Ethereum/Tron-style wrapper (S-P3), and fix `is_chunked` (S-P5).
7. **Add a module doc and a vector source to `signer/src/ff1.rs`**, explaining why it replaces `fpe`.
8. **Rename the codename personalizations** `IWStreamBytesV1`/`IWApprovalV1` (and decide on `TrezorIrnwdNonce`).
9. **Translate the app's `Mainnet`/`Testnet` labels**, and use one account label across Core and app ("Zcash account #1" vs "ZEC #1").
10. **Cut the three-fold `Progress` 1 s explanation** to one place (`development.md`), and trim the 10-12-line bodies of `3273d5681d` and `2bec506689`.

---

## The three things a Trezor maintainer would most likely push back on first

1. **Dependencies.**
   - Personal git forks of `orchard` and `sinsemilla`, described as needed for Ironwood when released orchard already has it. The fork's real additions are progress, the ivk classifier, bound recovery and wipe.
   - About 150 crates with no `cargo vet` entries.
   - A second lockfile outside every Makefile/CI target.
   - `bytecheck` newly compiled into Core.

   CONVENTIONS §4 records "Quite a lot new dependencies, we might have to split reviewing them" (PR #6802) and flash-KB questions on every feature. This is the first thing matejcik or cepetr will see in `sdk/apps/Cargo.toml`.
2. **Scope and ownership.**
   - An unsolicited new-coin app plus a new Core key-export service: a pseudo-curve `zip32-orchard` in `curves`, a spending key handed to an app, 44 B of session cache for every user, and `zcash__*` strings in btc-only builds.
   - The key service is proposed without the issue or email-channel agreement that the PR template and issue #6770 ask for.
   - It goes against SDK code that exists only on WIP stacks, and duplicates or overlaps the owners' in-flight commits (bieleluk `0be72a42dd`, vojczejk `9ee009365b`, stabby's runtime rewrite).

   Expect "please split, open an issue first, and coordinate with bieleluk".
3. **Review load and reimplementation.**
   - `3d69ea103d` is 8.4k lines: 2.7k source, 4k tests and a 1.5k lockfile. `4ba6e777d8` is 7.5k, most of it copied `ui_tests`.
   - The signer hand-rolls a PCZT v2 parser (`scanner.rs`, 783 lines), a ZIP-244 sighash (`sighash.rs`) and FF1 instead of using `pczt`, `zcash_primitives` and `fpe`. The reason (streaming within the app's RAM, keeping at most one action) is stated only in two module docs.
   - Add the `#[ignore]`d divergence test (M5) and the large table tests, and a reviewer will ask why this is safer than the upstream parser. Put the RAM and flash numbers that justify it in the PR text.
