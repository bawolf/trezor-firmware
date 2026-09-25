# Adversarial correctness review: Zcash series v2 at `6429dc1779`

**Reviewer model: `claude-opus-5-5[1m]` (Opus 5.5).** This is **not** a Fable run. It stands in for a Fable review that failed with a usage-credit 429. Record it as an Opus substitution.

Subject: `/Users/bryantwolf/conductor/workspaces/trezor-firmware/upstream-series`, branch
`zcash/ironwood-upstream-v2`, HEAD `6429dc1779`, 12 commits on `148e530180`. Compared
against the previous series head `3e737c17ca`, using the baseline in
`FABLE-SERIES-REVIEW-3e737c17ca.md` (M1–M5, S1–S13, N1–N8). Date: 2026-09-24.

## Verdict

| Scope | Verdict |
|---|---|
| **(a) Signing safety** | **ACCEPT.** From `3e737c17ca` to `6429dc1779`, the signing, approval, memory and key-handling code changes only by renames, comments, translated labels, ButtonRequest names and one error string. Behavior is unchanged. Baseline S1 and S2 remain open as design should-fixes. |
| **(b) The fix pass's upstream-readiness claims** | **ACCEPT-WITH-MUST-FIX.** Most claimed items are done correctly: the rename, wire-ID compaction, banner removal, translations, changelog, README and xtask split. However, the pass adds **four new must-fixes**. Three turn mandatory CI red and one is a lockfile churn inside PR C. M1 and M2, which are dependency decisions, are still open, so upstream submission as a whole is still **NOT-READY**. |

### New must-fixes

1. **Zcash CI build jobs fail before they reach the firmware.** `core/Makefile:74-87` adds `--zcash-shielded` to `XTASK_BUILD_OPTS` for every target. `core/embed/xtask/src/options.rs:354-359` rejects that flag for any project other than firmware.
2. **Two new unit tests fail on stock universal builds.** The weak-backup cache slot is now gated, but the tests that use it are not: `core/tests/test_apps.zcash.helpers.py:26,62-115` and `core/src/storage/cache_common.py:135-136`.
3. **`ruff` import-order failure (I001) in `core/src/trezor/utils.py:31`.** The rename put `USE_ZCASH_SHIELDED` out of order, and `make style_check` runs on every PR.
4. **`uv.lock` is rewritten in PR C (`6664649c73`) and reverted in PR D (`6429dc1779`).** The C-slice rewrite downgrades the lock and removes `exclude-newer-span = "P30D"`.

Details, evidence and fixes are in §2.

---

## 1. What I ran and what I didn't

Everything was read-only against the source tree. I never checked out a commit, never wrote to the worktree, and never built. `git status --porcelain` was empty and HEAD was `6429dc1779` throughout. The checks below ran on `git archive` exports under `/tmp`.

| Check | Result |
|---|---|
| `git diff 3e737c17ca 6429dc1779` (93 files), read hunk by hunk. All Rust hunks read in full; Python, C, build and CI hunks read in full. | see §3 |
| Leftover old-name search at every commit (`git grep` for `USE_IRONWOOD`, `trezorironwood`, `--ironwood`, `IRONWOOD=`, `mark.ironwood`, `ironwood_account`, `test_rust_ironwood`, `feature = "ironwood"`, `trezor_lib/ironwood`, `upymod/ironwood`, `IRONWOOD_MODELS`, `matrix.ironwood`, `"ironwood_…"` br_names) | **none remain**. The only hits are the crate/JSON field `"ironwood"` and `ironwood_bsk_present` in fixtures. |
| Wire IDs across the proto, `MessageType.py`, `enums/__init__.py`, `trezorlib/messages.py` and the Rust `messages.rs` enum and `from_i32`, plus the embedded descriptor bytes | consistent at 2300–2307. In the descriptor, the enum length went from `\xb9l` to `\xa9l` (−16 B, the two removed `reserved` ranges), and 2307 is encoded as `\x83\x12`. No `2308`, `2309`, `reserved 2307` or `DONOR` remains outside unrelated files. |
| Translation Merkle root (`core/translations/cli.py merkle-root`, repo `.venv` python, archives of C4 `6664649c73` and D2 `6429dc1779`) | **matches `signatures.json` at both commits** |
| `core/tools/build_mocks --check` on archives of `b92ba49d11`, `6664649c73` and `6429dc1779` | no content diffs. The only noise is broken relative symlinks from `/tmp`. |
| `librust_qstr.h`: re-derived the qstr set (grep `MP_QSTR_` in rust/src plus `en.json` keys) at `b92ba49d11`, `6664649c73`, `0d12e8dcc1` and `6429dc1779` | membership matches at all four. Sorting and gating were not re-rendered. |
| `ruff check` / `ruff format --check` (ruff 0.15.10, `/opt/anaconda3/bin`, repo `pyproject.toml`) over every series `.py` file at HEAD | **1 error: I001 in `core/src/trezor/utils.py`**. Formatting is clean (32 files). The same check on the base `utils.py` passes. |
| stable `rustfmt 1.8.0 --check` on the changed xtask, rust/ironwood and upymod files | clean (the nightly-only options in `rustfmt.toml` are not applied by stable) |
| `docs/git/hooks/commit-msg` on all 12 messages | 12/12 exit 0; no attribution trailers |
| UI fixtures, base vs HEAD, every model and group | only `zcash_tests` added (26 per T3B1/T3T1/T3W1); no stock hash changed |
| Files churned in the series but unchanged net | **`uv.lock` only** (`6664649c73` rewrites it, `6429dc1779` reverts it) |
| **Not run:** any build (firmware, emulator, xtask tests), `cargo test -p ironwood`, `cargo vet`, core unit tests, device or UI tests, CI, flake8, pylint, pyright, hardware. I did not re-audit the fork diffs. I did not measure the C-slice flash footprint. | |

---

## 2. Must-fix findings

### MF-1: every Zcash firmware and emulator CI job fails at the boardloader or bootloader step

- **Evidence:** `core/Makefile:74-87` adds `--zcash-shielded` (and, on T3T1/T3B1, `--require-free AUX1_RAM=4096`) to the global `XTASK_BUILD_OPTS` whenever `ZCASH_SHIELDED=1`. Every target uses that variable: `build_boardloader`, `build_bootloader`, `build_prodtest`, `build_bootloader_emu`, and others.
- `ResolvedBuildArgs::from_build_args` → `validate()` (`core/embed/xtask/src/options.rs:354-359`) bails with "--zcash-shielded is supported only for T3B1, T3T1, T3W1 firmware builds" whenever the project is not `Firmware`. The series' own test `rejects_zcash_shielded_for_other_projects` (`features.rs`) pins this behavior.
- **Failing jobs:**
  - `core_firmware`: the Zcash matrix entries (`.github/workflows/core.yml:94-108`) are `coins: universal`, `type: normal`, so the step at `core.yml:126-133` runs `make -C core build_boardloader` with `ZCASH_SHIELDED=1`. It fails for T3B1, T3T1 and T3W1 before `build_firmware` runs.
  - `core_emu`: the Zcash entries (`core.yml:182-198`) are `universal`/`noasan`, so `make -C core build_bootloader_emu` (`core.yml:213`) fails.
  - `core_zcash_test` depends on `core_emu` and therefore never gets its artifact.

  Only `core_zcash_unit_test` (`build_unix`) and `core_zcash_rust_test` survive.
- **Scope:** broken since the flags were introduced in `b92ba49d11` (C2). The v1 series had the same defect under `--ironwood`, and the baseline review did not catch it because nobody ran CI or `make build_boardloader IRONWOOD=1`. This pass extended the broken entries to all three models.
- **Why `--miniscript` doesn't hit this:** it is also a `map` option and also sits in the global opts, but it has no project check, so non-firmware projects simply ignore it.
- **Fix:** pick one of these.
  - (a) Keep `--zcash-shielded` out of the global opts. Append it, and `--require-free`, only in the `build_firmware`, `build_unix*` and `build_unix_frozen` recipes.
  - (b) Make `validate()` refuse only an explicit top-level request for a non-firmware project. Treat the option as ignored for other projects, as `miniscript` is, and have `memory_requirements()` check `self.project == Project::Firmware` as well, so the floor never depends on its call site.

  Either way, add an xtask test that `boardloader`/`bootloader` builds with the Makefile's option set resolve.

### MF-2: two stock unit tests fail because of the new gated cache slot (a regression introduced by the S5 fix)

- **Evidence:** `core/src/storage/cache_common.py:135-136` now appends the `APP_ZCASH_WEAK_BACKUP` field only `if utils.USE_ZCASH_SHIELDED`. `helpers.has_weak_backup()` (`core/src/apps/zcash/helpers.py:81`) calls `context.cache_get_int(APP_ZCASH_WEAK_BACKUP)`. That goes through `SessionlessCache.get(7)` → `DataCache.get`, whose `utils.ensure(key < len(self.fields))` raises `AssertionError` on a stock build, which has 7 fields.
- `core/tests/test_apps.zcash.helpers.py:26` gates `TestZcashAccount` only on `not utils.BITCOIN_ONLY`. As a result, `test_backup_strength_policy` (`:62-86`) and `test_weak_backup_predicate_is_memoized` (`:88-115`) call the real `has_weak_backup()` on every **stock universal** emulator.
- **Failure scenario:** `core_unit_python_test` (`core.yml:~304-330`, matrix T2T1/T3B1/T3T1/T3W1 universal, `make -C core test`) turns red from `6664649c73` onward.
- **Why it wasn't caught:** `core_zcash_unit_test` runs only on a `ZCASH_SHIELDED=1` emulator, which hides the failure. The baseline's run 8 was also on an `IRONWOOD=1` build.
- **Commit message error:** `d9aaa43d73` says "stock builds keep the same eight fields". Stock has seven.
- **Fix:** put `@unittest.skipUnless(utils.USE_ZCASH_SHIELDED, …)` on those two tests, or on the class. Also consider a guard such as `utils.ensure(len(self.fields) == 8)` inside the `if`, so that a future upstream sessionless key taking index 7 cannot silently alias the Zcash slot. See N-new-3.

### MF-3: `ruff` import sorting fails in `core/src/trezor/utils.py`, breaking the mandatory style job

- **Evidence:** `core/src/trezor/utils.py:31` places `USE_ZCASH_SHIELDED` between `USE_HAPTIC` and `USE_MCU_ATTESTATION`, the old `USE_IRONWOOD` slot. `ruff check --select I` reports I001 on this file. The base file passes.
- **CI path:** `tools/style.py.include` covers `^core/src/`, and `.github/workflows/prebuild.yml:46` runs `make style_check` → `ruff check $(PY_FILES)`.
- **Scope:** introduced in `b92ba49d11`, which means PR C fails the style check on its own.
- **Other entries left in the "ironwood" alphabetical slot** (not CI-checked, but worth fixing in the same edit):
  - `core/embed/rust/Cargo.toml:82`
  - `core/embed/upymod/Cargo.toml:47`
  - `core/embed/projects/firmware/Cargo.toml:40`
  - `core/embed/projects/firmware/project.toml:70`
  - `modtrezorutils.c:855,1011`
  - `mocks/generated/trezorutils.pyi:267`
- **Fix:** `ruff check --fix` (move the name after `USE_WARD`). Re-sort the others by hand.

### MF-4: `uv.lock` is rewritten in the C slice and reverted in the D slice

- **Evidence:** `6664649c73` changes `uv.lock` from `4af8eec5ae` to `f44e97fdfb`, a 1,677-line diff. It sets `revision = 3 → 2`, **removes `[options] exclude-newer-span = "P30D"`**, and adds and re-resolves packages such as `annotated-doc`, apparently because an older `uv` rewrote the lock. `6429dc1779` reverts it byte for byte. The net change from base is zero; it is the only such file in the series.
- **Failure scenario:**
  - PR C, reviewed or merged alone, ships an unrelated lock downgrade that drops the project's 30-day dependency-age limit. That is a supply-chain policy change nobody asked for.
  - PR D then carries a 1,677-line lock revert that has nothing to do with signing.
  - Anyone bisecting between C and D runs a different resolution.
- **Fix:** remove `uv.lock` from both commits (`git checkout 148e530180 -- uv.lock` when re-cutting C4 and D2).

---

## 3. Priority 1: behavior changes in `3e737c17ca..6429dc1779` beyond renames

Every hunk in the named files was read. Summary:

| File | Change | Behavioral? |
|---|---|---|
| `core/embed/ironwood/src/{session,stream,lib,wire}.rs` | Design-doc section numbers renumbered (§3→1, §4→2, §5→3, §6→4, §7→5, §10→6, §11→7, §13→8); history sentences removed from doc comments (`lib.rs:14-16`, `:989-990`) | No. Comments only. I checked every "§N" in the source against the new headings (`docs/common/zcash-ironwood-signing.md` 16/63/149/175/185/211/221/279), and all are consistent. One pre-existing citation is off: `session.rs:62` "design §2 step 8" describes the step-9 rule; see N-new-5. |
| `core/embed/rust/src/ironwood/{allocator,arena,mod,signing}.rs` | cfg `feature = "ironwood"` → `"zcash_shielded"` (all three gates in `mod.rs`, both in `lib.rs`); comments | No. The gate is renamed consistently in `rust/Cargo.toml:82`, the firmware `Cargo.toml:40`, `project.toml:70`, `upymod/Cargo.toml:47` and `build.rs:71-72,964,992`, in the C macro `USE_ZCASH_SHIELDED` (`rustmods.c:55,70`, `modtrezorutils.c:1011`, `librust_qstr.h(.mako)`), and in the Python `utils.USE_ZCASH_SHIELDED`. |
| `micropython/ironwood.rs` → `zcash.rs` | `mp_module_trezorironwood` → `mp_module_trezorzcash`; `MP_REGISTER_MODULE(MP_QSTR_trezorzcash, …)` | No. The mock generator derives `trezorzcash` from the static name, and the checked-in `.pyi` matches (§1). |
| `sign_pczt.py` | imports `trezorzcash`/`helpers`; three consent-screen literals now `TR.zcash__expires_at_block` / `total_actions` / `public_amount`; error text "Zcash shielded support is not enabled" | No. Order of checks, `require_session` placement, approve→sign with no `await` between, stack wipes and handle ownership are all unchanged. English strings are identical, so the UI hashes are unchanged. |
| `get_address.py`, `get_viewing_key.py` | renames; `br_name` `ironwood_*` → `zcash_*`; error text | No. The only host-visible change is the ButtonRequest name. No host code or test matches the old names. |
| `storage/cache_common.py` | slot gated on `USE_ZCASH_SHIELDED` | Production behavior is the same on Zcash builds (index 7 exists there), and stock production never imports `apps.zcash.helpers`. **It does break stock tests (MF-2).** |
| `core/embed/rust/Cargo.toml` | trailing blank line restored | Baseline N5 is fixed. |

**Signing-safety conclusion:** a host still cannot obtain a signature over anything the user did not see. The baseline §1 mapping from rule to code still holds line for line; only the "§" labels moved. **No must-fix.**

---

## 4. Priority 2: rename completeness and dependency builds

- **Complete for build and test surfaces.** Details in §1: no leftover old flag, feature, module, macro, marker or Makefile target at any commit. The CLI flag `--zcash-shielded` (derived by clap from `map zcash_shielded`), the `project.toml` key `zcash-shielded`, the cargo feature `zcash_shielded`, the C macro `USE_ZCASH_SHIELDED` and the Python `utils.USE_ZCASH_SHIELDED` agree. `test_rust_zcash` and `test_rust_zcash_corpus` exist, and CI calls them by those names.
- **Kernel and secmon dependency builds.** `build_impl` (`xtask/src/cargo.rs:94-106`) clones the args with `project` switched to the dependency and **keeps `zcash_shielded: true`**. The clone does not act on it:
  - `validate()` runs only in `from_build_args`, not on the clone;
  - `resolve_features` ignores options a project does not map (the test `omits_zcash_shielded_from_firmware_dependency_builds` covers this);
  - `-Zbuild-std=core,alloc` requires `Project::Firmware` (`features.rs:185`);
  - `memory_requirements()` runs only when `!is_dependency` (`cargo.rs:124`).

  So a firmware build never applies the AUX1 floor or the feature to kernel or secmon. What remains is robustness: `memory_requirements()` (`options.rs:375-382`) does not check `project` itself, and a **top-level** non-firmware build fails validation outright (MF-1). No `for_dependency` helper is needed if MF-1's fix (b) adds the project check.
- **Old name that is intentionally kept:** the crate `ironwood`, `rust/src/ironwood/`, `POOL_IRONWOOD`, and the doc file `zcash-ironwood-signing.md`. Ironwood is the pool/protocol name here, so this is defensible, but see S9 in §6.
- **Cosmetic leftovers:**
  - `workflow_handlers.py:245` `# zcash / ironwood`;
  - the CI job name `core.yml:740` "Zcash Ironwood corpus and mutation sweeps";
  - the linker-script comments "built with Ironwood" (`stm32u58/firmware.ld:77`, `stm32u5g/firmware.ld:89`).

## 5. Priority 3: wire-ID renumbering

Consistent everywhere, as listed in §1. `test_zcash_protocol.py:28-37` pins the new values. `common/protob/messages.proto:411-419` has no `reserved` line, and the Rust descriptor bytes agree. Legacy excludes `Zcash*` through `SKIPPED_MESSAGES`.

**Residual (nit):** `test_the_block_holds_only_zcash` (`BLOCK = range(2300, 2400)`, `test_zcash_protocol.py:40,79-83`), `test_wire_id` and `test_one_capability` (`CAPABILITY = 30`) still pin allocation choices. Maintainers decide these, and the tests break the moment the block or capability moves. Keep them if maintainers want them; otherwise drop the `BLOCK`/`CAPABILITY` pins and keep the round-trip and collision tests.

## 6. Priority 4: per-commit hygiene (A = 2, B = 2, xtask = 2, C = 4, D = 2)

| Commit | Standalone? | Notes |
|---|---|---|
| `ce32c15be8` A1 proto | proto only | Generated files land in A2, so `templates_check`/`protobuf_check` is red at A1 alone. Acceptable inside one PR (nit). |
| `09505f5640` A2 generated | yes | |
| `9789a73b1e` / `11e0c66185` B | yes | `test_sign_pczt_streams_a_transparent_bundle_unchanged` (`python/tests/test_zcash_client.py:808-830`) skips unless the D-slice fixture `common/tests/fixtures/zcash/sign_pczt.transparent.json` exists. That is a hidden cross-PR dependency, and the skip is dead code after D (nit). |
| `6f5e8a599c`, `f3ce2e421d` xtask | yes | S4 is fixed: the stock-affecting memusage and `cargo.rs` changes now have their own commits. |
| `87e1a743a3` C1 crate | yes | The commit message's "no `git+` source after this commit" is accurate (0). |
| `b92ba49d11` C2 glue | **no** | Introduces MF-1 (Zcash CI jobs) and MF-3 (ruff I001). Introduces the forks (M1): the lock has 2 `git+` sources from here on. |
| `d9aaa43d73` C3 cache slot | yes | Wrong commit message ("eight fields"); sets up MF-2. |
| `6664649c73` C4 receive/viewing | **no** | MF-2 (stock unit tests), MF-4 (`uv.lock`). The viewing-key device tests (`tests/zcash_tests/test_get_viewing_key.py`) only land in D2, because they need `vector()` and the D-slice fixtures, so PR C ships `ZcashGetViewingKey` with no device test (should-fix: move an FVK golden into C). |
| `0d12e8dcc1` D1 approval core | yes | Adds the crate's `computed-generators` feature. The firmware only enables it in D2 (`rust/Cargo.toml:89`). This is the same as v1 and harmless for D1. |
| `6429dc1779` D2 signing | contains MF-4 revert | |

**Unverified risk (should-fix: verify, don't assume).** From C2 through D1, the Zcash firmware links `orchard` (the FVK self-check in `rust/src/ironwood/signing.rs`) **without** `computed-generators`. The patched `sinsemilla` therefore links its precomputed generator table, and the C-slice image is larger than the head image. Nobody has built the C slice for T3B1/T3T1/T3W1, so whether it fits flash is unknown. Build `b92ba49d11`/`6664649c73` Zcash firmware once, or move the `"ironwood/computed-generators"` wiring into C2.

## 7. Priority 5: status of baseline findings at `6429dc1779`

| ID | Status | Evidence |
|---|---|---|
| M1 git forks | **OPEN** | `core/embed/Cargo.toml:146-148`; Cargo.lock has 2 `git+` sources from C2 |
| M2 `cargo vet` | **OPEN** | `core/embed/supply-chain/` untouched; `core.yml:1072` still runs `vet_rust` |
| M3 provisional banners/tests | **FIXED** (residual nit, §5) | banners gone from `messages-zcash.proto`, `messages.proto`, `messages-management.proto:199`, `trezorlib/zcash.py:17`; the donor/provisional/freed-ID tests are removed |
| M4 untranslated consent strings | **FIXED** | `sign_pczt.py:288-306` uses `TR.zcash__*`; new keys 1306–1308 in `en.json`/`order.json`; Merkle root verified. The remaining `"Mainnet"`/`"Testnet"`, `"ZEC #n"` and `f"Zcash {network} {account}"` (`sign_pczt.py:315`, `get_viewing_key.py:73`, `helpers.py:31-34,53`) follow the Solana `"Solana #n"` and Cardano network-name precedent. |
| M5 binary host fixtures | **FIXED** for trezorlib (the `.pczt` files and `MANIFEST.json` are deleted; the tests use synthetic bytes) | new should-fix below: the device vectors have no in-tree generator |
| S1 multi-receiver UA display | OPEN | `sign_pczt.py:120-140` unchanged |
| S2 5 s chunk timer | OPEN | `sign_pczt.py:54,97-111` |
| S3 AUX1 margin 192 B | OPEN | `REGION_BYTES = 40 KiB` (`allocator.rs:45`), floor 4096 (`options.rs:321`), region unchanged |
| S4 split memusage/cargo.rs | **FIXED** | commits `6f5e8a599c`, `f3ce2e421d` |
| S5 unconditional cache slot | **FIXED, but regressed the tests** | `cache_common.py:135-136`; see MF-2 |
| S6 two-sided `test_capabilities` | **FIXED** | `tests/device_tests/test_basic.py`, `tests/conftest.py` and `REGISTERED_MARKERS` are byte-identical to base; the capability test moved to `tests/zcash_tests/test_capabilities.py` |
| S7 `_call` wrapper hides PIN error | OPEN | `python/src/trezorlib/zcash.py:310-322` |
| S8 host-pinned `MAX_ACTIONS` | OPEN | `zcash.py:67-68,580,592` |
| S9' cancel-outcome tests | OPEN (not re-derived; no relevant diff) | |
| S9 naming/layout | **MOSTLY FIXED** | flag/module/marker/helpers renamed; `apps/zcash/README.md` added; `docs/common/index.md:28` entry. Still no `layout.py`; crate and module dir still `ironwood` (defensible). |
| S10 no `cli/zcash.py` | OPEN | |
| S11 project-history wording | **MOSTLY FIXED** | "Safe 5", "Boundary 2", the dated bug and "donor" are gone from the diff. Residue: `ironwood/tests/scratch_tier.rs:7-12` narrates what `region_budget.rs` "used to" do; the pinned fork comments ("MUST-FIX #3", "Fable review") are unchanged because the fork SHAs are unchanged. |
| S12 doc numbering | **FIXED** | sections 1–8, contiguous |
| S13 changelog fragments | **FIXED** | `core/.changelog.d/+zcash-shielded.added` (C4), `+zcash-shielded-signing.added` (D2), `python/.changelog.d/+zcash.added` (B1); `+name` has upstream precedent; the `tools/changelog.py` style rules pass by inspection |
| N1 double `owns` | OPEN | `rust/src/ironwood/signing.rs:490-493` |
| N2 `Box::new` placement comments | OPEN | comments unchanged |
| N3 UI `ValueError` → "Malformed PCZT" | OPEN | `sign_pczt.py:396-405` |
| N4 ungated `SCRATCH_BYTES`/`debug_region_info` qstrs | OPEN | `librust_qstr.h:67,356`; mako set at `librust_qstr.h.mako:45-54` |
| N5 prewarm tripwire | OPEN (by design) | |
| N6 btc-only qstrs | OPEN | `qstrdefsport.h:690-697` |
| N7 T3B1/T3W1 fixtures unchecked | **FIXED on paper, blocked by MF-1** | `core.yml:760` matrix is all three models; the emulators it needs fail to build |
| N8 `__all__` / type pinning | OPEN | `zcash.py:43-54` |
| `allocator.rs` dead `cfg_attr` | OPEN | `allocator.rs:71` |
| prior N5 blank line | **FIXED** | `rust/Cargo.toml` diff from base has no trailing hunk |

## 8. Other new findings

**Should-fix**

- **SF-new-1: the device vectors have no in-tree generator.** `common/tests/fixtures/zcash/sign_pczt{,.memos,.transparent,.failed}.json` (hex PCZTs, expected FVK, seed fingerprint and records) are described only as "built for the emulator's wallet" (`tests/zcash_tests/test_sign_pczt.py:17-24`). Nothing in `core/embed/ironwood/tests` or `tools` produces or checks them, so a maintainer cannot regenerate them from a clean checkout. The handoff rules require exactly that. Fix: add a `cargo test -p ironwood` test with `--ignored`/`UPDATE_FIXTURES` that emits and compares these JSON files from the crate's synthetic builder.
- **SF-new-2: the design note contradicts CI.** `docs/common/zcash-ironwood-signing.md:53-55` says "CI boots an Ironwood image only on T3T1 … the T3B1 and T3W1 Ironwood images are built but not run". `core.yml:760` now runs all three. Fix: update the text.
- **SF-new-3: the Makefile's `--require-free AUX1_RAM=4096` applies to every project built with `ZCASH_SHIELDED=1`** (boardloader, bootloader, prodtest map files), not only firmware (`core/Makefile:85-87`). It is also redundant with xtask's own floor (`options.rs:377`). Once MF-1 is fixed, this can silently gate unrelated images. Fix: drop it from the Makefile (xtask already enforces the floor), or move it to the firmware recipes with the flag.

**Nits**

- **N-new-1:** in the `core_zcash_test` job, `ui-report` (`core.yml:778-784`) moves `tests/ui_tests/reports/test/*` away before the later `upload-artifact` step uploads that same path, so the job's own artifact has no UI report.
- **N-new-2:** the `apps/zcash/README.md:6` command is `cargo xtask build --zcash-shielded`; the actual invocation is `cargo xtask build firmware --zcash-shielded`.
- **N-new-3:** the conditional `self.fields += (1,)` (`cache_common.py:135-136`) couples `APP_ZCASH_WEAK_BACKUP = 7` to "no other sessionless key after index 6". If a future upstream key takes index 7 unconditionally, Zcash builds silently alias it. Add an `ensure` or a comment next to the key list.
- **N-new-4:** the core changelog fragments announce a `[T3B1,T3T1,T3W1]` user feature that ships in no release build (off by default). Say "firmware built with `ZCASH_SHIELDED=1`" in the signing fragment too, or ask maintainers whether an experimental build flag gets a changelog entry at all.
- **N-new-5:** `session.rs:62` "design §2 step 8" should read step 9 (display). Pre-existing, carried through the renumbering.

## 9. Residual trust boundaries

Unchanged from baseline §8: host-asserted input totals, an unauthenticated reference height, the hedged-nonce dependence on the TRNG, and the fact that emulator runs don't exercise the arenas. This review adds one: **none of the v2 CI changes have been exercised.** MF-1 shows the Zcash CI chain as written cannot pass, so the "all three models in CI" claim is so far only a claim about configuration.
