# Reviews of the v2 fix pass and what was done about them

Series: `zcash/ironwood-upstream-v2` on `upstream/main` `148e530180`.

- **Reviewed head:** `6429dc1779`, the fix pass for the 2026-09-24 whole-series review of
  `3e737c17ca` ([FABLE-SERIES-REVIEW-3e737c17ca.md](FABLE-SERIES-REVIEW-3e737c17ca.md)).
- **Head after this pass:** `ecde24e43c`. It has the same twelve commits, with fixups
  squashed into the commits that own the code. Nightly-`rustfmt` rewrapping made it
  `6b4c5f090c`, the head tested on hardware. A rebase onto a newer `upstream/main` then
  made the current head; see `UPSTREAM_SERIES.md`.

## The reviews

| Review | Model actually run | Report | Verdict |
|---|---|---|---|
| Adversarial correctness | **Opus 5.5** (`claude-opus-5-5[1m]`). A Fable 5.1 run was started first and failed at once with HTTP 429 "out of usage credits" (model sent: `claude-fable-5-1`), so this is an **Opus substitution, not a Fable review**. | [ADVERSARIAL-REVIEW-V2-6429dc1779-opus.md](ADVERSARIAL-REVIEW-V2-6429dc1779-opus.md) | Signing safety **ACCEPT**: no behavior change beyond renames. Fix-pass claims **ACCEPT-WITH-MUST-FIX** (four new must-fixes). |
| Clarity / readability | **Opus 5.5** (`claude-opus-5-5[1m]`) | [CLARITY-REVIEW-V2-6429dc1779.md](CLARITY-REVIEW-V2-6429dc1779.md) | 13 should-fix, 13 nits |
| Second-opinion adversarial, of the fixed head `6b4c5f090c` (2026-09-25) | **GPT-6 Sol** (`gpt-6-sol`, OpenAI via the Codex CLI 0.155.0-alpha bundled with the ChatGPT app, reasoning effort medium; 121,015 tokens). The Homebrew Codex CLI 0.154.0 refused the model for a ChatGPT account; that failed run's log is kept. | [SOL-REVIEW-V2-6b4c5f090c.md](SOL-REVIEW-V2-6b4c5f090c.md) | Signing safety **ACCEPT-WITH-MUST-FIX**; upstream readiness **REJECT**. It found no path to a signature over unreviewed data, and no finding new to the earlier reviews. It **raises S1 (the per-output screen shows an Orchard-only address, not the address the user supplied) to must-fix**, and restates S2 (5 s chunk timer), M1 (git forks) and M2 (`cargo vet`). |

All three reviews were read-only. The gates below were run by the coordinator; see `UPSTREAM_SERIES.md`, "Gates".

## Must-fixes: all four reproduced, then fixed

| # | Finding | Reproduced | Fix | Commit |
|---|---|---|---|---|
| MF-1 | `make ZCASH_SHIELDED=1` passes `--zcash-shielded` to every target, and xtask rejected it for boardloader/bootloader, so every Zcash CI build job would fail before reaching firmware. | Yes. `make build_boardloader` and `build_bootloader_emu` with `ZCASH_SHIELDED=1` both failed with the rejection message. | xtask treats the option like `--miniscript`: it is ignored for projects other than firmware and validated only for firmware. `memory_requirements()` also checks the project. The test `ignores_zcash_shielded_for_other_projects` covers boardloader, bootloader and kernel. Both builds now succeed. | C2 |
| MF-2 | Once the cache slot was gated, stock universal unit tests hit a cache slot that does not exist. | Yes, and with a third failure the review missed. The stock T3T1 `make test` gave 2/140 files failing: `helpers` (the two weak-backup tests) and `get_viewing_key` (the seed-fingerprint test, which needs `trezorzcash`). | The three tests are gated on `USE_ZCASH_SHIELDED`. C3's message said "eight fields" and now says seven. | C3 message, C4 |
| MF-3 | `ruff` I001 in `core/src/trezor/utils.py`: `USE_ZCASH_SHIELDED` sat in the old `USE_IRONWOOD` place. | Yes (`ruff check`, 1 error). | Moved to its sorted place. The same entries were also re-sorted in the upymod, firmware `Cargo.toml` and `project.toml` files. | C2 |
| MF-4 | `uv.lock` was rewritten in C4 (dropping `exclude-newer-span`) and reverted in D2. | Yes. | `uv.lock` is untouched in every commit (`git log 148e530180..HEAD -- uv.lock` is empty). | C4, D2 |

## Should-fixes and nits

**Fixed:**

- **Makefile floor.** The duplicate `--require-free AUX1_RAM=4096` block is gone from the Makefile (adversarial SF-new-3, clarity #10). The xtask floor is the one Zcash floor, applied on every build path.
  - `--require-free` stays as a generic xtask option that a developer can pass. It is no longer used by the Makefile.
- **Design doc (SF-new-2, clarity #5 and #13).**
  - It now says CI runs the device tests on all three models.
  - It no longer describes the handler as using the Bitcoin "generator pattern" or receiving a "change flag".
  - The reviewer note about `support.json`, the "old `.buf` region" history and "Decided" in a heading are removed.
  - The title is now "Zcash shielded signing", with a sentence saying it signs Ironwood actions. SUMMARY, index and README were updated to match.
- **`lib.rs`.** It said the reference height is displayed; it now says, as the handler does, that the height is a policy input and not shown. `session.rs` cites "§2 step 9", not step 8.
- **History narration (clarity #1, #2).** Removed or cut to a line from:
  - `sign_pczt.py`: autolock essay, chunkify, progress reports, the Bitcoin comparisons
  - `get_address.py`, `get_viewing_key.py`
  - the core test docstring
  - `features.rs`, `scratch_tier.rs`, `prewarm.rs`
  - the proto and trezorlib "firmware older than this field" text, and the trezorlib test named after it
- **Leftover "ironwood" names for Zcash-app code (clarity #3).**
  - Allocator fatal strings now say "Zcash …".
  - The linker comments name `ZCASH_SHIELDED`.
  - `# zcash / ironwood` is now `# zcash`.
  - The stale `ironwood` feature name in the crate `Cargo.toml` is updated.
  - The CI job and Makefile help say "Zcash crate corpus".
- **README (clarity #12).** It says that shielded spends are Ironwood actions and lists `signer.py`/`hasher.py` under transparent signing.
- **Commit messages.**
  - A1 now says eight messages, not six.
  - B1 describes what the host checks, instead of "helpers a caller needs".
  - C2 describes the build gating as it now is and drops the reviewer-directed sentence.
  - D2 wording is fixed.

**Open. Recorded here, not done in this pass; each is a code-structure change that should get its own review:**

- **`rust/src/ironwood/` → `rust/src/zcash/`** (clarity #3). This is a directory rename across C2 and D2. The crate `core/embed/ironwood` keeps its name, because Ironwood is the pool.
- **Sort order of `zcash_shielded` in `rust/Cargo.toml` and `modtrezorutils.c`/`trezorutils.pyi`** (clarity #4). D2 also changes the feature block, and the C file and mock have to move together.
- **`sign_pczt.py` structure** (clarity #6, #9): the 12-parameter `_stream_and_sign`, the `handle_out` list, duplicate output-confirm helpers, unused native return fields, `MAXIMUM_FEE`/`EXPIRY_WINDOW`/`SCRATCH_BYTES` duplicated in Python, and two vocabularies for steps and networks.
- **Unreachable defensive checks** (clarity #7), and the repeated weak-backup and stack-wipe wrappers (clarity #8).
- **trezorlib API shape** (clarity #11): `export_viewing_key` plus `get_viewing_key`, and the UFVK re-encoding.
- **Device vectors** in `common/tests/fixtures/zcash/*.json` have no in-tree generator (adversarial SF-new-1). This matters for handoff: a maintainer must be able to regenerate them.
- **Baseline items still open:**
  - M1: git forks in `[patch.crates-io]`
  - M2: `cargo vet`
  - S1: the per-output screen shows an Orchard-only address
  - S2: the 5 s chunk timer
  - S3: the AUX1 margin
  - S7, S8, S10; N1–N4, N6, N8
- **S1 severity.** GPT-6 Sol rates the Orchard-only recipient screen must-fix. The fix it proposes: decode the address the user supplied, require its Orchard receiver to equal the verified one, and show that full address; otherwise show the Orchard-only form clearly. This is a design decision for the maintainers and the user, recorded here, not yet made.
- **Fable adversarial review of this head.** It is still owed once Fable usage is available. The Opus substitution above does not count as one.
