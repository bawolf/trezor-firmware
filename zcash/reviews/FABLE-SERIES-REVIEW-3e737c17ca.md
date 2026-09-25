# Fable adversarial review of the upstream-shaped Zcash series at `3e737c17ca`

Subject: `/Users/bryantwolf/conductor/workspaces/trezor-firmware/upstream-series`, branch
`zcash/ironwood-upstream`, head `3e737c17ca`, base `upstream/main` = `148e530180`,
`git diff 148e530180..3e737c17ca` (131 files, +26,966 / −222), nine commits in four
PR groups: A common/protobuf (`6fcad8234c`, `865e122b6e`); B python (`5197407aae`,
`88684ff5e2`); C receiver (`45abc1f930`, `3861c0f640`, `5592be9fde`); D signing
(`036cdc6043`, `3e737c17ca`). Read as a SatoshiLabs core maintainer would before merge.

Reviewer model: **Fable 5.1 (`claude-fable-5-1`)**, the session model. Three delegated
sub-reviews (host library, cross-app impact, upstreamability) were launched as
in-session `general-purpose` agents inheriting the session model; the SDK does not
report their model back, so it is recorded as "inherited, not independently verified".
The upstreamability agent never returned (transcript idle since 01:23, no live
processes); its scope was covered directly and is marked as such below. Every claim
from the two agents that did return was re-derived before it was kept; one of their
must-fixes was **dismissed** on emulator evidence (see "Dismissed").

Read-only: the worktree was not modified (`git status --porcelain` empty at start and
end, HEAD `3e737c17ca`). Two emulator images were rebuilt under `core/build-xtask/`
(ignored build output) because the artifact present at start predated PR D; nothing
was flashed or pushed. Context read: `docs/shippability/UPSTREAM_SERIES.md`,
`MERGEABILITY_REVIEW.md`, `docs/THREAT_MODEL.md`, the firmware tree's
`docs/common/zcash-ironwood-signing.md`, the prior `FABLE-REVIEW.md` and
`FABLE-REVERIFY-1C.md` (for context only; nothing below is repeated from them without
re-derivation). Date: 2026-09-24.

---

## Verdict

| Group | Verdict | Why |
|---|---|---|
| **A** protocol | **READY-AFTER-MUST-FIX** | Generated content is reproducible (`templates_check` 0, `make protobuf` no-op per §5 of UPSTREAM_SERIES, re-confirmed by the hook run); blocked only by the PROVISIONAL banners that must move out of the shipped files (M3) and the duplicated changelog fragment (S13). |
| **B** trezorlib | **READY-AFTER-MUST-FIX** | Transfer loop and record parser are sound; blocked by process-pinning tests and provisional narration in shipped code (M3) and non-reproducible binary fixtures (M5). The host agent's "post-terminal Cancel invalidates the session" must-fix is **dismissed** (below). |
| **C** receive + build | **NOT-READY** | Two `git+` forks in `[patch.crates-io]` (M1) and a red `cargo vet` (M2) are dependency decisions no edit in this tree resolves; naming/layout (S9) is what a maintainer will ask for next. Everything else in C is in order. |
| **D** signing | **NOT-READY** (depends on C) | The signing core has **no must-fix**: every rule in the in-repo design note runs before any signature and the isolation model holds. What blocks D on its own is the consent screen carrying untranslated literals (M4), and two design questions a maintainer will raise before reading the cryptography: the recipient display for multi-receiver UAs (S1) and the 5 s per-chunk timer (S2). |
| **Overall** | **NOT-READY** for upstream submission; **signing-safety: no must-fix**; safe to flash on the dedicated test Safe 5 with the test seed: **yes**. |

---

## What ran

All commands in `/Users/bryantwolf/conductor/workspaces/trezor-firmware/upstream-series`
with `/tmp/ironwood-build-env.sh` sourced; `-j 2` / `CARGO_BUILD_JOBS=2`;
`HOST_CC=/usr/bin/clang CFLAGS=-Wno-error` for emulator builds.

| # | Command | Result | Exit |
|---|---|---|---|
| 1 | `cd core/embed && cargo test -p ironwood --features test -j 2` (debug profile, **corpus included**) | lib 6, conformance 53, digest_equivalence 8, hedged_nonce 5, receive 11, region_budget 1, request_disposal 3, seed_fingerprint 3, session_equivalence 12 (10,654 s), stream_equivalence 9 (450 s), transparent_outputs 13, doctests 0: **124 passed, 0 failed**; wall 6h08m | 0 |
| 2 | `cd core && uv run xtask test trezor_lib` | **88 passed, 1 ignored** (all 11 `ironwood::arena` tests pass on the host) | 0 |
| 3 | `cd core/embed/xtask && cargo test` (cross-app agent) | **53 passed** | 0 |
| 4 | `cd python && uv run --frozen pytest tests/test_zcash_client.py tests/test_zcash_protocol.py -q` (host agent) | **204 passed** | 0 |
| 5 | `cd core/embed && cargo vet --locked` (cross-app agent) | **Vetting Failed, 116 unvetted dependencies**; `supply-chain/` untouched | non-zero |
| 6 | `make -C core build_unix_frozen TREZOR_MODEL=T3T1 IRONWOOD=1 PYOPT=0` (head, debuglink, frozen) | built in 9m52s; binary now contains the PR D strings (`PCZT transfer timed out`, `Zcash PCZT rejected`: 2 hits; 0 in the artifact present at start) | 0 |
| 7 | `core/emu.py --disable-animation --headless --temporary-profile -c pytest tests/device_tests/zcash tests/device_tests/test_basic.py::test_capabilities --ironwood --ui=test --lang=en` on (6) | **35 passed, 1 skipped, 1 ui_missing** (both the device-only region test), **UI hashes match** | 0 |
| 7a | same command on the artifact present at start (built 01:02, from a PR-C-state tree: `strings` shows the receive handlers but no `apps/zcash/sign_pczt`) | 13 passed (receive, viewing key, capabilities), 22 failed with `UnexpectedMessage` on `ZcashSignPczt`, 22 ui_failed. **Artifact staleness, not a series defect**; superseded by (7). Recorded because it is the run that also shows `apps/base.py:423 handle_Cancel` answering an idle device (see "Dismissed"). | 0 (pytest tail) |
| 8 | `make -C core build_unix TREZOR_MODEL=T3T1 IRONWOOD=1 PYOPT=0` (head, non-frozen) then `core/tests/run_tests.sh` over the six Zcash unit-test modules | **6/6 OK** (`test_apps.zcash.{sign_pczt,get_address,get_viewing_key,ironwood_account,unified_addresses}`, `test_trezorironwood`) | 0 / 0 |
| 9 | `bash docs/git/hooks/commit-msg` over each of the nine messages | **9/9 exit 0**; message lengths 14–160 lines; no `Co-authored-by`, no tool attribution | 0 |
| 10 | `make -C core templates_check` (includes `translations/cli.py merkle-root`) | clean | 0 |
| 11 | `ruff check` / `ruff format --check` (ruff 0.15.10 from `/opt/anaconda3/bin`, not the project venv) over the 21 series Python files; `uv run flake8 core/src/apps/zcash python/src/trezorlib/zcash.py` | all pass / 21 already formatted / clean | 0 / 0 / 0 |
| 12 | Map-file arithmetic on `core/build-xtask/artifacts/T3T1/firmware.map` (Ironwood production build, 00:39) | `.heap 0x3004d110 0x3aaf0` = **240,368 B** = AUX2 330,752 − `.stack` 32,768 − `.buf` 57,616; `.zcash_region 0x30024d40 0xa000` in AUX1; AUX1 free after it **4,288 B** | n/a |
| — | Not run: a stock (non-`--ironwood`) firmware/emulator build (would overwrite the shared artifacts; the stock claim is verified from the linker script + map instead), `pyright`, `cargo audit`, `make protobuf_check` (macOS `cp -T`), hardware. | | |

---

## 1. Signing safety (`core/embed/ironwood`, `core/embed/rust/src/ironwood/signing.rs`, `core/src/apps/zcash/sign_pczt.py`)

**No must-fix.** Every rule in `docs/common/zcash-ironwood-signing.md` §3–§7, §11, §13 is
implemented and runs before `Session::sign` can produce a byte. Map of rule to code,
re-derived from the head:

| Rule (doc §) | Where it runs | Note |
|---|---|---|
| Init: network/account/height/length validated; FVK + `expected_ak` from seed, seed borrowed once (§4) | `signing.rs::begin` 259–299; `Policy::new` (`lib.rs` 466–475) requires `branch_for_height == Nu6_3`; `Limits::new` bounds fee ≤ MAX_MONEY, window > 0; `Scanner::new` caps `declared_len ≤ 65,536` | `sign_pczt.py:330-337` re-checks length 1..65,536 and height ≤ u32 before touching the seed |
| Header: v6 version/group, branch = Nu6_3 and = derived, coin type, lock_time 0, expiry in `(h, h+window]`, `tx_modifiable == 0`, empty proprietary, Orchard tag absent, Sapling absent-or-empty, transparent bundle zero inputs / ≥1 output / ≤31 (§4, §13) | `stream.rs::header/transparent/empty_sapling/shielded`; `session.rs::Body::new` 826–851 | expiry 0 ("never expires") is refused because `expiry > host_reference_height ≥ 0`; `Digest::new` fixes the hashed version constants to V6 after admission enforced equality |
| Transparent output: value 1..MAX_MONEY, exactly P2PKH-25 / P2SH-23, no redeem script, empty bip32 map, bounded UTF-8 `user_address` ignored, empty proprietary; hashed as `TxOut::write`; shown only after the joint cap is known (§13) | `stream.rs::transparent_output` 447–470; `digest.rs::transparent_output`; `session.rs::advance` holds rows until `Item::Shielded`, `next_transparent` releases one per call with 0 bytes consumed | Python loop accepts a 0-consumed kind-3 and rejects any other 0-consumed step (`sign_pczt.py:568-569`) |
| Action: grammar; canonical fields via `Spend::parse`/`Output::parse`/`Action::parse` (V3 note version); real spend ⇒ wire FVK `same_bytes` session FVK and no signature; dummy ⇒ signature present; identity `rk` refused; zip32 claim on spend or output must equal `(own seed fp, m/32'/coin'/account')` or be absent; `verify_cv_net`; nullifier ownership under device FVK; `verify_rk`; `verify_note_commitment`; duplicate nullifier; running hashers; classify recipient; `verify_encryption` (§4 steps 1–9) | `session.rs::Body::action` 890–1007 in exactly that order; `OwnDerivation::admit` (`lib.rs` 400–413) is `same_bytes` + exact 3-index path | `records[index]` is written only after `verify_encryption` succeeds; any error → `Session::feed` → `reset()` → stream and records zeroized (`Records: ZeroizeOnDrop`, `Scanner::drop` wipes the section buffer) |
| Memo policy (§11): padding ⇒ `0xF6…` or all-zero; change ⇒ exactly `0xF6…`; payment ⇒ text within 256 B and `renders_faithfully`, else BLAKE2b-256 digest; recovered from the digest-committed `enc_ciphertext` under `pk_d/esk` bound to the cmx-verified note | `lib.rs::verify_encryption` 771–843, `classify_memo` 176–206, `renders_faithfully` 134–173; fork `note_encryption.rs::recover_bound_inner` checks `domain.rho == note.rho`, version byte, diversifier, value, rseed | Supplementary-plane, bidi, NBSP, controls other than `\n` all hash instead of display; pinned by `conformance::memos_the_device_cannot_render_faithfully_are_hashed` and the `memo_supplementary_plane`/`memo_bidi_override` device vectors (run 7) |
| OVK policy (§11): payment must recover under external OVK; change may carry no OVK (`OvkPolicy::Sender`) but a change output the *external* OVK recovers is refused | `lib.rs` 814–839 | `conformance::change_without_ovk_is_accepted`, `internal_change_requires_internal_outgoing_viewing_key`, `nonzero_output_with_discarded_ovk_is_rejected` |
| Trailer: exact Ironwood-V3 default flags, non-negative value sum ≤ MAX_MONEY, optional valid anchor, note version 1, proof absent, `bsk` ignored, grammar ends exactly at `declared_len` (§4) | `stream.rs::trailer` 519–532; `Scanner::feed` 645–650 (`(start == total) != Done` ⇒ Malformed); `review_stream` 592–606 | |
| Bundle checks: ≥1 real spend and ≥1 reviewed output; `fee = in − shielded_out − transparent`; `value_sum == fee + transparent`; fee ≤ 0.01 ZEC; `payment + change + transparent + fee == in` (§4) | `review_stream` 614–643 | `MAXIMUM_FEE`/`EXPIRY_WINDOW` are Python constants (`sign_pczt.py:32-33`), never host-supplied |
| VerifyDummies then token (§4, §6) | `review_stream` 611–616, 645–672: token = BLAKE2b(session ‖ counter ‖ network ‖ account ‖ height ‖ fee cap ‖ window ‖ FVK ‖ sighash ‖ declared_len ‖ `IWStreamBytesV1`(len ‖ every consumed byte ‖ count)) | `conformance::same_digest_anchor_substitution_requires_new_consent`, `equivalent_zero_lock_times_keep_digest_but_require_new_consent` |
| Totals + approval consumes the token; sign takes the slot first, requires `SpendValidatingKey::from(ask) == expected_ak`, `rk == ak.randomize(alpha)` per real record, signs with the hedged RNG, releases records only when all signed; consent consumed on every attempt (§4 Sign) | `Session::approve` 675–690, `sign` 697–708, `sign_records` 710–759; `signing.rs::sign` 380–410 drops the request unconditionally after the attempt | Python: `_confirm_totals` → `require_session` → `session_approve` → `session_sign` with **no `await` between approve and sign** (`sign_pczt.py:574-585`) |
| Hedged nonce (§7, hedge.rs) | `Hedged<DeviceRng>` is the `Session`'s RNG type (`signing.rs:113`), constructed in `begin` from the seed; per draw `BLAKE2b-512("TrezorIrnwdNonce", 0x01‖secret‖entropy[32]‖counter‖i)`; secret/counter zeroized on drop; input and output buffers `Zeroizing` | The prior review's L1 (blunt 64-byte compare) is fixed: `hedged_nonce.rs:104` compares `[..32]` |
| Stack wipe | `utils.zero_unused_stack()` in `finally` after `session_begin` (`sign_pczt.py:453-465`), after every `session_feed` (`:551`), after `session_sign` (`:582-585`), in `_cancel_native` (`:415-421`), and in the receive handlers | `test_every_seed_touching_native_call_is_followed_by_a_stack_wipe` pins BEGIN→WIPE, SIGN→WIPE adjacency |
| Session handle binding | `with_active` refuses a foreign handle **without** dropping the slot (`signing.rs:322-341`); `cancel(Some(h))` is a no-op unless `h` owns; `cancel(None)` is blind and used only pre-begin and on `finally` (`sign_pczt.py:405`) | `test_trezorironwood.py:149-176`, `:213-228` |
| Hold-to-confirm | totals through `layouts.confirm_total` → `confirm_summary` (hold on every model, as Bitcoin); UFVK export `confirm_action(hold=True)` (`get_viewing_key.py:84`) | per-output and memo screens are taps and are explicitly not consent (§4) |
| FVK self-check | `derive_viewing_key` OR-folds the receive crate's 96 bytes against `signing::orchard_full_viewing_key` before release, zeroes on mismatch (`micropython/ironwood.rs:96-130`) | covers the 16-byte SLIP-39 seed; `test_device_fvk_matches_fixture` passed in run 7 |
| Transparent-output warning | one `show_warning(br_name="zcash_transparent_payment", Warning)` before the first row (`sign_pczt.py:229-247`, `:553-556`); totals repeat "Public amount" | `test_transparent_outputs` vectors 1/2/4 outputs + testnet passed |
| 32-action cap and expiry | `stream.rs::shielded` (`actions > MAX_ACTIONS − transparent_outputs` ⇒ `Capacity`), `transparent` (`> 31` ⇒ `Capacity`); `Body::new` expiry window | |

**Can a host obtain a signature over data the user did not see?** No path found. The
sighash commits to every action's `cv_net, nf, rk, cmx, epk, enc, out` and the
transparent `(value, script)` rows; every value-bearing output is either shown
(payment, including a payment to the wallet's own external address) or proved to be the
device's own internal address (change) with an empty memo; `cv_net` binds the shown
value to the commitment; the note is recovered from the very ciphertext the digest
hashed. A host inventing a real-spend note it does not own can only inflate the *input
total* (and therefore the displayed fee, bounded at 0.01 ZEC) of a transaction the chain
will reject — the hardware-wallet-universal limit, not a defect here.

**Dummy-spend soundness** (re-derived, since the doc tells reviewers not to "fix" it):
a value-0 spend keeps its host-chosen FVK; `verify_nullifier_with_classifier` (fork
`verify.rs:125-162`) falls back to `fvk.scope_for_address` when the wire FVK differs
from the device's; `verify_rk` uses the host FVK; and the host's signature over the
*finished* sighash is verified in `VerifyDummies`. The identity-`rk` encoding `[0;32]`
is refused before parse (`session.rs:937`). Sound.

### Findings

**S1 (should-fix, design) — the recipient screen cannot be compared against a
multi-receiver Unified Address.** `sign_pczt.py:145-149` renders the verified 43-byte
Orchard receiver as an Orchard-only UA (`u1…`, 106 chars) and `user_address` is ignored
by rule (§13, `stream.rs::user_address`). A wallet's UA normally carries Orchard +
Sapling + transparent receivers (the series' own `USER_ADDRESS_BUDGET` comment says
"about 213 characters"). Scenario: the user has the recipient's UA out of band; the
device shows a *different* string for an honest transaction, so the user cannot tell an
honest recipient from a substituted one, and the per-output screen — the reason the
device exists — carries no information for the common case. Fix that keeps the binding:
accept the host's `user_address` for display only if it ZIP-316-decodes to a UA whose
Orchard receiver equals the verified 43 bytes (the receiver check stays authoritative;
the extra receivers are the recipient's own, not a diversion of this payment); fall back
to the Orchard-only encoding otherwise. Worth a sentence in the design note either way.

**S2 (should-fix) — `_call` races a 5 s `loop.sleep` against every host chunk
(`sign_pczt.py:97-111`).** No other coin app times out a host round trip (`loop.race` +
`sleep` exists only in `reboot_to_bootloader` and the debug app). Scenario: a slow
THP/BLE link on T3W1, or a host that re-encodes a 64 KiB PCZT between chunks, misses
5 s once → the device raises `ActionCancelled("PCZT transfer timed out")`, which the host
sees as a *user* cancel, and the host's late `ZcashPcztAck` arrives as the stale message
the next `Initialize` reports (the AGENTS.md "PCZT transfer timed out" symptom is this
path). The file's own comment already argues autolock bounds an abandoned sign; either
drop the timer or make it a distinct `Failure` class and document it in the trezorlib
docstring.

**N1 (nit)** `signing.rs::sign` checks `owns(handle)` and then `with_active` checks the
same (prior I2, still there). **N2 (nit)** `session.rs:322-324`, `:848`, `Body::new`
comments claim CAP-sized values "never materialise on the stack" because they are
`Box::new(...)`; Rust does not guarantee placement for `Box::new(struct literal)` — it
is measured on hardware (the two-sign device test) but the comment overclaims.
**N3 (nit)** `sign_pczt.py:389-397` maps *any* `ValueError` from the UI layer (e.g. a
layout refusing a string) to `DataError("Malformed PCZT")`; harmless but misleading.

## 2. Memory safety and isolation

Re-derived from `arena.rs`, `allocator.rs`, `allocator_unix.rs`, `micropython/ironwood.rs`,
`prewarm.rs`, the linker scripts and the map file.

- **Tiers.** Rooted tier = `static REGION: [u8; 40 KiB]` in `.zcash_region`, formatted
  once (`Arenas::root` is a no-op once live), never freed; scratch tier = the workflow's
  `bytearray(48 KiB)` installed at `session_begin` and released at `session_cancel`.
  `alloc` routes to the scratch iff one is installed, never falls back (`arena.rs:280`);
  `dealloc` routes by address and **drops** a pointer into neither tier (a block from an
  already-released scratch) instead of writing through it (`arena.rs:432-440`).
  `release_scratch` uninstalls first, wipes the whole span, and reports any retained
  block, which `allocator::release_scratch` turns into a named fatal ("Ironwood scratch
  retained"). Every freed payload is zeroed before coalescing. Arena tests: 11/11
  (run 2); device traces `rooted_tier*`/`scratch_tier` under `computed-generators` were
  run by the series author (119+48, UPSTREAM_SERIES §5) and not repeated here (they are
  behind `required-features`, not in run 1).
- **Other apps.** The `#[global_allocator]` is compiled only under `feature = "ironwood"`
  on `target_arch = "arm"` (`ironwood/mod.rs`), and `extern crate alloc` in `trezor_lib`
  only under the feature; nothing else in `trezor_lib` uses `alloc` (grep: only the two
  ironwood lines). So no Bitcoin/Ethereum/Solana Rust path allocates through the arena,
  and their MicroPython heap is untouched: `.heap` on T3T1 is 240,368 B in the Ironwood
  map (run 12), the region comes out of a previously free 45,248 B gap in AUX1, and a
  stock build has an **empty** `.zcash_region` (the only input is the feature-gated
  static). T3W1 is the exception by design: one bank, so its heap loses 40 KiB
  (~265 KiB left); T3B1 has ~180 KiB of AUX1 slack.
- **Between sessions.** The scratch is wiped in `release_scratch` before it returns to
  the collector, on every path: normal return (`sign` sets `*active() = None` then the
  `finally` runs `session_cancel(handle)` → idle → release), host cancel, `DataError`,
  and autolock (`GeneratorExit` through the same `finally`). `forbid_installed_scratch`
  at the next `session_begin` makes the one impossible-by-construction case (a workflow
  collected without its `finally`) fatal instead of a write into reused heap. The rooted
  tier holds only public constants (Pasta sqrt table, orchard `CommitDomain` caches);
  `Signing` (FVK, hedge secret, records, token) lives in the scratch and is dropped before
  release. Residue: NOLOAD region across a fatal — documented, same class as stack
  residue elsewhere.
- **Host aborts mid-transfer.** `_call` timeout → `ActionCancelled` → `finally` →
  `_cancel_native(handle)` → `Session::drop` → `reset()` zeroizes records/stream →
  `release_scratch` wipes → `del scratch`. A `Cancel` message → `context.call` raises
  → same path. A disconnect → codec read error → same path. Verified by
  `test_cancel_at_output`, `test_cancel_at_totals_then_sign` (run 7) and the
  session-half of `test_trezorironwood.py` (run 8).
- **N5 (prior, still open as a tripwire rather than a proof).** `prewarm.rs` warms
  `SqrtTables<Fp>` and the two `OnceBox<CommitDomain>`; Pasta's `FQ_TABLES` and any other
  lazy static first touched *inside* a session would be carved from the scratch and
  turn the next `release_scratch` into the named fatal. The `scratch_tier.rs` traces
  (2/8/16/32 actions, 1+31 deshield) assert `release_scratch() == Ok`, so the shapes the
  wire admits are covered; the residual is a code path none of those shapes reach. No
  `pallas::Scalar::sqrt` on the verify/sign path was found (RedPallas signing needs no
  square root; point decompression is `Fp`). Keep N5 as "documented, guarded by a
  fatal, not proven".

**No memory-safety finding.** One nit: `allocator.rs:71`
`#[cfg_attr(not(target_os = "macos"), link_section = ".zcash_region")]` is on an
arm-only module, so the condition is dead.

## 3. Cross-app impact (everything outside the Zcash directories)

Verified directly and by the cross-app agent (its numbers re-checked against the map):

- **Stock build unchanged in behaviour**: `USE_IRONWOOD` is `mp_const_false` unless
  `upymod/build.rs:71` defines it; `MP_REGISTER_MODULE(trezorironwood)` and the GC-block
  `_Static_assert` sit in `#ifdef USE_IRONWOOD` (`rustmods.c:55-71`); `apps/base.py:178`
  appends `Capability.Zcash_Shielded` only under the flag (**stock reports it absent** —
  confirmed by the flag chain; the series author's stock-emulator run in UPSTREAM_SERIES
  §5 agrees); `workflow_handlers.py:246-251` returns the Zcash modules only under the
  flag, so a stock build answers `Failure(UnexpectedMessage)` and never imports
  `apps.zcash.*`; `apps/debug/__init__.py:463-478` appends arena items only when
  `debug_region_info()` is not `None` (device debuglink only). `xtask` refuses
  `--ironwood` for non-firmware projects, `--btc-only`, and any model outside
  `[T3B1, T3T1, T3W1]`; the AUX1 floor and `-Zbuild-std=core,alloc` apply only to
  Ironwood firmware builds; xtask tests 53/53.
- **Three stock-path changes a maintainer will want split out or named (S4, S5, S6
  below)** and **two things that are not gated but should be (N4, N6)**.

**M1 (must-fix, PR C) — `[patch.crates-io]` to personal git forks**
(`core/embed/Cargo.toml:146-148`; `Cargo.lock` has 2 `git+` sources: `bawolf/orchard@d4792911`,
`bawolf/sinsemilla@6ca88ff5`). Upstream's lock has none. The patch is workspace-global:
stock builds do not *link* them (`cargo tree` for `trezor_lib`/`firmware` without the
feature: 0 matches) but every stock `cargo metadata`/build now resolves 119 extra crates
and needs network or a git cache for github.com/bawolf. Documented as PROVISIONAL with the
three shapes offered; still a blocker until one is chosen with maintainers (the fork SHAs
are unchanged since the prior review, whose fork audit therefore still stands).

**M2 (must-fix, PR C) — `cargo vet --locked` fails with 116 unvetted dependencies**
(run 5), `core/embed/supply-chain/` untouched; `.github/workflows/core.yml:1046-1056`
runs `vet_rust` unconditionally, so **every** PR on this base — stock included — goes
red. UPSTREAM_SERIES §5a lists this as a maintainer decision (exemptions vs audits vs
`cargo vet trust`), which is right, but a PR that opens with a red mandatory job is not
reviewable; the decision must precede PR C.

**S3 (should-fix) — the T3T1 AUX1 floor is 192 B from firing.** `IRONWOOD_AUX1_FLOOR =
4096` (`xtask/src/options.rs:326`) against 4,288 B free (run 12). Any unrelated `.bss`
growth over 192 B turns the new `Build firmware (T3T1, universal, normal, ironwood)`
job red on a commit that has nothing to do with Zcash. Lower the region, lower the
floor, or make the Ironwood firmware jobs non-blocking until the budget is agreed.

**S4 (should-fix) — `memusage.rs` and `cargo.rs` changes alter stock builds and are
buried in the feature commits.** The parser now counts wrapped 16-character section
names (`.no_dma_buffers`, 28,628 B on T3T1 the old parser silently dropped) and skips
unparsable lines instead of erroring; `print_memusage` now runs before signing/postbuild
for every non-emulator build. Both are good and both belong in their own PR with their
tests, ahead of A.

**S5 (should-fix) — `cache_common.py:29,134` adds `APP_ZCASH_WEAK_BACKUP` to
`SessionlessCache.fields` unconditionally** (stock and btc-only get a ninth slot). Gate it
on `utils.USE_IRONWOOD` (the frozen sed folds it to a constant) or say why not.

**S6 (should-fix) — `tests/device_tests/test_basic.py:33`** now asserts
`Zcash_Shielded ∈ capabilities == --ironwood`, so the *stock* suite run against an
Ironwood emulator without the flag fails `test_capabilities`. Relax to a one-sided check
or document.

**N4 (nit)** `librust_qstr.h.mako:45-53` gates eight module qstrs under `USE_IRONWOOD`
but `MP_QSTR_SCRATCH_BYTES` and `MP_QSTR_debug_region_info` are emitted unconditionally.
**N6 (nit)** `qstrdefsport.h` adds the `apps.zcash.*`/`ZcashNetwork` qstrs without a
`#if !BITCOIN_ONLY`, consistent with existing altcoin entries but a small btc-only cost.
**N7 (nit)** CI boots an Ironwood emulator on T3T1 only; T3B1/T3W1 have recorded UI
fixtures (123 entries each) that nothing in CI checks — the series says so; a maintainer
will ask for the matrix or for the fixtures to be dropped.

## 4. Host library (`trezorlib.zcash`) — PR B

Transfer loop re-derived (`python/src/trezorlib/zcash.py:520-602`): latches
`transfer_id` on the first request, requires contiguous offsets, `length ==
min(1024, remaining)`, refuses out-of-range slices, sends `Cancel` on any violation,
parses `records` as 66-byte units with pool `0x03`, strictly ascending indices, and
returns records only (the host applies them with the `pczt` crate's
`apply_orchard_spend_auth_signature`, which re-verifies). `@workflow(capability=…)` is
real upstream API at the base (`tools.py:377-386`) and, as the design note says, purely
informational. 204 host tests pass (run 4).

**Dismissed (host agent M1).** The agent held that `_cancel` after a terminal reply
(`ZcashAddress`/`ZcashViewingKey`) gets `Failure(UnexpectedMessage)` from an idle device
and so invalidates the session, and that the tests script an impossible
`Failure(ActionCancelled)`. The emulator log from run 7a shows the opposite:
`received message: Cancel` → `apps/base.py:423 handle_Cancel` → `ActionCancelled` →
`write: Failure` "Cancelled" on an idle device. The tests' transcript is the real one;
`_cancel` leaves `is_invalid` false. Not a finding.

**M3 (must-fix, PR A + B) — provisional narration and process-pinning tests in shipped
files.** `common/protob/messages-zcash.proto:8-33` (26-line "PROVISIONAL, LOCAL-ONLY
SCHEMA" banner, "verified Ironwood donor"), `messages.proto:412-414`,
`messages-management.proto:199-200`, `trezorlib/zcash.py:19-21` (docstring narrating
that the block "is not assigned or reserved upstream yet"), and
`python/tests/test_zcash_protocol.py:44-70, 198-228` (`PROVISIONAL`, `DONOR_WIRE_IDS =
range(32000, 32009)`, `test_schema_declares_itself_provisional` asserting the banner text
exists, `test_donor_identifiers_are_unimplemented`, `test_freed_download_ids_stay_unclaimed`
asserting `reserved 2307, 2308;` for identifiers that were never public). A PR *is* the
allocation request; these tests fail the moment maintainers assign or change the block,
and "donor"/32000 mean nothing upstream. Keep the codec/round-trip/required-field tests
(`:252-427`); move the banner text to the PR description (prior S9, still open, now
mirrored into two more files). `reserved 2307, 2308` should go too — nothing public ever
used them.

**M5 (must-fix, PR B) — binary fixtures with non-reproducible provenance.**
`python/tests/fixtures/zcash/MANIFEST.json:6-15, 67-68` says the generator command "is
not recorded", cites "the lane that owns the pinned builder" and "two donor records";
`pyproject.toml` ships `tests/` in the sdist excluding only `*.bin`, so the `.pczt`
blobs would reach PyPI. The module never parses PCZT bytes (`zcash.py:26-27`) and
`test_sign_pczt_boundary_lengths` already covers the chunk-boundary cases with synthetic
bytes; the in-repo device corpus (`common/tests/fixtures/zcash/sign_pczt.json`, hex,
with provenance) can supply a "real" PCZT if one is wanted. Delete the fixtures and the
manifest tests.

**S7 (should-fix, B)** `zcash.py:319-327` `_call` wraps `session.call` and turns any
`ValueError/TypeError/KeyError/OSError` into a second `Cancel` + `ProtocolError("Malformed
Zcash response")`. `client._callback_pin` raises `ValueError("Invalid PIN provided")`
*after* already cancelling; the wrapper cancels an idle device again and hides the PIN
error. No neighbour wraps `session.call`. Narrow or drop. **S8 (should-fix, B)** the
host enforces firmware-internal constants (`length == min(1024, remaining)` at `:544`,
`MAX_ACTIONS` at `:585, 597`) as protocol invariants — the proto does pin the length
formula, but a device that ever raises the cap would have its *approved* signatures
rejected by an old trezorlib; at least drop `MAX_ACTIONS` from the host. **S9' (B)**
hostile-upload tests script no reply to the `Cancel`, so `_cancel`'s failure branch runs
silently and `is_invalid` is never asserted; no test interleaves a `ButtonRequest`
between `ZcashPcztAck` and the next request, though the device does exactly that.
**S10 (B)** no `cli/zcash.py` while stellar/solana/monero/tron all have one. **N8 (B)**
`__all__` (`zcash.py:47-58`) is stale and no neighbour defines one; `type(x) is not
bool/bytes` pinning is un-idiomatic for trezorlib; "raised 8 -> 32" history in `:71-73`.

## 5. Upstreamability defects (covered directly; the delegated agent did not return)

**M4 (must-fix, PR D/C) — untranslated user-facing strings on consent screens.**
`sign_pczt.py:288` `"Expires at block"`, `:295` `"Total actions"`, `:301` `"Public
amount"`, `:310` `f"Zcash {network_label} {account_label}"`;
`get_viewing_key.py:73` `action=f"Zcash {network_label}\n{account_label}\n{path}"`;
`ironwood_account.py:53` `f"ZEC #{account + 1}"` (label), `:29-33` "Mainnet"/"Testnet".
Every screen string in `core/src/apps` goes through `TR.*`; the translations CI check
does not catch f-strings, so this would ship. The seven `zcash__*` keys that were added
and re-signed do not include these.

**S9 (should-fix; prior S5, S6, N1, still open) — naming and layout against
neighbours.** `apps/zcash/ironwood_account.py` is codename-prefixed and there is no
`layout.py` (stellar, solana, monero all have one) and no `apps/zcash/README.md`
(solana, stellar have one); `docs/common/index.md` has no Zcash entry (only
`SUMMARY.md`). The flag/module/crate/marker family is `USE_IRONWOOD`,
`trezorironwood`, `--ironwood`, `IRONWOOD_MODELS`, crate `ironwood`,
`pytest.mark.ironwood`, `core_ironwood_rust_test`, changelog "Ironwood protocol bindings".
Ironwood is a network-upgrade codename that the next upgrade will make stale; every
neighbour is named after the coin. Expect a rename request to `zcash`/`USE_ZCASH_SHIELDED`
before merge; cheaper to do now than after PR C is reviewed.

**S11 (should-fix) — project-history wording in shipped code and in the forks.**
`core/embed/ironwood/src/lib.rs:4` "for the Safe 5 Zcash application"; `:14-16` "The
pending upstream PCZT still contains viewing-key material; allocator-backed PCZT cleanup
is a required later integration boundary" (no longer true); `:991-992` "Boundary 2 must
measure and remove that extra live allocation before firmware integration";
`xtask/src/options.rs:301` "the boot-lifetime 96 KiB signing REGION" (it is 40 KiB);
`region_budget.rs`, `scratch_tier.rs`, `allocator.rs` "on the first Safe 5 boot";
`tests/device_tests/zcash/test_ironwood_streaming_sign.py:552` "the 2026-09-18
cross-session bug"; "donor" in the proto banner and throughout `python/tests`;
`common/tests/fixtures/zcash/sign_pczt.json` `"datetime": "2026-09-23…"`. In the pinned
orchard fork, which maintainers will read: `keys.rs:609` "(streaming-signing 16/32
integration, MUST-FIX #3)", `note_encryption.rs:151` "MUST-FIX #1 (Fable review)". No
`Co-authored-by`, no AI attribution, no `copenhagen`/`Codex`/`lane`/`receipt` hits in the
diff; four `github.com/bawolf` URLs, all in the `[patch.crates-io]` context.

**S12 (should-fix; prior S8, still open)** `docs/common/zcash-ironwood-signing.md`
starts at §3 and skips §8, §9, §12 with a note saying so; an upstream doc should be
renumbered.

**S13 (should-fix) — changelog fragments.** Five `core/.changelog.d/99999.added*` for one
feature (towncrier renders five bullets); `python/.changelog.d/99999.added` (added by
commit 1, a `common/` commit) and `99999.added.1` (commit 3) say the same thing;
`99999` placeholders as expected pre-PR. One fragment per component, named by PR.

**Verified fine:** commit messages pass the hook 9/9 (lengths 14–160 lines are long for
Trezor but carry the design context deliberately); `messages-zcash.proto` uses
`@start/@next/@end`, `required` for every selecting field, comments on every message;
`MessageType` block at 2300 with `wire_in/wire_out`; `common/protob/Makefile`,
`cointool.py` `ALTCOIN_PREFIXES`, legacy `SKIPPED_MESSAGES` are minimal and correct;
`signatures.json` regenerated `current` only (precedent `95adcc90f4`); the seven keys are
`universal_fw`-gated; `tests/conftest.py --ironwood` skips rather than silently passing;
`pytest.mark.models("t3b1","t3t1","t3w1")`; core unit tests follow `common.py` /
`unittest`; `core/embed/ironwood/Cargo.toml` has `edition = "2024"`, `links`, the
host-only `test` feature with a bare-metal `compile_error!`, `[[test]] required-features`;
`ErrorCode::capacity()` gate is the only content change against the source branch, as
documented. The N5 trailing-blank-line deletion in `core/embed/rust/Cargo.toml` is
**still present** (unrelated hunk in commit 6).

## 6. Status of the six previously open findings

| ID | Was | Now | Evidence |
|---|---|---|---|
| S5 | codename-prefixed `ironwood_account.py`, no `layout.py` | **OPEN** | `ls core/src/apps/zcash`: `ironwood_account.py`, no `layout.py` |
| S6 | no `apps/zcash/README.md`; `docs/common/index.md` not updated | **OPEN** | no README; `grep zcash docs/common/index.md` empty (only `SUMMARY.md:46`) |
| S8 | design doc starts at §3, skips 8, 9, 12 | **OPEN** | `docs/common/zcash-ironwood-signing.md` lines 12–16 |
| S9 | proto PROVISIONAL banner | **OPEN, widened** | banner in `messages-zcash.proto:8-33`; now also `trezorlib/zcash.py:19-21` and three tests that assert it (M3) |
| N1 | `USE_IRONWOOD` vs `USE_ZCASH` | **OPEN** | whole flag/module family still `ironwood` (S9) |
| N5 | deleted trailing blank line in `core/embed/rust/Cargo.toml` | **OPEN** | diff hunk `@@ -172,4 +184,3 @@ … -` (blank) still in commit 6 |

None of the six moved; the series pass was, by its own account, a re-partition of
content with one requested change. All six are cheap and should be done before PR C.

## 7. Ranked summary

**Must-fix** — M1 git forks (C); M2 `cargo vet` (C, blocks every PR's CI); M3
provisional banners + process-pinning tests (A, B); M4 untranslated consent strings (D,
C); M5 non-reproducible binary fixtures (B).

**Should-fix** — S1 multi-receiver UA display (D, design); S2 5 s chunk timer (D); S3
192 B AUX1 margin (C); S4 memusage/cargo.rs stock changes split out (C); S5
unconditional cache slot (C); S6 `test_capabilities` two-sided assert; S7/S8/S9'/S10
trezorlib `_call` wrapper, host-pinned constants, cancel-outcome tests, missing CLI (B);
S9 naming/layout (C, prior S5/S6/N1); S11 project-history wording incl. in the forks;
S12 doc numbering (prior S8); S13 changelog fragments (A/B/C/D).

**Nits** — N1 double `owns`; N2 placement comments overclaim; N3 UI `ValueError` →
"Malformed PCZT"; N4 two ungated qstrs; N5 prewarm coverage is a fatal tripwire, not a
proof (prior N5); N6 btc-only qstr cost; N7 T3B1/T3W1 fixtures unchecked in CI; N8
trezorlib `__all__`/type pinning; `allocator.rs:71` dead `cfg_attr`; prior N5 blank line.

**Dismissed** — host agent's "post-terminal Cancel invalidates the session" (emulator
log shows `handle_Cancel` answers `ActionCancelled` when idle).

## 8. Residual trust boundaries (unchanged, restated)

The device cannot verify note existence or Merkle paths (input total is host-asserted;
bounded by the fee cap and by the chain rejecting the transaction); the host reference
height is a policy input the device cannot authenticate (the digest-bound expiry is what
is shown); the hedged nonce is only as strong as the seed-derived secret if the TRNG is
dead (equal to the RFC 6979 class already on the device); emulator runs do not exercise
the arenas (`allocator_unix.rs` is `malloc`) — the region invariants are asserted by the
host traces and by the one device-only test, which was not run here (no hardware in
this review). The fork diffs (`orchard@d4792911`, `sinsemilla@6ca88ff5`) were not
re-audited line by line; the SHAs are the ones the prior review audited, and the
behaviour-preserving claims were spot-checked at the four call sites the session uses.
