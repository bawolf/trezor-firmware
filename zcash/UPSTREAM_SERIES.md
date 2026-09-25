# The upstream-shaped commit series

The Zcash shielded (Orchard/Ironwood) work cut into twelve commits and four PRs on
`upstream/main`, the way `MERGEABILITY_REVIEW.md` Part 4 proposes.

| | |
|---|---|
| Branch | `zcash/ironwood-upstream-v2` in `bawolf/trezor-firmware`. Never pushed to `trezor/trezor-firmware`; no PR is open. |
| **Head** | **`dcc602fd48`** |
| Base | `6c38a6ab1e` (`upstream/main`, 2026-09-25) |
| Previous heads | `3223765a80`, the same series before the recipient-address change. `6b4c5f090c`, the same twelve commits on `148e530180`, tested on hardware (tag `archive/ironwood-upstream-v2-6b4c5f090c-hw`). `3e737c17ca`, the first nine-commit cut (tag `archive/ironwood-upstream-3e737c17ca`); its record is kept below from "Record of the first cut". |
| Date | 2026-09-24 |

## The twelve commits

Every message passes `docs/git/hooks/commit-msg` (12/12). Author and committer are
`Bryant Wolf <bawolf@gmail.com>`, with no `Co-authored-by:` or tool attribution.
`uv.lock` is untouched in every commit.

| # | SHA | Subject | Files | + | − |
|---|---|---|---|---|---|
| 1 | `d12225e919` | `feat(common): add Zcash shielded message definitions` | 6 | +179 | −2 |
| 2 | `45cdb1c472` | `feat(common): regenerate protobuf bindings for Zcash` | 15 | +2585 | −150 |
| 3 | `6b4f3368e7` | `feat(python): add trezorlib.zcash host bindings` | 2 | +595 | − |
| 4 | `29a63c183d` | `test(python): cover the Zcash host transfer protocol` | 2 | +1513 | − |
| 5 | `3884bad6e3` | `fix(core): count wrapped section names in the xtask memory report` | 1 | +82 | −10 |
| 6 | `5ae4e66f4b` | `feat(core): add --require-free to xtask build` | 3 | +84 | −13 |
| 7 | `0ef7619833` | `feat(core): add the ironwood crate with Orchard receiver derivation` | 12 | +1096 | −2 |
| 8 | `9d09c80543` | `feat(core): expose the Zcash receiver to MicroPython` | 30 | +2526 | −19 |
| 9 | `886558c088` | `feat(core): add a sessionless cache slot for the Zcash weak-backup check` | 1 | +8 | − |
| 10 | `0716ef6887` | `feat(core): add Zcash shielded address display and viewing-key export` | 26 | +1782 | −8 |
| 11 | `1ee9d68431` | `feat(core): add the Ironwood streaming approval core` | 30 | +12525 | −25 |
| 12 | `3223765a80` | `feat(core): sign a streamed Zcash PCZT` | 29 | +3961 | −22 |

**PRs.**
- **A** = 1–2: protocol messages and generated bindings.
- **B** = 3–4: `trezorlib.zcash` and its tests.
- **C** = 5–10: two stock xtask improvements, the crate, the MicroPython glue, the cache slot, and address and viewing-key export.
- **D** = 11–12: the streaming approval core and PCZT signing.

**What changed from `3e737c17ca`.** Two passes changed it.

The first was the fix pass for the whole-series review:
- The build surface was renamed: `--zcash-shielded`, `ZCASH_SHIELDED=1`, `USE_ZCASH_SHIELDED`, `trezorzcash` and `helpers.py`.
- The wire IDs are now a contiguous 2300–2307, with no `reserved` line.
- The PROVISIONAL banners, and the tests that pinned them, are gone.
- Consent-screen strings are translated.
- The binary host fixtures were replaced with synthetic bytes.
- The Zcash device tests moved to `tests/zcash_tests`, their own CI job on all three models.
- Changelog fragments are named `+zcash*`.
- `apps/zcash/README.md` was added.
- The xtask memory-report and `--require-free` changes are split into their own commits.

The second pass fixed what the reviews of that first pass found. See
[reviews/V2-FIX-PASS.md](reviews/V2-FIX-PASS.md): four must-fixes, all of which would
have turned Trezor CI red, plus the should-fixes applied and those left open.

## Recipient address (2026-09-25, `3223765a80` → `dcc602fd48`)

**The change.** A shielded payment is now shown under the wallet's `user_address` from
the PCZT. The handler accepts it only if:
- it is plain letters and digits;
- it is no shorter than the Orchard-only encoding;
- it decodes, for the session's network, to a unified address whose Orchard receiver is
  exactly the verified one.

Otherwise the PCZT is refused with `DataError`. With no `user_address`, the Orchard-only
address is shown, as before.

This follows upstream's `apps/zcash/signer.py` `output_derive_script`, which pays the
transparent receiver of a unified address entered for a transparent payment and shows the
whole address. It resolves S1, which GPT-6 Sol rated must-fix. The rule is in design
§7 "Recipient address". The string is copied out of the scanner only on the payment event;
the scratch peak is unchanged at 25,008 B even with a 512-byte address on all 32 payments.

**Commits.** The change was squashed into D1 (scanner and session event) and D2 (glue,
handler, tests, design doc).

**Tests.**
- Crate: `a_payment_confirmation_carries_its_user_address`, and a scratch-tier case with
  the longest addresses.
- Unit: `TestZcashPaymentAddress`, 8 cases, including a NUL suffix and a short string with a
  valid checksum.
- Device, all three models:
  - `test_payment_shows_its_user_address`
  - `test_long_user_address_is_shown_whole` (213 and 512 characters)
  - `test_user_address_for_another_receiver_is_refused`
- The `stock_sdk_view_sdk` vector now carries real multi-receiver unified addresses, not
  placeholders.
- `screen_text` now follows long values across pages on delizia and eckhart as well as
  caesar. That is why some hashes of existing screens changed: the tests now visit, and
  check, every page.

**Reviews.** None of these runs was Fable.
- GPT-6 Sol ([SOL-REVIEW-RECIPIENT-f15a05aef8.md](reviews/SOL-REVIEW-RECIPIENT-f15a05aef8.md)):
  one must-fix and two should-fixes, all applied. The must-fix was that the C bech32 decoder
  stops at a NUL, so a suffix could be shown unchecked. The should-fixes were a short payload
  raising `AssertionError` in F4Jumble, and missing long-address device coverage.
- Sol re-check ([SOL-RECHECK-dcc602fd48.md](reviews/SOL-RECHECK-dcc602fd48.md)): all three
  resolved, nothing new.
- Opus 5.5 clarity review
  ([CLARITY-REVIEW-RECIPIENT-8d7d37ff97.md](reviews/CLARITY-REVIEW-RECIPIENT-8d7d37ff97.md)):
  applied.

**Hardware, Safe 5, 2026-09-25, image `dcc602fd48`** (SHA-256 `d5276ace…febe7b2b9`):
- Flashed from the bootloader. `get-features` shows revision `dcc602fd48…` and capability
  30; the test wallet and settings are unchanged.
- Two testnet sends to a 145-character two-receiver unified address (a transparent
  receiver plus the Orchard receiver of the wallet's own address #3), both signed, proven,
  verified and relayed: 12 actions `9f5b41e2…587c0f90` (mined at 4393742) and 1 action
  `3fa6eec7…010b825c`.
- On the second send the user watched the recipient screen and confirmed it shows the
  **wallet's address** (`utes t1ln z9zk yj7z u43v…`), not the Orchard-only form
  (`utest1qkmcs…`). The emulator tests check that the full string is reachable.
- Logs: the author's `session-logs/2026-09-25-dcc602fd48-*`.

**Upstream note.** The same NUL-prefix and short-payload behaviour exists in upstream's own
`unified_addresses.decode` on the transparent `SignTx` path. It is worth reporting upstream,
subject to the user's decision on contact.

**Gates at `dcc602fd48`, all green.**
- FLASH: T3T1 **98.62%** (+1 KB), T3B1 88.43%, T3W1 71.06%; zero warnings.
- Tests: crate 168, xtask 53, `trezor_lib` 88, Python 406; unit tests 140/140 on both the
  Zcash and stock builds.
- Emulators: 30 passed and 1 skipped on each of the three models, UI hashes matching; the
  stock emulator gives the single capability-absent failure.
- Style, generation and translations: pass.
- Slices: A 92.25%; B 405 passed, 1 skipped; C T3T1 98.47%.

## Gates

Run on this machine: macOS arm64, `CARGO_BUILD_JOBS=4`, build env `/tmp/ironwood-build-env.sh`,
nightly `rustfmt`. Logs are in the author's `.context/product-scaffold/v2-gates/<head>/`.

**Rebase onto `6c38a6ab1e` (2026-09-25).** The rebase applied without conflicts. Upstream's
8 new commits include delizia ActionBar/MoreInfoScreen changes. In files the series touches,
it changed only upstream's own hashes in `tests/ui_tests/fixtures.json`; the Zcash lines are
the same. The full gate set below was re-run at `3223765a80`, and every gate is green:
- Production builds: T3T1 **98.56%** (+0.5 KB from upstream), T3B1 88.37%, T3W1 71.03%;
  zero warnings.
- Tests: crate 167, xtask 53, `trezor_lib` 88 (1 ignored), Python 406;
  `translations_check`; the generation and style checks; `ruff`/flake8/pylint.
- Emulators: 26 passed and 1 skipped on each of the three models, with UI hashes matching.
  The stock emulator gives the single capability-absent failure, as intended.
- Core unit tests 140/140, both Zcash and stock; the boardloader and bootloader-emu builds pass.
- Slices: A 92.25%; B 405 passed, 1 skipped; C T3T1 98.47%, T3B1 88.28%, T3W1 70.98%;
  C crate 16 passed.

The hardware section below is for `6b4c5f090c`, before the rebase.

**Which head each gate ran at, before the rebase.** Everything in the tables below ran at `ecde24e43c`. The
hardware-tested head `6b4c5f090c` differs from it only by nightly-`rustfmt` rewrapping of two doc
comments (`ironwood/src/lib.rs`, `ironwood/src/prewarm.rs`). At `6b4c5f090c` itself, these
were re-run: the three production builds, `ruststyle_check`, `cargo fmt --check` and the
commit hook.

### Production firmware (`xtask build firmware --model <m> --zcash-shielded --frozen --emit-memory-analysis`)

| Model | FLASH | AUX1_RAM | Warnings | Image SHA-256 at `6b4c5f090c` |
|---|---|---|---|---|
| T3T1 | 1639.5 / 1664.0 KB = **98.53%** | 187.3 / 191.5 KB = 97.83% | 0 | `39838bd7…4fed718a` |
| T3B1 | 1470.5 / 1664.0 KB = **88.37%** | 50.2 / 191.5 KB = 26.22% | 0 | `b521a49c…07ce59aa` |
| T3W1 | 2369.5 / 3336.0 KB = **71.03%** | 416.0 / 416.0 KB | 0 | `9762daf9…7bf316a5` |

The sizes are identical to `3e737c17ca`. The images, with `.elf` and `.map`, are staged
for the hardware check.

### Host, emulator, generation, style

| Gate | Result |
|---|---|
| `make test_rust_zcash` | **167 passed** (119 + 48), 0 failed |
| `cargo clippy -p ironwood --all-targets -D warnings`, with and without `--features test` | clean |
| `cargo fmt --check` for `ironwood`, `trezor_lib`, `xtask` (nightly) | clean at `6b4c5f090c` |
| `cargo test -p xtask` | **53 passed**. Includes `ignores_zcash_shielded_for_other_projects`. |
| `xtask test trezor_lib` | **88 passed**, 1 ignored. The 3 warnings are upstream's `testutil.rs`, which the series does not touch. |
| `python/tests` | **406 passed**. `3e737c17ca` had 411; the 5 removed tests pinned the provisional banners and binary fixtures. |
| Emulators built the way CI builds them (`build_unix_frozen`, debuglink, universal, `ZCASH_SHIELDED=1`) + `make test_emu_zcash_ui` | **T3T1, T3B1, T3W1: 26 passed, 1 skipped each; UI hashes match.** The earlier count of 35 included upstream's nine transparent `test_sign_tx.py` cases, which stay in the stock device suite. The skip and its `ui_missing` are the two-signs arena test, which only runs on hardware. |
| Stock T3T1 emulator (`ZCASH_SHIELDED=0`) + `make test_emu_zcash` | **1 failed, 26 skipped.** This is the expected result: the only failure is `test_zcash_shielded_capability`, because the capability is absent on a stock build. |
| Core unit tests (`make test`), non-frozen T3T1 `ZCASH_SHIELDED=1` | **140/140** |
| Core unit tests (`make test`), stock T3T1 | **140/140**. At `6429dc1779` the result was 2/140 failing (MF-2). |
| `make build_boardloader` and `build_bootloader_emu` with `ZCASH_SHIELDED=1` | pass. At `6429dc1779` both failed (MF-1). |
| `templates_check`, `mocks_check`, `icons_check`; `make protobuf` | pass; regeneration changes 0 files |
| `cstyle_check`, `ruststyle_check`, `protostyle_check`, `changelog_check`, `translations_style_check`, `yaml_check`, `workflow_timeout_check`, `docs_summary_check` | clean |
| flake8, `ruff check`, `ruff format --check` (ruff 0.15.10), pylint over the 1,046 `PY_FILES` | clean, clean, clean, **10.00/10**. The `ruff` I001 at `6429dc1779` (MF-3) is fixed. |
| `pyright`, `editor_check` | **not run**: `pyright` and `editorconfig-checker` are not installed. |
| `cargo vet` | not re-run; the result at `3e737c17ca` stands, because no dependency changed (§5a below). |

### Per-PR buildability

| State | Checked | Result |
|---|---|---|
| A (commit 2) | `templates_check`; T3T1 production build without the feature | pass; FLASH **92.22%** |
| A + B (commit 4) | `python/tests` | **405 passed, 1 skipped**. The skip waits for D's transparent fixture (adversarial-review nit). |
| A + B + xtask + C (commit 10) | T3T1/T3B1/T3W1 `--zcash-shielded` production builds; `cargo test -p ironwood --features test` | **T3T1 98.47%**, T3B1 88.28%, T3W1 70.98%; crate 16 passed. The C slice fits on every model, which answers the adversarial review's open question. |

### Hardware (Safe 5, T3T1), 2026-09-25, image `6b4c5f090c`

- **Image:** `firmware-T3T1-2.12.6-6b4c5f090c.bin`, SHA-256 `39838bd7…4fed718a`, flashed from
  the bootloader. Tag `archive/ironwood-upstream-v2-6b4c5f090c-hw` keeps this commit.
- **Features:** `revision 6b4c5f090c81…`, `Capability.Zcash_Shielded` (30). The public
  test wallet is intact, with no PIN or passphrase and auto-lock at its 600 s default. No
  settings were changed.
- **Receive:** `ZcashGetAddress` index 3 from a cold boot took 20 s. The address is the
  same as on earlier images, and the wallet's stored viewing key derives the same receiver.
- **Send:** a testnet send of 0.001 TAZ to our own address, which the wallet built as
  **14 Ironwood actions**. The device reviewed it, and the user confirmed. It returned 14
  spend-authorization records. The host proved and verified the transaction and relayed it
  (txid `90993c2c1ec07ef5acb5a88f017d5b4a7204390266116a6a30c376267e07bf74`); 2 min 21 s
  end to end.
- **Bitcoin, same boot:** a synthetic Bitcoin sign succeeded.
- **Viewing key:** a `ZcashGetViewingKey` export (testnet, account 0) returned a canonical
  199-character UFVK. The wallet does not store the key as a string, so it was not compared
  byte for byte. The 14-spend sign above required the device's own derived FVK to equal the
  wallet's for every real spend.
- **Not run:** the arena counters. They need a debuglink image; this was the production one.

Logs: the author's `.context/product-scaffold/session-logs/2026-09-25-6b4c5f090c-*.log`.

## What remains before a PR could open

- **Arena counters on hardware** from a debuglink image of the head.
- **A Fable adversarial review of this head.** The v2 review was an Opus substitution because
  Fable usage was exhausted.
- **Maintainer decisions:**
  - M1: the git forks in `[patch.crates-io]`
  - M2: `cargo vet` for about 87 new crates
  - the 2300–2307 block and capability value 30 ([WIRE_ID_COORDINATION.md](WIRE_ID_COORDINATION.md))
  - the Orchard-only address on the per-output screen (S1)
  - the 5 s chunk timer (S2)
  - the AUX1 floor margin (S3)
- **Clarity items left open** ([reviews/V2-FIX-PASS.md](reviews/V2-FIX-PASS.md)):
  - the `rust/src/ironwood/` directory name
  - the `sign_pczt.py` structure
  - unreachable defensive checks
  - the trezorlib API shape
  - a generator for the device vectors
- **PR paperwork:**
  - changelog fragment names become `<PR>.added` once PRs exist
  - SatoshiLabs re-signs translations for the new `zcash__*` keys
  - PR descriptions: the drafts below are written for the first cut and must be refreshed for the renames and the 2300–2307 block before use

---

# Record of the first cut (`3e737c17ca`)

The rest of this note is the record of the nine-commit cut. It is kept for its
rebase-conflict and content-equivalence evidence, its `cargo vet` analysis and the
dependency decision. Flag names (`--ironwood`), wire IDs (2300–2309 with a reserved gap)
and commit numbers in it are those of the first cut.


What `ironwood/zcash-streaming-signing` looks like rebased onto `upstream/main` and
re-cut into the nine commits and four PRs that Part 4 ("The commit series") of
`MERGEABILITY_REVIEW.md` proposes.

| | |
|---|---|
| Worktree | `/Users/bryantwolf/conductor/workspaces/trezor-firmware/upstream-series` |
| Branch | `zcash/ironwood-upstream` → pushed to `origin` (`bawolf/trezor-firmware`) only; never to `upstream` |
| **Head** | **`3e737c17ca`** |
| Base | **`148e530180`** (`upstream/main`, 36 commits past the old merge base `ebd0468e20`) |
| Source of content | `ironwood/zcash-streaming-signing` @ **`170ea78cf4`** (on origin), untouched. Feature diff = `git diff ebd0468e20..170ea78cf4`, 131 files |
| Deliberate difference from the source | one: `ErrorCode::capacity()` gated `#[cfg(feature = "test")]` (§1), plus six re-recorded UI hashes forced by upstream copy (§5) |
| Previous heads | `b3ecec3abd` on `4178d500dc` (first pushed; replaced on origin by a `--force-with-lease` push of `3e737c17ca`); earlier local-only cuts from `252cda1b52`, `87dc48e0aa`, `85f748d098`. All scratch branches (`zcash/target-*`) and tags (`ironwood-rebased-scratch*`, `ironwood-series-*`) deleted |
| Date | 2026-09-24 |

The source branch was not modified.

**Two rebases.** The series was first built on `4178d500dc` (`upstream/main` when the
work started) and pushed as `b3ecec3abd`. It was then rebased onto `148e530180`, which
another session's fetch had moved `upstream/main` to: 11 commits (the `ward` feature
flag, English copy changes and regenerated translations, `deny(unused_imports)` in
`trezor_lib`, payment-request test changes) touching 18 of the series' files. §3 has
both sets of conflicts. Every result below is against `148e530180`.

---

## 1. The nine commits

Every message was run through `docs/git/hooks/commit-msg`: **9/9 pass**. Every message
carries the design context the Phase 1b comment purge moved out of the source — why
streaming, why device-owned records, the action cap, the two allocator tiers and their
sizes, the hedged nonce, the transparent-output rules — plus ZIP citations. Single
author and committer, `Bryant Wolf <bawolf@gmail.com>`; **no `Co-authored-by:`
trailers**.

| # | SHA | Subject | Files | +/− |
|---|---|---|---|---|
| 1 | `6fcad8234c` | `feat(common): add Zcash Ironwood message definitions` | 9 | +201 −2 |
| 2 | `865e122b6e` | `feat(common): regenerate protobuf bindings for Zcash` | 15 | +2,586 −150 |
| 3 | `5197407aae` | `feat(python): add trezorlib.zcash host bindings` | 2 | +603 |
| 4 | `88684ff5e2` | `test(python): cover the Zcash host transfer protocol` | 6 | +1,747 |
| 5 | `45abc1f930` | `feat(core): add the Ironwood receiver derivation crate` | 12 | +1,096 −2 |
| 6 | `3861c0f640` | `feat(core): expose the Ironwood receiver to MicroPython` | 32 | +2,695 −42 |
| 7 | `5592be9fde` | `feat(core): derive and confirm Zcash Unified Addresses and export the Orchard FVK` | 28 | +1,689 −6 |
| 8 | `036cdc6043` | `feat(core): add the Ironwood streaming approval core` | 30 | +12,533 −25 |
| 9 | `3e737c17ca` | `feat(core): stream a PCZT and return Zcash spend authorization signatures` | 25 | +3,845 −24 |

Total against the base: 131 files, +26,966 −222. Commit 2's −150 is
`rust/trezor-client/.../messages_management.rs` regenerated for the new capability
value: generator churn, not hand edits.

### What is in each

**1 — protocol.** `messages-zcash.proto` (new); the `MessageType` block in
`messages.proto` at **2300**; `Capability_Zcash_Shielded = 30` in
`messages-management.proto`, with a comment that hosts must check it themselves;
`common/protob/Makefile`; `legacy/firmware/protob/Makefile` (`SKIPPED_MESSAGES`);
`ALTCOIN_PREFIXES` in `cointool.py`; the cross-component changelog fragment,
byte-identical in `core/` and `python/`, plus legacy's.

**2 — generated bindings.** `core/src/trezor/messages.py`, the `MessageType`,
`ZcashNetwork` and `Capability` enums, `python/src/trezorlib/messages.py`, all of
`rust/trezor-client` (feature, `build_messages` entry, `trezor_message_impl!` block,
README line, generated protos). Pure `make gen` output, plus the two hand-maintained
`qstrdefsport.h` lines for the new enum module, called out in the message.

**3 — `trezorlib.zcash`**, whose three workflows declare
`capability=Capability.Zcash_Shielded` (informational: it does not refuse a device
without it), and its changelog fragment.

**4 — host tests.** `test_zcash_protocol.py`, `test_zcash_client.py`,
`python/tests/fixtures/zcash/`.

**5 — the crate.** `core/embed/ironwood/` with `Cargo.toml` (receive dependencies and
the host-only `test` feature), `build.rs`, a receive-only `lib.rs` (with the bare-metal
`compile_error!` guard), `src/receive/`, `tests/receive.rs`, `tests/seed_fingerprint.rs`,
and the workspace lines in `core/embed/Cargo.toml`/`Cargo.lock`.
**`grep -c 'git+' core/embed/Cargo.lock` is `0` at this commit.**

**6 — native module, allocator, build gating.**
`core/embed/rust/src/ironwood/{mod,allocator,allocator_unix,arena}.rs` — **already in
their final, fixed form** (in-place `realloc`, 48 KiB scratch, the refusal of a
never-released scratch, compile-time grid asserts) — and a key-derivation-only
`signing.rs`; `micropython/ironwood.rs` with `derive_receiver`, `derive_viewing_key`,
`seed_fingerprint` and `debug_region_info`; the `USE_IRONWOOD` chain (`modtrezorutils.c`
→ `utils.py` → `rustmods.c`, with its GC-block `_Static_assert` → `qstrdefsport.h` →
`librust_qstr.h.mako`); the `ironwood` feature across four manifests; `lib.rs` wiring
including the Linux-only `libgcc_s` link for debug emulators; xtask's flag,
`IRONWOOD_MODELS`, the build-owned AUX1 floor and report-before-publish; the
`.zcash_region` linker sections; the arena figures in `DebugLinkGcInfo`
(`apps/debug/__init__.py`, the one shared-core change here); the receiver half of
`test_trezorironwood.py`; `make IRONWOOD=1`, `test_rust_ironwood`; the
`core_ironwood_rust_test` CI job; and **the `[patch.crates-io]` block with the two git
dependencies**, whose message carries the "Dependency decision for maintainers"
section (see §9, PR C).

**7 — the receive handlers.** `apps/zcash/{get_address,get_viewing_key,
ironwood_account}.py`, the `unified_addresses.py` delta, `storage/cache_common.py`,
`Capability.Zcash_Shielded` in `apps/base.py`'s `get_features`, the dispatch for the two
receive messages, four `zcash__*` TR keys (1299–1302) with everything they generate,
four core unit-test modules, `pytest.mark.ironwood` (`conftest.py`,
`REGISTERED_MARKERS`), `test_basic::test_capabilities`, the device test with
`test_receive_address_chunkify` and its three UI fixtures, the `test_emu_zcash*`
targets (which also run `test_capabilities`), the `core_zcash_test` and
`core_zcash_unit_test` CI jobs, and three changelog fragments.

**8 — the streaming core.** `ironwood/src/{wire,stream,digest,effects,error,session,
prewarm,hedge}.rs`, the full `lib.rs` and dependency set, the four
`[[test]] required-features` entries, and the whole `tests/` suite: conformance, digest
equivalence, `Session`-vs-`Engine` equivalence, stream equivalence, hedged nonce, region
budget, request disposal, action bounds, transparent outputs, and the device-arena
traces (`rooted_tier`, `rooted_tier_prewarm_first`, `rooted_tier_vk_first`,
`scratch_tier`, `device_arena/`) that run the firmware's own `arena.rs` under
`computed-generators`. Plus their `test_rust_ironwood` lines, the second clippy lane,
`test_rust_ironwood_corpus` and the nightly corpus job.

**9 — the signing workflow.** The rest of `signing.rs` and `micropython/ironwood.rs`
(the session calls), `apps/zcash/sign_pczt.py`, the `ZcashSignPczt` dispatch, three
memo/transparent TR keys (1303–1305), `test_apps.zcash.sign_pczt.py`, the session half
of `test_trezorironwood.py`, `common/tests/fixtures/zcash/*.json`, the rest of the
device test (including two signs in one boot read over debuglink) and its UI fixtures,
`docs/common/zcash-ironwood-signing.md` + `docs/SUMMARY.md`, and the headline changelog
fragment.

### How the source's late commits were folded

The source moved five times during this work (`252cda1b52` → `3cf23f11e4` →
`87dc48e0aa` → `85f748d098` → `ff44d7c740` → `170ea78cf4`). **None of that became an
extra commit**; each file went where §D's shape puts it, and no commit in the series ever
ships the allocator that faulted on hardware or the 24 KiB scratch a 2-action sign
overran.

| Source change | Folded into |
|---|---|
| `allocator.rs`, `arena.rs`, `allocator_unix.rs` (in-place `realloc` `3cf23f11e4`; never-released-scratch refusal and wipe `9134c2f786`; 48 KiB scratch `81f8356df8`; follow-ups `87dc48e0aa`) | **6**. Blob-for-blob identical to `170ea78cf4` at commit 6 and at 7, 8 and 9 (checked). |
| `rustmods.c` GC-block assert; xtask AUX1 floor + report-before-publish (`89dbdfcf27`) and the Makefile comment about it; `debug_region_info` device-only + `SCRATCH_BYTES` const-assert; `DebugLinkGcInfo` arena items (`fc25155672`); Linux `libgcc_s` link (`170ea78cf4`) | **6** |
| Output slots from the shared cap in `session.rs` (`cbafd6699f`); arena traces, `device_arena/`, `region_budget.rs` rework (`0eb6efbb8c`, `8910cae97f`); `[[test]] required-features`; `computed-generators` test lines and clippy lane | **8** — they need the crate's `test` feature, `prewarm` and `computed-generators`, which first exist there |
| `Capability_Zcash_Shielded` (`85f748d098`) + its wording fix (`ff44d7c740`) | proto + comment → **1**; generated enums/protos → **2**; `@workflow(capability=…)` → **3**; `test_zcash_protocol.py` → **4**; `get_features`, `test_capabilities`, conftest comment, Make targets, changelog `99999.added.4` → **7**; design-note paragraph → **9** |
| `session_begin` changes, `sign_pczt.py`, session half of `test_trezorironwood.py`, device-test additions, design-note changes | **9** |

Messages 1, 2, 3, 6, 7, 8 and 9 were extended with the reasoning from those commits:
why `realloc` must resize in place, why a refused request exits from the allocator
itself under `panic = "immediate-abort"`, why 48 KiB (the device's own arena code under
every admitted shape; the 24 KiB tier was a 432 B shortfall, not fragmentation), why a
live scratch at `session_begin` is fatal, what the scratch holds and must not expose,
why output slots come from the shared cap, what the capability does and does not
enforce, and why `support.json` is unchanged. **The `support.json` reason is the
corrected one from `ff44d7c740`** (it records the release from which each model supports
a coin; `bitcoin:ZEC` already has an entry on every model; shielded signing ships in no
release), not `85f748d098`'s "keyed by coin".

**One change to the content, requested.** `ErrorCode::capacity()` in
`core/embed/ironwood/src/error.rs` is used only by the host-only whole-PCZT scanner
(`wire::scan`, itself `#[cfg(feature = "test")]`); the streaming parser returns
`Error::Capacity` directly. Every production build therefore warned `dead_code`. The
constructor is now `#[cfg(feature = "test")]` with a two-line comment saying why, in
commit 8 where the file is introduced. `cargo build -p ironwood` (no features) and both
clippy lanes are warning-free, and so are all three production firmware builds. This is
the only line of code in the series that is not in `170ea78cf4`.

Two fixes to the intermediate shape, found while re-cutting: commit 5's `Cargo.toml`
declares the crate's `test` feature, without which PR C's own CI lane
(`make test_rust_ironwood`, `cargo clippy --features test`) could not run on PR C
alone; and commit 6's `micropython/ironwood.rs` imports `Tuple` inside the
debuglink-device block, so the production build of PR C is warning-free.

---

## 2. The four PRs

| PR | Commits | Depends on | Reviewer | Size |
|---|---|---|---|---|
| **A** Protocol surface | 1–2 | — | protocol owner | 24 files, ~2,900 lines, almost all generated |
| **B** Host library | 3–4 | A | trezorlib owner | 8 files, ~2,350 lines |
| **C** Receive + viewing key | 5–7 | A (+ B for one device test) | core + build | 72 files, ~5,500 lines |
| **D** Streaming signing | 8–9 | C | crypto + core + security | 55 files, ~16,400 lines |

Order: **A first** — the 2300 block and capability value 30 are the long-pole external
dependency and block nothing else. **B alongside C** — different reviewers. **C before
D** — C is the smallest change that puts a working Zcash address on a Trezor, and it
settles the crate layout, the flag, the allocator, the UI conventions and the CI story
before D asks for review of the cryptography.

**One correction to §D's dependency graph.** C builds, and its core unit tests run,
without B. Its one device test does `from trezorlib import zcash`, so that test needs
B; in practice B lands before or with C. Moving the test to D would leave C with no
emulator coverage, which is worse.

---

## 3. Conflicts resolved during the rebases

### First rebase: onto `4178d500dc`

`4178d500dc` is 25 commits and 71 files past `ebd0468e20`; eleven of those files are
also touched by the feature. `git merge 252cda1b52` into `4178d500dc` produced **three**
content conflicts; the other eight auto-merged. Every later source commit then applied
without conflict (`87dc48e0aa`'s 25 files touch nothing upstream touched; `85f748d098`'s
two generated files that upstream also touched merged cleanly with `--3way`).

| File | Conflict | Resolution |
|---|---|---|
| `core/translations/order.json` | upstream took 1297 (`stellar__deploy_contract`) and 1298 (`stellar__wasm_hash`); the branch had its seven `zcash__*` keys at 1297–1303 | Kept upstream's two, moved the Zcash keys to **1299–1305** in the same relative order. `order.py --check` enforces only contiguity; the series re-derives the numbering commit by commit (four keys in 7, three in 9) and lands on the same assignment. |
| `core/embed/rust/src/translations/generated/translated_string.rs` | generated from the above | Took upstream's and regenerated (`make -C core templates`). |
| `core/translations/signatures.json` | `current` differs on both sides | Took upstream's; `translations/cli.py gen` recomputed `current`. What upstream does for an unreleased translation change; the new keys need a SatoshiLabs re-sign at release (a PR note). |

### Second rebase: onto `148e530180`

The rebased target on `4178d500dc` was merged with `148e530180`. Eighteen files
overlapped; **two** conflicted, sixteen auto-merged (`Cargo.lock`, `Cargo.toml`,
`projects/firmware/{Cargo.toml,project.toml}`, `rust/Cargo.toml`, `librust_qstr.h`,
`rust/src/lib.rs`, `upymod/{Cargo.toml,build.rs,modtrezorutils.c,rustmods.c}`,
`trezorutils.pyi`, `trezortranslate_keys.pyi`, `trezor/utils.py`, `en.json`,
`fixtures.json`). Upstream's `ward` flag runs through the same `USE_*` chain as
`USE_IRONWOOD`, one line apart, so the two flags now sit side by side in every file of the
chain.

| File | Conflict | Resolution |
|---|---|---|
| `core/embed/xtask/src/options.rs` | upstream added `map ward: bool` after `miniscript`, exactly where the branch added `map ironwood: bool` | Kept both, `ward` first, so the feature's hunk is still a pure insertion. The feature delta for this file is byte-identical to the source's. |
| `core/embed/rust/src/translations/generated/translated_string.rs` | upstream regenerated it for its English copy changes | Took upstream's, regenerated. Only the string-offset tables move; the Zcash keys keep 1299–1305. |

Then `translations/order.py` (no new items), `cli.py gen`, `make -C core templates`,
`core/tools/build_mocks` and `make protobuf` over the merge: nothing further changed,
and `cli.py merkle-root` passes.

**Every commit carried over.** For each of the nine commits, the added and removed lines
on the new base were compared with the same commit on `4178d500dc`: commits 1–6 are
identical; 7 and 9 differ only in `translated_string.rs`'s string-offset tables
(upstream's copy changes shift every offset); 8 differs only by the `capacity()` gate.
None of the text-anchored partition scripts silently missed on the new base.

**No wire-ID or capability collision.** The 2300 block and capability 30 are both still
free at `148e530180`.

---

## 4. Content equivalence against `170ea78cf4`

**Step 1 — the target.** The resolved merge of `252cda1b52` into `4178d500dc`, plus
exactly the patch `252cda1b52..170ea78cf4` (identical added/removed lines and file set),
merged with `148e530180` (§3), plus the `capacity()` gate, plus the six re-recorded UI
hashes (§5).

**Step 2 — the series reproduces the target exactly.**

```
$ git diff <target> 3e737c17ca
(empty)
```

The nine commits, applied in order to `148e530180`, land on precisely that tree. The
hand-cut intermediate slices — `lib.rs`, `signing.rs`, `micropython/ironwood.rs`, two
`Cargo.toml`s, `Cargo.lock`, `en.json`, `order.json`, `qstrdefsport.h`,
`workflow_handlers.py`, `core/Makefile`, `core.yml`, `test_trezorironwood.py`, the device
test, `fixtures.json` — are each restored to the target's exact bytes by a later commit.

**Step 3 — against the source branch itself.** For every file where
`git diff 170ea78cf4 3e737c17ca -- <the 131 feature paths>` is non-empty (25 files, all
of them files upstream changed between `ebd0468e20` and `148e530180`, plus `error.rs`),
the feature's own added and removed lines — `git diff ebd0468e20 170ea78cf4 -- <file>`
against `git diff 148e530180 3e737c17ca -- <file>`, sorted — were compared:

| Result | Files |
|---|---|
| **Feature delta identical** (21) | `Cargo.lock`, `Cargo.toml`, `projects/firmware/Cargo.toml`, `projects/firmware/project.toml`, `rust/Cargo.toml`, `librust_qstr.h`, `rust/src/lib.rs`, `upymod/Cargo.toml`, `upymod/build.rs`, `modtrezorutils.c`, `qstrdefsport.h`, `rustmods.c`, `xtask/src/options.rs`, `trezorutils.pyi`, `trezortranslate_keys.pyi`, `enums/__init__.py`, `core/src/trezor/messages.py`, `trezor/utils.py`, `en.json`, `trezorlib/messages.py`, and — before the re-record — `fixtures.json` |
| Differs, forced by upstream (3) | `order.json` (same seven keys, same order, +2 for Stellar's 1297/1298); `translated_string.rs` (same keys, offsets re-derived); `signatures.json` (`history` identical, `current` recomputed) |
| Differs, deliberate (2) | `ironwood/src/error.rs` — the `capacity()` gate; `fixtures.json` — six Zcash hashes re-recorded for upstream's "Please wait..." copy (§5) |

**Nothing dropped, nothing added.** All 131 files the source touches are present at the
head, and `git diff --name-only 148e530180 3e737c17ca` names no file outside that set.

---

## 5. Gates

Builds run sequentially at `CARGO_BUILD_JOBS=4`, build env `/tmp/ironwood-build-env.sh`.
The head `3e737c17ca` differs from `f60b6c5937` — the first cut on `148e530180`, where
most gates ran — only in six UI hashes in `tests/ui_tests/fixtures.json` (checked with
`git diff`), so non-UI results carry over; the UI suites were re-run against the head's
tree. "Run at" says which.

### Production firmware

| Model | FLASH | AUX1_RAM | AUX2_RAM |
|---|---|---|---|
| **T3T1** | 1639.5 / 1664.0 KB = **98.53%** (24.5 KB free) | 187.3 / 191.5 KB = 97.83% | 323.0 / 323.0 KB |
| **T3B1** | 1470.5 / 1664.0 KB = **88.37%** | 50.2 / 191.5 KB = 26.22% | 536.0 / 536.0 KB |
| **T3W1** | 2369.5 / 3336.0 KB = **71.03%** | 416.0 / 416.0 KB | — (one bank) |

`uv run xtask build firmware --model <m> --ironwood --frozen --emit-memory-analysis`,
exit 0, **zero warnings** on all three (the `capacity()` gate removed ours; upstream's
`fonts` import is gone on this base). **T3T1 idle GC heap: `.heap` = 240,368 B**, the
stock size, with `.zcash_region` = 40,960 B in AUX1. Run at `f60b6c5937`.

### Host, emulator, generation, style

| Gate | Result | Run at |
|---|---|---|
| `commit-msg` hook, all nine messages | **9/9 pass**; author and committer `Bryant Wolf <bawolf@gmail.com>`; no `Co-authored-by:` | head |
| `make test_rust_ironwood` | **119 + 48 passed, 0 failed**: lib 6, conformance 53, digest_equivalence 8, hedged_nonce 5, receive 11, region_budget 1, request_disposal 3, seed_fingerprint 3, session_equivalence 10, stream_equivalence 6, transparent_outputs 13; rooted_tier 12, rooted_tier_prewarm_first 12, rooted_tier_vk_first 12, scratch_tier 12 | `f60b6c5937` |
| `cargo clippy -p ironwood --all-targets -D warnings`, both lanes; `cargo build -p ironwood` (no features) | clean; warning-free | `f60b6c5937` |
| `cargo fmt -p ironwood --check`, `-p trezor_lib --check` | clean | `f60b6c5937` |
| `xtask test trezor_lib` | **88 passed**, 1 ignored | `f60b6c5937` |
| `python/tests` | **411 passed** | `f60b6c5937` |
| T3T1 non-frozen emulator, `core/tests/run_tests.sh` | **140/140** | `f60b6c5937` |
| Frozen `--ironwood` emulators, `device_tests/zcash` + `test_basic::test_capabilities`, `--ironwood --ui=test`, port 21347 | **T3T1 35 passed, 1 skipped · T3B1 35 passed, 1 skipped · T3W1 35 passed, 1 skipped; UI hashes match on all three** | head's tree |
| Stock T3T1 emulator (no `--ironwood`) | **10 passed, 26 skipped** — the capability is **absent** | `f60b6c5937` |
| `make gen_check` parts | `templates_check`, `mocks_check`, `icons_check` pass; `make protobuf` regenerates **zero** files. `protobuf_check` itself fails on macOS only (GNU `cp -T`) | `f60b6c5937` |
| flake8 / `ruff check` / `ruff format --check` / pylint over the Makefile's 1,041 `PY_FILES` | clean / clean / clean / **10.00/10** | `f60b6c5937` |
| `cstyle_check`, `ruststyle_check`, `protostyle_check`, `changelog_check`, `translations_style_check`, `yaml_check`, `workflow_timeout_check`, `docs_summary_check` | all clean | `f60b6c5937` |
| `pyright`, `editor_check`, `defs_check` | **not run**: `pyright`, `editorconfig-checker`, Graphviz `dot` not installed | — |

**UI fixtures: T3B1 and T3W1 checked, six hashes re-recorded.** On the new base the
three receive/viewing-key flows (`test_receive_address_chunkify`,
`test_device_fvk_matches_fixture`, `test_seed_fingerprint_export_is_opt_in`) failed the
UI check on **T3T1 and T3B1** and passed on **T3W1**. The cause is upstream's English
copy change `1dbc2c3c82`: `progress__please_wait` became "Please wait..." on Caesar and
Delizia (Eckhart already had the ellipsis), and exactly those three flows show a static
"Please wait" ring while the device derives keys (`get_address.py:72`,
`get_viewing_key.py:121`). The six hashes were re-recorded with
`tests/update_fixtures.py local` and folded into their owning commits: the two
`test_receive_address_chunkify` hashes into **7**, the other four into **9**. The tool
also added entries for the emulator-skipped test; those two were removed, because that
test never runs on an emulator and has nothing to record. After the re-record all three
models pass `--ui=test` with no `ui_failed`. The signing flows' hashes were unaffected
on every model. This is the first check of the T3B1 and T3W1 Zcash fixtures since they
were first recorded.

**The skip and the `ui_missing`.** Both are
`test_two_signs_in_one_boot_leave_the_region_as_they_found_it`, which reads the arena
counters over debuglink and skips itself on an emulator (`allocator_unix.rs` is
`malloc`). It runs on a device.

**One unreproduced failure (earlier head).** One device-suite run at `38d50c46d0`
reported `1 failed, 1 ui_failed` under a machine load of ~120 from other sessions; the
same binary passed on the next run and every run since. Recorded, not attributed.

### Per-PR buildability, on `148e530180`

| State | What was checked | Result | Run at |
|---|---|---|---|
| **A alone** (commit 2) | `templates_check`; T3T1 production build without `--ironwood`; `trezorlib.messages` has `ZcashGetAddress = 2300`, `Capability.Zcash_Shielded = 30` | pass; FLASH 1534.5 KB / 92.22% | `9c830c722b` — same tree as final commit 2 |
| **A + B** (commit 4) | `python/tests` | **410 passed, 1 skipped** | `c34414f140` — same tree as final commit 4 |
| **A + C** (5–7 cherry-picked onto 2, no conflicts) | T3T1 `--ironwood` production build; `cargo test -p ironwood --features test`; clippy `--features test -D warnings`; `cargo build -p ironwood`; non-frozen emulator core unit tests; frozen emulator device tests `--ui=test` (with PR B's `trezorlib/zcash.py` overlaid, untracked, then removed) | cherry-pick clean; build pass, **FLASH 1638.5 KB / 98.47%**, zero warnings; crate 16 tests pass; clippy clean; `cargo build -p ironwood` warning-free; **139/139**; **11 passed**, UI hashes match (upstream's nine `test_sign_tx.py` cases, `test_receive_address_chunkify`, `test_capabilities` confirming presence) | final SHAs `865e122b6e` + `45abc1f930` `3861c0f640` `5592be9fde` |
| **A + B + C + D** (head) | everything above | as above | head |

---

## 5a. `cargo vet`

Run with Trezor's own setup: `core/embed/supply-chain/` (`config.toml`, `audits.toml`,
`imports.lock`), cargo-vet **0.10.2** (the config pins `version = "0.10"`; installed
locally with `cargo install cargo-vet --version '~0.10' --locked`), invoked as CI does:
`make -C core vet_rust`, which is `cd embed; cargo vet --locked`. **Nothing was added to
the supply-chain files.** Every exploratory edit below was made in the worktree and
reverted; `git status` was clean afterwards.

| Tree | `cargo vet --locked` |
|---|---|
| `upstream/main` `148e530180` | **Vetting Succeeded** (40 fully audited, 2 partially audited, 65 exempted) |
| series head | **Vetting Failed: 116 unvetted dependencies** |

**Why 116 under `--locked`, and 87 in fact.** `--locked` uses the cached
`imports.lock`, which cargo-vet prunes to the audits the current graph needs, so it
carries none of the imported audits (notably `zcash/rust-ecosystem`'s) for our new
crates. Refreshing imports means a plain `cargo vet`, and on `upstream/main` that
**already fails** before it reaches any of our crates, on three stale upstream policy
entries: `[policy.qrcodegen]` names a crate that is really `qrcodegen-no-heap`, and
`qrcodegen-no-heap` and `trezor-thp` (path crates matching published versions) have no
`audit-as-crates-io` entry. That is upstream's own latent breakage, masked in CI by
`--locked`: worth a one-line note to upstream, not ours to fix in this series. To
measure the real gap I made those three entries valid in the scratch edit only, plus the
two policies our forks force (next paragraph), and ran `cargo vet` with fresh imports:
**89 unvetted, of which 2 (`qrcodegen-no-heap`, `trezor-thp`) are the upstream
artifacts just described. The series adds 87.** The other 29 of the 116 are covered by
imported audit sets once `imports.lock` is refreshed (for example `rand_core`,
`zcash_note_encryption`, `subtle`, `cipher`, `sha2 0.10.9`, `redjubjub`, `postcard`).

**The forks force a policy decision before vet can run at all.** With the git patches,
cargo-vet refuses to proceed until `orchard 0.15.3@git:d4792911…` and
`sinsemilla 0.1.0@git:6ca88ff5…` each have a `policy.*.audit-as-crates-io` entry,
because both match published crates.io versions. The choice matters:
`audit-as-crates-io = false` makes cargo-vet treat them as first-party and **not check
them at all** (that is how the measurement above ran, so neither fork appears in the
87); `= true` requires an audit of each fork (a `cargo vet diff` from the published
version, or a full audit). That is the dependency decision again in cargo-vet's terms,
and it is the maintainers' call. Upstream uses `true` for its own in-tree forks
(`pareen`, `trezor-tjpgdec`) and carries two first-party audits for
`trezor-noise-protocol`.

**The 87 crates lacking audits or exemptions** (all `safe-to-deploy` except the two
`zerocopy*`, which are dev-only `safe-to-run`):

- **Zcash-stack crates (24)**: `bls12_381:0.8.0`, `corez:0.1.1`, `equihash:0.3.0`, `f4jumble:0.1.1`, `ff:0.13.1`, `fpe:0.6.1`, `group:0.13.0`, `halo2_poseidon:0.1.0`, `incrementalmerkletree:0.8.2`, `jubjub:0.10.0`, `memuse:0.2.2`, `pairing:0.23.0`, `pasta_curves:0.5.1`, `pczt:0.9.3`, `reddsa:0.5.1`, `sapling-crypto:0.7.0`, `zcash_address:0.13.0`, `zcash_encoding:0.4.0`, `zcash_primitives:0.30.1`, `zcash_protocol:0.10.5`, `zcash_script:0.4.5`, `zcash_spec:0.2.1`, `zcash_transparent:0.10.0`, `zip32:0.2.1`. For each of these, cargo-vet itself
  suggests `cargo vet trust <crate> <publisher>` (publishers `nuttycom`, `str4d`,
  `ebfull`, `daira`, `conradoplg`), because the already-imported `zcash` audit set trusts
  those people. That is probably the cheapest honest route, but it is a trust decision,
  so it is listed, not taken.
- **Everything else (63)**: `aead:0.5.2`, `aes:0.8.4`, `arrayvec:0.7.8`, `bech32:0.11.1`, `bip32:0.6.0-pre.1`, `bitvec:1.1.1`, `blake2b_simd:1.0.2`, `blake2s_simd:1.0.5`, `block-buffer:0.11.0-rc.3`, `bs58:0.5.1`, `cbc:0.1.2`, `chacha20:0.9.1`, `chacha20poly1305:0.10.1`, `chrono:0.4.45`, `constant_time_eq:0.4.2`, `cpufeatures:0.2.17`, `crypto-common:0.1.7`, `crypto-common:0.2.0-rc.1`, `darling:0.23.0`, `darling_core:0.23.0`, `darling_macro:0.23.0`, `defmt:1.1.1`, `defmt-macros:1.1.1`, `defmt-parser:1.0.0`, `digest:0.10.7`, `digest:0.11.0-pre.9`, `funty:2.0.0`, `generic-array:0.14.7`, `getrandom:0.2.17`, `getset:0.1.7`, `hmac:0.13.0-pre.4`, `hybrid-array:0.2.3`, `ident_case:1.0.1`, `jiff:0.2.37`, `jiff-core:0.1.1`, `jiff-static:0.2.37`, `num-bigint:0.4.8`, `num-integer:0.1.47`, `pin-project-lite:0.2.17`, `poly1305:0.8.0`, `portable-atomic:1.15.0`, `portable-atomic-util:0.2.8`, `radium:0.7.0`, `rand:0.8.8`, `ripemd:0.1.3`, `ripemd:0.2.0-pre.4`, `secp256k1:0.29.1`, `secp256k1-sys:0.10.1`, `serde_with:3.22.0`, `serde_with_macros:3.22.0`, `sha1:0.10.7`, `sha2:0.11.0-pre.4`, `tap:1.0.1`, `time:0.3.55`, `tinyvec:1.13.3`, `tracing:0.1.44`, `tracing-core:0.1.36`, `typenum:1.20.1`, `version_check:0.9.5`, `wasi:0.11.1+wasi-snapshot-preview1`, `wyz:0.5.1`, `zerocopy:0.8.57`, `zerocopy-derive:0.8.57`.

cargo-vet estimates the audit backlog at **877,952 lines**. Its recommendations include
short diffs against already-audited versions (`cargo vet diff crypto-common 0.1.6
0.1.7`, 25 lines; `rand 0.8.6 0.8.8`, 63; `num-integer 0.1.46 0.1.47`, whose publisher
isrg/mozilla/bytecodealliance already trust) and whole-crate inspections for the large
ones (`sapling-crypto` 19,478 lines, `jiff-core` 15,306, `bls12_381` 14,973,
`secp256k1` 14,423, `pczt` `0.0.0 → 0.9.3` 13,335).

**What an exemption entry looks like.** Trezor's `config.toml` already holds 65, all of
this form, one per crate version:

```toml
[[exemptions.pczt]]
version = "0.9.3"
criteria = "safe-to-deploy"

[[exemptions.zerocopy]]
version = "0.8.57"
criteria = "safe-to-run"
```

The alternatives are a first-party audit in `audits.toml`:

```toml
[[audits.pczt]]
who = "Name <email>"
criteria = "safe-to-deploy"
version = "0.9.3"
```

or a trust entry (`cargo vet trust pczt nuttycom` writes a `[[trusted.pczt]]` block with
`criteria`, `user-id`, `start` and `end`). None has been added. Which crates get
exemptions, audits or trust is SatoshiLabs' policy, and `cargo vet regenerate exemptions`
would paper over exactly the question a reviewer needs to see. PR C's description should
carry this list, and CI's `vet_rust` job will fail until it is settled.

---

## 6. Method

The target tree is a three-way merge of `252cda1b52` into `4178d500dc`, resolved and
regenerated, each later source range applied on top and regenerated again, then merged
with `148e530180`, resolved and regenerated, plus the `capacity()` gate and the six
re-recorded UI hashes. The branch was then reset to the base and the nine
commits built forward against the target by a script
(`/tmp/partition/build_series.sh`): whole files by `git checkout <target> -- <path>`
where a file belongs to one commit, hand-cut slices where it is split, with a later
commit restoring the target's bytes. Generated artifacts (`librust_qstr.h`,
`translated_string.rs`, `trezortranslate_keys.pyi`, `signatures.json`,
`core/mocks/generated/*`) are re-derived at each commit by `make -C core templates` and
`core/tools/build_mocks`, never hand-split. `Cargo.lock` is re-resolved at each commit
that changes the graph, starting from the final lock so pinned versions do not drift.
§4 step 2 is the proof nothing leaked.

---

## 7. The dependency decision

Kept as decided: the two forks stay git-pinned by full SHA, and nothing is vendored or
published. Commit 6's message and PR C's draft description (§9) carry a labelled
**"Dependency decision for maintainers"** section: what each fork changes against its
crates.io base (`sinsemilla` 0.1.0 → `bawolf/sinsemilla@6ca88ff5…`, one commit, a
default-off `computed-generators` feature; `orchard` 0.15.3 →
`bawolf/orchard@d4792911…`, three commits: the FF1-free scope classifier, Sinsemilla
dedup on the verify path, the classifier ivk wipe — all behaviour-preserving), why
(flash; per-action verification latency), and the three shapes Trezor itself uses —
upstream to `zcash/sinsemilla` and `zcash/orchard` first, a published renamed fork
(`trezor-noise-protocol`), or vendoring under `core/vendor` with `audit-as-crates-io`
(`qrcodegen-no-heap`) — with the author offering to do whichever the maintainers prefer
and asking for ECC/ZF advice. The `PROVISIONAL` marker stays.

It is in commit 6, not commit 1, because commit 6 is the one that adds the
`[patch.crates-io]` block. Commit 5 deliberately needs no fork (its lock has zero `git+`
sources), and PR A contains no Rust dependencies at all.

---

## 8. What remains before the PRs open

| Item | State |
|---|---|
| **Keep up with `upstream/main`** | The series is on `148e530180`. Any later upstream movement means another merge of the target plus `build_series.sh`; the per-commit delta comparison in §3 is the check that the text-anchored partition scripts still hit. |
| **Hardware confirmation** | Flash the head's T3T1 image on the test Safe 5; run a cold `ZcashGetAddress`, a viewing-key export and a 2-action sign; read the arena figures over debuglink (the test that skips on every emulator). The emulator cannot show the allocator failures this series fixes. |
| **UI fixtures** | All three models' Zcash hashes match on `148e530180` after the six re-recorded (§5). CI boots only the T3T1 Ironwood emulator, so T3B1/T3W1 drift would go unnoticed; consider adding them to `core_zcash_test`'s matrix. The emulator-skipped test has no fixture on any model, by design. |
| **PR numbers** | Seven fragments still use `99999`: `core/.changelog.d/99999.added{,.1,.2,.3,.4}`, `python/.changelog.d/99999.added{,.1}`. They become `<PR>.added` once the PRs exist. Legacy's `+ironwood-zcash-skip-messages.added` can stay. |
| **Wire-ID and capability requests** | Ask for the 2300 block and capability value 30 in PR A. The three-line comment in `messages.proto` and the 26-line `PROVISIONAL` banner in `messages-zcash.proto` belong in PR A's description (review finding S9 is only partly addressed). |
| **SatoshiLabs translation re-sign** | Seven new `zcash__*` keys. A PR note. |
| **`cargo vet`** | Run (§5a). Fails with 87 new crates lacking audits/exemptions, plus a required `audit-as-crates-io` policy for each fork. Needs a maintainer decision on exemptions vs audits vs `cargo vet trust` for the Zcash publishers; nothing added silently. Also worth telling upstream that plain `cargo vet` (without `--locked`) fails on their three stale policy entries. |
| **Review findings still open** | **S5** codename-prefixed `ironwood_account.py`, no `layout.py`. **S6** no `apps/zcash/README.md`; `docs/common/index.md` not updated. **S8** the design doc still starts at §3 and skips 8, 9, 12. **S9** the proto banner. **N1** `USE_IRONWOOD` vs `USE_ZCASH`. **N5** the deleted trailing blank line in `core/embed/rust/Cargo.toml` (now an unrelated hunk in commit 6). All content changes, out of scope for a pass that re-partitions content; the one content change made (`capacity()`) was requested. |
| **`Co-authored-by:`** | Policy applied: none. |

---

## 9. PR-description drafts

Drafts only. Nothing is opened, and nothing may be sent upstream without the user's
explicit authorization. PR numbers, and the changelog fragment names that depend on
them, are filled in when the PRs exist.

### PR A — `feat(common): Zcash Ironwood message definitions`

Adds `messages-zcash.proto` (six messages, one enum), `Capability_Zcash_Shielded` in
`Features.capabilities`, and the generated bindings for core, trezorlib and
trezor-client. No device code; nothing depends on this PR except
the three that follow.

**Wire-ID request.** The messages use the 2300 block — the lowest unused one, per
`common/protob/protocol.md`. It is not reserved upstream; we are asking for it here,
and will move to whatever block you prefer. The same goes for capability value 30: the
lowest unused one, so a host can tell a firmware with the Zcash app (only `--ironwood`
builds of T3B1/T3T1/T3W1) from one without it. `reserved 2307, 2308` records the
whole-PCZT download pair an earlier design had and this one does not.

**Protocol summary.** Receive (`ZcashGetAddress`, always confirmed on-device),
viewing-key export (`ZcashGetViewingKey`, Orchard-only UFVK; the ZIP-32 seed
fingerprint is a separate opt-in with its own warning), and streamed signing
(`ZcashSignPczt` → `ZcashPcztRequest`/`ZcashPcztAck` → `ZcashSpendAuthSignatures`):
the device pulls the PCZT in 1,024-byte chunks it never retains and returns detached
66-byte signature records the host applies with the `pczt` crate, which re-verifies
each one. Every field that selects what the device derives, reviews or signs is
`required`.

### PR B — `feat(python): trezorlib.zcash`

Host bindings for the three workflows, and host-only tests of the transfer protocol.
Depends on A.

### PR C — `feat(core): Zcash receive and viewing-key export`

The first device-side PR, and deliberately the small one: a new `core/embed/ironwood`
crate (receiver derivation only), the `trezorironwood` native module, the
`USE_IRONWOOD` build gating (off by default; T3B1/T3T1/T3W1 only), the two receive
handlers and their screens, core unit tests, one emulator device test, and CI jobs.
It settles the crate layout, the flag, the UI conventions and the CI story before the
signing PR asks for review of the cryptography. Depends on A; its device test also
needs B.

Flash with PR C applied to PR A alone: T3T1 1638.0 / 1664.0 KB (98.44%), measured. The
table for all three models at the full series is in PR D's description.

#### Dependency decision for maintainers

This PR adds a `[patch.crates-io]` section to `core/embed/Cargo.toml` with two git
dependencies, both pinned by full commit SHA to forks the author maintains publicly.
**PROVISIONAL.** Upstream `core/embed/Cargo.lock` has no `git+` sources, and we know
this cannot ship as written. Nothing has been vendored or published, because which of
the shapes below to use is your call, and undoing the wrong one is more work than
waiting.

**`sinsemilla`** — base: crates.io **0.1.0**. Fork:
`github.com/bawolf/sinsemilla` @ **`6ca88ff52835899c87ff83489b95c5cda887e8a4`**, one
commit on the 0.1.0 release. Adds a `computed-generators` cargo feature, **off by
default**: with it on, `HashDomain::hash_to_point` computes each generator `S_j` from
its defining hash-to-curve construction instead of reading the precomputed coordinate
table. `SINSEMILLA_S` stays exported; the change is API-additive, and a consumer that
does not enable the feature gets exactly the crate it had. Why: the table is the
largest constant in the Zcash path and the T3T1 image cannot hold it (T3T1 is at
98.5% of flash with the table gone). The cost is runtime, paid once per boot.
Table-free generators are not a Trezor-only need — Ledger's `ledger_zcash_crypto`
carries an equivalent construction.

**`orchard`** — base: crates.io **0.15.3**. Fork:
`github.com/bawolf/orchard` @ **`d4792911986deda16cc3d689a97f316655f08e84`**, three
commits on the 0.15.3 release:
1. `scope_for_address` reconstructs the candidate address from the address's own
   diversifier for each scope instead of recovering and re-encrypting the diversifier
   index — an **FF1-free scope classifier**. Same result; FF1 index recovery leaves
   the call path.
2. **Verification dedup** on the Ironwood PCZT verify path: per action, from ~17
   NoteCommit + 2 CommitIvk Sinsemilla evaluations to exactly 2 NoteCommit
   (device-owned spend) or 3 (dummy), with the two Commit^ivk paid once per bundle via
   a session-scoped classifier. This is the per-action **latency** that decides whether
   a 32-action signing flow is usable on a Cortex-M33.
3. The classifier's cached ivk is zeroized on drop.

All three are **behaviour-preserving** — same outputs, fewer evaluations, plus a wipe —
rather than feature-gated, because they are on the path every caller of the verify API
takes.

**Three shapes Trezor already uses — we will do whichever you prefer:**
1. **Upstream first**: propose `computed-generators` to `zcash/sinsemilla` and the
   verification changes to `zcash/orchard`, then depend on released versions. The two
   crates share maintainers, so it is one conversation. This leaves nothing behind.
2. **Published renamed fork** — the `trezor-noise-protocol` precedent (a fork of
   `noise-protocol` published under a new name, audited in-tree with a first-party
   `[[audits]]` entry); `trezor-tjpgdec` and `qrcodegen-no-heap` follow the same
   pattern.
3. **Vendor under `core/vendor/`** with a `version` + `path` dual-spec and
   `[policy.<crate>] audit-as-crates-io = true`, as `qrcodegen-no-heap` is handled
   today.

We would welcome advice from ECC or the Zcash Foundation on (1) before opening those
upstream PRs, and will open them with a maintainer's framing rather than without one.
The same section is in the message of commit 6 (`feat(core): expose the Ironwood
receiver to MicroPython`), the commit that introduces the dependencies.

### PR D — `feat(core): streamed Zcash PCZT signing`

The streaming approval core (a `no_std`, `forbid(unsafe_code)` state machine that
reviews a PCZT section by section and keeps one fixed-size record per action), its
host test suite with a CI job, the signing handler, and
`docs/common/zcash-ironwood-signing.md`. Depends on C. The commit messages carry the
design reasoning — why streaming, why device-owned records, the 32-action cap, the two
allocator tiers, the hedged nonce, the transparent-output rules — so a reviewer can
read the series in order. Flash table for T3B1/T3T1/T3W1 goes here.
