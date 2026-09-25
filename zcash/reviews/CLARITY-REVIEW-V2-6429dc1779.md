# Clarity review: `zcash/ironwood-upstream-v2` @ `6429dc1779`

- **Reviewer model:** `claude-opus-5-5[1m]` (Opus 5.5). This is the independent readability review. It is **not** a Fable run.
- **Scope:** 12 commits `148e530180..6429dc1779` in `/Users/bryantwolf/conductor/workspaces/trezor-firmware/upstream-series`. I only read the tree (`git show`/`git diff` at the commit). I did not build, run tests, or edit anything.
- **Yardstick:** upstream `apps/stellar`, `apps/solana`, `apps/monero`, `trezorlib/stellar.py`, `COMMITS.md`, the existing `.changelog.d` fragments, and the upstream ordering/style of the touched build files.

**Overall.** The re-cut fixed the big items from the last round: the handlers are named after their functions (`get_address`, `get_viewing_key`, `sign_pczt`), the flags are consistent (`--zcash-shielded` / `ZCASH_SHIELDED` / `USE_ZCASH_SHIELDED` / `zcash_shielded` / `Capability_Zcash_Shielded`), the consent strings are in `TR`, and all commit scopes are on the allowlist. Three kinds of problem remain:

1. Rename residue: "ironwood" still names things that are really the Zcash app, and the alphabetical placement still follows the old name.
2. History and "why we changed it" narration, mostly in `sign_pczt.py`, the tests and the docs.
3. A few factual mismatches between the docs/commit messages and the code.

Findings are listed by rank. Line numbers refer to `6429dc1779`.

---

## Should-fix (ranked)

### 1. History narration in shipped code, concentrated in `sign_pczt.py`
Upstream code comments describe what the code does now. These comments describe what an earlier version did and why it changed:
- `core/src/apps/zcash/sign_pczt.py:518-540`: "that gap was the live-session symptom", "a bar that used to animate…", "`_stream_and_sign` no longer touches the timer itself", "strictly more often than the per-chunk touch it replaces". **Rewrite as:** `# Report before each blocking feed: it repaints the screen after a confirmation and touches the idle timer (ProgressLayout.report).`
- `sign_pczt.py:142-150`: explains the chunkify asymmetry with "that screen shipped unchunked and must not change under an existing host". `ZcashGetAddress` has never shipped. **Rewrite as:** `# Always chunked: a 106-character UA is not comparable unbroken.`
- `sign_pczt.py:35-53`: a 19-line essay on autolock parity with Bitcoin ("we do exactly the same thing… we deliberately do NOT…"). Keep two lines: `# Drops a host that stalls one chunk. An abandoned sign is closed by autolock; the finally in sign_pczt() wipes the native session.` The rest belongs in the PR text.
- `sign_pczt.py:458-476` and `578-582`: "Bitcoin shows… Same here", "so do we". Cut each to one line.
- `get_address.py:70-76` ("the way every other app does… appears exactly as before") and `get_address.py:107-111` ("an old host… gets exactly the screen it got before"). Replace both with `# Show progress during the blocking native derivation.` Delete the chunkify comment: the kwarg explains itself.
- `get_viewing_key.py:111-127`: a 17-line essay on why the ring is static. Keep one line. `get_viewing_key.py:77-83`: reduce to `# Hold: an exported UFVK cannot be revoked.`
- `core/tests/test_apps.zcash.sign_pczt.py:102-110,266`, `core/tests/test_apps.zcash.get_address.py:133-138` ("the old screen is what an old host gets"), and `core/embed/xtask/src/features.rs:296` ("a hand-written list kept forgetting"). State the property that is tested, not what it replaced.
- `core/embed/ironwood/tests/scratch_tier.rs:7-16`: a debugging story ("used to trace… the device refused… 432 B over"). Replace with what the test measures and why first-fit peak equals the minimum tier.
- `core/embed/ironwood/src/prewarm.rs:24` ("the old `bench::warmup`") and `:69` ("as before"). Delete the comparisons.

### 2. Pre-existence narration for fields that have never shipped
Several comments describe how firmware older than a field behaves, but no such firmware exists:
- `common/protob/messages-zcash.proto:21` ("which is what the device did before the field existed") and `:92-93` ("absent from any firmware older than this field").
- `python/src/trezorlib/zcash.py:95-97`, `:207-208` and `:227-229`.
- `python/tests/test_zcash_client.py:402` (`test_export_viewing_key_loads_a_response_without_the_fingerprint_field`).

Delete these. "Optional; absent unless requested" is the whole contract.

### 3. `ironwood` still names Zcash-app glue, not only the Ironwood pool
Using "Ironwood" is justified where the thing is the Ironwood pool or protocol: the `core/embed/ironwood` crate (the same kind of name as `orchard`), `POOL_IRONWOOD`, "Ironwood bundle/actions" in the doc and proto, and `IronwoodDomain`. It is not justified in these places:
- `core/embed/rust/src/lib.rs:56` `mod ironwood;` and the directory `core/embed/rust/src/ironwood/`. Its own module doc says "Zcash shielded device integration", and its sibling is `micropython/zcash.rs`. Rename to `mod zcash` / `src/zcash/`, following the `thp/` + `thp/micropython.rs` split that the module doc cites.
- Fatal strings in `core/embed/rust/src/ironwood/allocator.rs:133,168,190,224` ("Ironwood scratch not released", "Ironwood allocation failed"). The allocator also serves Orchard receive derivation. Use `"Zcash …"`.
- Linker comments `core/embed/sys/linker/stm32u58/firmware.ld:77` and `stm32u5g/firmware.ld:89` say "built with Ironwood". Use "built with `ZCASH_SHIELDED`".
- `core/src/apps/workflow_handlers.py:245` `# zcash / ironwood`. Use `# zcash`, the same as every other block.
- `core/embed/ironwood/Cargo.toml:12` says "via the `rust` crate's `ironwood` feature". That feature is `zcash_shielded`, so the comment is stale.
- `core/embed/Cargo.toml:80,89`: `ironwood-pasta-curves` / `ironwood-sinsemilla` are package aliases, but the lock file has exactly one `pasta_curves` and one `sinsemilla`. Drop the aliases and use the real crate names. This removes about 20 `ironwood_pasta_curves::` paths.
- Two titles: "Zcash Ironwood streaming signing" (`docs/SUMMARY.md`, `docs/common/index.md`) and "The Zcash Ironwood app" (`docs/common/zcash-ironwood-signing.md:3`), while the app everywhere else is "Zcash shielded". Title it "Zcash shielded signing". In the first paragraph, say that it signs Ironwood actions.
- `core/embed/ironwood/tests/*` and `Makefile` are mixed: `test_rust_zcash` has help text "Ironwood corpus", and the CI job "Zcash Ironwood corpus and mutation sweeps" sits next to "Zcash crate tests". Pick "Zcash (ironwood crate)" and use it everywhere.

### 4. Rename residue in alphabetical placement
The new entries sit exactly where `ironwood`/`IRONWOOD` would sort. Upstream keeps all of these lists sorted, and isort will reorder the Python import:
- `core/src/trezor/utils.py:31` (`USE_ZCASH_SHIELDED` sits between `USE_HAPTIC` and `USE_MCU_ATTESTATION`).
- `core/embed/projects/firmware/Cargo.toml:40` and `project.toml:70` (between `frozen` and `log_*`).
- `core/embed/upymod/Cargo.toml:47` (between `haptic` and `layout_*`).
- `core/embed/rust/Cargo.toml:82` (feature between `layout_eckhart` and `micropython`), and `:31-33` (deps `ironwood`/`orchard`/`rand_core` placed after `qrcodegen-no-heap`).
- `core/embed/Cargo.toml` `[workspace.dependencies]` (`ironwood` after `num-traits`).

Move each entry to its sorted position.

### 5. Docs and commit messages that contradict the code
- `docs/common/zcash-ironwood-signing.md:53-55`: "CI boots an Ironwood image only on T3T1… T3B1 and T3W1 … built but not run". `core_zcash_test` runs on all three models, and commit `6664649c73` says so. Fix the doc.
- The same doc at `:213-214` says the handler runs "on the Bitcoin signer's generator pattern". `sign_pczt.py` is a plain async loop. At `:217` it lists "a change flag" among what the handler receives. The payload is `(action_index, receiver, value, memo_kind, memo)`, with no change flag.
- `core/embed/ironwood/src/lib.rs:10-11` says the device must "display the reference height as unverified". Both `sign_pczt.py:309-310` and doc §2/§7 say it is deliberately not shown.
- `core/embed/ironwood/src/session.rs:62` cites "design §2 step 8". That text is step 9.
- Commit `ce32c15be8` says "six messages and one enum". The file has **eight** messages.
- Commit `9789a73b1e` says "`get_address`, `export_viewing_key` and `sign_pczt`, plus the Bech32m/F4Jumble helpers a caller needs". The helpers are private (`_bech32m_*`, `_f4jumble`), and `__all__` exports `get_viewing_key`, not `export_viewing_key`.

### 6. `sign_pczt.py` structure
- `_stream_and_sign` (`:431-593`) takes 12 positional parameters and is about 160 lines, roughly a third of them comments. The `handle_out: list[int]` out-parameter (`:365-369`, `:493`) exists only so the caller's `finally` can see the handle. Instead, call `session_begin` inside `sign_pczt` with `handle = None` before the `try` and `_cancel_native(handle)` in the `finally`, then pass `handle` in. Resolve `coin = coininfo.by_name(coin_name)` once and pass `coin`. Today `by_name` runs in `_confirm_output`, `_transparent_address`, `_confirm_transparent_output` and `_confirm_totals` on every output.
- `_confirm_output` and `_confirm_transparent_output` (`:120-204`) are the same `layouts.confirm_output(...)` call with a different address string. Collapse them into one `_confirm_payment(address, value, index, coin, account_label, path)`.
- `payments` (`:499`) counts transparent outputs too, and it is passed as `output_index`. Rename it to `output_index`.
- The native API returns fields that no caller reads. The review tuple has 11 positions, and `_confirm_totals` discards 5 of them (`:274-286`). `action_index`/`index` in both output payloads are also discarded (`:548,558`). Trim `session_feed` (`micropython/zcash.rs:280-318`) to what is shown. The step kinds 0/1/2/3 also read out of stream order: transparent is 3 but arrives first. Renumber them, or give Rust `Step` and Python `_STEP_*` the same names (see #9).
- `MAXIMUM_FEE` / `EXPIRY_WINDOW` (`:29-33`) are called "Device-owned policy limits (ironwood::Limits)", yet Python passes them as two `session_begin` arguments that have one caller and one value. Keep them in Rust (`Limits::DEVICE`) and drop both arguments.
- `SCRATCH_BYTES` (`:56-59`) duplicates `trezorzcash.SCRATCH_BYTES`, with a comment plus a test (`test_trezorzcash.py:181-184`) to keep the two equal. Use `trezorzcash.SCRATCH_BYTES` and delete both.

### 7. Checks the protobuf decoder already guarantees, plus a gate checked twice
- `helpers.py:25-26` and `sign_pczt.py:342-347` do `type(x) is not int` and `0 <= h <= 0xFFFF_FFFF` on `required uint32` fields. The decoder already guarantees both, so these branches are unreachable.
- `get_address.py:46-47`, `get_viewing_key.py:56-57` and `sign_pczt.py:331-332` each repeat `if not utils.USE_ZCASH_SHIELDED`. `workflow_handlers.py` already gates dispatch.
- `if type(receiver) is not bytes` (`get_address.py:88`) and `type(records) is not bytes` (`sign_pczt.py:590`) re-check native return types.

Upstream handlers (Stellar, Solana) do none of this. Delete these checks and the tests that exist only to cover them (e.g. `test_direct_feature_disabled_invocation_fails_closed`).

### 8. The same code repeated across `get_address.py` / `get_viewing_key.py` / `sign_pczt.py`
- The weak-backup block (`show_warning(br_name="zcash_weak_backup", …)` + `require_session`) appears three times: `get_address.py:61-67`, `get_viewing_key.py:101-107` and `sign_pczt.py:354-360`. Move it to `helpers.warn_if_weak_backup(session)`.
- The "call native, then `utils.zero_unused_stack()` in `finally`" wrapper is written five times (`get_address._call_native`/`_derive_receiver`, `get_viewing_key._call_native`/`_derive_viewing_key`/`_derive_seed_fingerprint`, `sign_pczt._cancel_native`, and inline at `:477-489`, `:585-588`). The `_call_native` + `_derive_*` pair adds two layers per handler. Keep one test seam per module and one wiping helper, e.g. `helpers.call_wiping_stack(fn, *args)`.

### 9. Two vocabularies for the same native concepts
- The crate's `Event::{None, ConfirmOutput, ConfirmTransparentOutput, Review}` becomes the glue's `Step::{Continue, Output, TransparentOutput, Review(Totals)}`, then Python's `_STEP_OUTPUT/_REVIEW/_TRANSPARENT_OUTPUT`. Use one set of names.
- The crate has two `Network` enums: `ironwood::Network` and `ironwood::receive::Network`. Glue has to map between them (`micropython/zcash.rs:107-110`) and parses network twice (`:28-30` vs `parse_network` at `:177`). Keep one.
- `RECORD_LEN` (Rust, `sign_pczt.py:94`) versus `RECORD_BYTES` (proto, trezorlib). `MAXIMUM_FEE` versus `MAX_PCZT_BYTES`/`MAX_ACTIONS`. Settle on `*_BYTES` and `MAX_*`.
- In `session_feed`'s transparent payload, "kind" means P2PKH/P2SH, while the enclosing tuple's "kind" is the step type (`zcash.rs:467-468`). Call the inner one `script_type`.

### 10. The AUX1 floor is applied twice, so `--require-free` has no real caller
- `core/Makefile:74-87` passes `--require-free AUX1_RAM=4096` and says so: "Stated here as well because this is what CI reads". `xtask/src/options.rs` already applies the same floor to the same models automatically (`memory_requirements`).
- That duplicate is the only caller of the new `--require-free` option (commit `f3ce2e421d`). It is also the only reason for the "a twice-named region is held to the larger floor" merge logic in `memusage.rs`.

Drop the Makefile block. Then either drop `--require-free` and its merge logic and keep the automatic floor, or keep only the generic flag and drop the automatic one. Either way there is one mechanism.

### 11. `trezorlib.zcash` API shape
- Two public functions cover one message: `export_viewing_key` (`zcash.py:190`, returns a custom `ViewingKeyExport`, not in `__all__`) and `get_viewing_key` (`:239`, in `__all__`, a one-line wrapper without `@workflow`). Upstream has one `get_<thing>` per `Get<Thing>` message and returns the message or its field. Keep a single `get_viewing_key(session, network, account, include_seed_fingerprint=False) -> messages.ZcashViewingKey`.
- `_check_encoded_text(..., kind: t.Literal["address"])` (`:325-345`) has a parameter with exactly one possible value and a one-entry dict of dicts. Inline it as `_check_address`.
- About 140 lines of Bech32m/F4Jumble (`:348-487`) exist to re-encode a UFVK the host has just received from the device. No other trezorlib coin module validates responses this deeply. Consider an HRP/length check like the address check. At minimum, note in the module docstring why this module is the exception.
- `SEED_FINGERPRINT_BYTES` is public but missing from `__all__`. Upstream coin modules have no `__all__`; drop it.

### 12. The README leaves out the two facts a maintainer needs first (`core/src/apps/zcash/README.md`)
- It never says that the signer signs **Ironwood** actions, or how that relates to "Orchard receivers" (Ironwood reuses Orchard keys and receiver typecode `0x03`). A reader who arrives from the linked "Zcash Ironwood" doc cannot connect the two. Add one sentence.
- `:8-9` says "Transparent Zcash transactions are handled by the Bitcoin app". Yet `signer.py` and `hasher.py`, the v5 transparent signer that `apps.bitcoin` dispatches to, live in this directory, and the README does not list them. Add a "Transparent (via apps.bitcoin)" bullet for `signer.py`/`hasher.py` next to `unified_addresses.py`/`f4jumble.py`.

### 13. Design doc (`docs/common/zcash-ironwood-signing.md`): PR and process text in a permanent doc
- `:57-61`: the `support.json` paragraph ("External contributors also do not edit that file unless asked") is a note to the reviewer. Move it to the PR.
- `:200-201` ("which the old `.buf`-resident region was not") is history. Delete it.
- The heading "7. Decided display and admission policy" should drop "Decided".

---

## Nits

1. `core/embed/ironwood/src/lib.rs`: about 25 `#[cfg(feature = "test")]` attributes are interleaved through the production surface (`Engine`, `Pending`, `PendingSlot`, and imports at `:24-80`, `:598-1000`). Move the test-only reference into `src/engine.rs` behind one `#[cfg(feature = "test")] mod engine;`.
2. Comments that are too long for the point they make:
   - `core/src/apps/debug/__init__.py:456-463` (8 lines explaining 4 extra `GcInfo` items). One line is enough.
   - `core/src/storage/cache_common.py:24-28` repeats the `has_weak_backup` docstring. Keep `# Zcash weak-backup predicate (0/1), per boot.`
   - `core/embed/upymod/rustmods.c:56-65`: the 10-line essay ending "Say so here rather than at the first sign on the first device." The `_Static_assert` message already says it.
   - The workspace `Cargo.toml:81-88` comment ("FAST", "tens of minutes") is repeated in `ironwood/Cargo.toml:10-16` and `rust/Cargo.toml` (`zcash_shielded` feature). Keep it in the crate manifest only.
   - `micropython/zcash.rs:370-379` and `:491-495` (the `debug_region_info` doc, "None on the emulator is not an omission"). Two lines.
3. `xtask/src/options.rs:291-310`: the 20-line `ZCASH_SHIELDED_MODELS` doc includes release trivia ("T3T2 is unreleased… 1463.5 KiB"). Keep "STM32U5 models with room for the 40 KiB region". `zcash_shielded_models_phrase()` (`:330`) has one caller, so inline it. Eleven xtask tests for one flag is heavy next to `miniscript`'s. `a_…`/`the_…` test names don't match upstream's `rejects_…` style.
4. `micropython/zcash.rs:36-40` and `:81-85` are the same 5-line comment. `:205-209` ("…if a future path installs one") is speculative. `:102` ("Nothing but goldens has ever held the two together") is narration.
5. `session_cancel` is registered as `obj_fn_var!(0, 1, …)` (`zcash.rs:490`), but every caller passes one argument (possibly `None`). Make it `obj_fn_1!`.
6. `br_name="confirm_memo"` (`sign_pczt.py:240,252`) is the only br_name without the `zcash_` prefix.
7. `sign_pczt._call` (`:98`) is vague next to trezorlib's `_call`. Name it `_call_with_timeout`.
8. The translation key `zcash__memo` holds "Memo (text)", next to `zcash__memo_hash` = "Memo (hash)". Use `zcash__memo_text`.
9. Test names use four terms for one thing: `fvk`, `full_viewing_key` and `viewing_key` all appear (`tests/zcash_tests/test_get_viewing_key.py:37 test_device_fvk_matches_fixture`, `core/tests/test_trezorzcash.py:53-88`, `test_zcash_client.py` "get_viewing_key" vs "export_viewing_key"). Use `viewing_key` throughout.
10. The proto `ZcashViewingKey` doc (`messages-zcash.proto:77-101`, 25 lines) repeats doc §7. Keep 4 lines and point to the doc. `:124-125` "NOT a header, checkpoint, proof…" should be "unauthenticated; policy input only". `:25` "MAINNET/TESTNET" should match the enum's `Mainnet`/`Testnet`.
11. `receive/mod.rs:3-4` ("port… of Trezor's historical `zcash-orchard` branch") is provenance that belongs in the commit (it is already there).
12. Changelog fragments:
    - `core/.changelog.d/+zcash-shielded.added` names `ZCASH_SHIELDED=1` and `Capability_Zcash_Shielded`, which are not user-facing. Use `[T3B1,T3T1,T3W1] Zcash: Support shielded (Orchard) address display and viewing-key export (off by default).` Consider merging it with `+zcash-shielded-signing.added`: one feature, one line.
    - `python/.changelog.d/+zcash.added`: follow the `Coin: …` form used by `7100.added`/`7310.added`, i.e. `Zcash: Support shielded address, viewing-key export and PCZT signing.`
13. Commit messages:
    - Upstream subjects carry `[no changelog]` in the **subject** (`chore(core): update generated files [no changelog]`). This series puts it in the body.
    - `09505f5640` is regenerated output, so upstream would use `chore(common): regenerate …`.
    - `f3ce2e421d`/`6f5e8a599c` are xtask tooling, so `build(core)` fits better than `feat(core)`/`fix(core)`.
    - The `b92ba49d11` sentence "The dependency shape… is for the maintainers to decide; the pull request description carries the details" is a request to reviewers. Put it in the PR, not in history.
    - `6429dc1779` "…lends for its duration and gets back wiped" is awkward. Use "…lends for the session and receives back wiped."
    - No AI attribution or trailers were found in any of the 12 commits.
