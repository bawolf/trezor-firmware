# Adversarial correctness review: `zcash/extapp..7842d7c42d` (23-commit series)

- Reviewer model: **Fable 5.1**, model ID `claude-fable-5-1` (in-session Claude Code subagent). A genuine Fable run, not a substitute.
- Date: 2026-09-26
- Repo/worktree: `/Users/bryantwolf/conductor/workspaces/trezor-firmware/series`, range `zcash/extapp..7842d7c42d` (20 platform commits `e0bbd36a70..b46a40d286`+`2bec506689`, 3 app commits `3d69ea103d`, `4ba6e777d8`, `7842d7c42d`; 139 files, +20,450/-311).
- Baseline for the regression hunt: `07a27d1d76` (`zcash/extapp-signstart`), reviewed earlier by me and by GPT-6 Sol.
- Mode: read-only source review. No build, no `cargo test`, no emulator, no device session. Nothing edited except this file.

## Verdict

**Approve. No must-fix.** The restructure did not drop, reorder or weaken any verification, policy, state-machine, consent-binding or zeroization step of the signing path: a comment-stripped diff of the old `ironwood/src/{stream,digest,session}.rs` against the new `signer/src/{scanner,sighash,session}.rs` shows only renames, `expect()`/`unwrap` → `Result`, `same_bytes` → `subtle::ct_eq`, and progress plumbing (§A). The new `address_n` handling cannot select an undeclared coin type or account, and the network shown is the network signed (§B). The repeated-signature reply is complete and ascending (§C). The orchard/sinsemilla fork changes add progress callbacks and change no arithmetic (§D). The key service's entitlement, path, consent (hold, correct wording, header name), backup and wipe rules are correct as far as source reading goes (§E). The platform fixes close the memory-safety holes I know of; an app can no longer reach `access_unchecked`, an `unwrap!` on its own data, or an unguarded `ipc_send` in Core (§F). Nothing debug- or dev-only reaches a production build from this range; the test-device kernel commits are not in it (§G).

The should-fix items are robustness and hygiene, and three of the four are carried over from the previous review with the authors' documented reasons: the host-`Cancel` desync that kills the app on its next request, `--production --debug` still being accepted by xtask in this series, and the spending-key residue in Core's MicroPython heap. One is new to this review and pre-existing on sdk-wip rather than introduced here: the `SignTypedHash` crypto service ignores the app's declared curve and paths, so the "one curve per app" entitlement that gates the new key service does not gate Ethereum hash signing (§F, S4).

## Findings

### Must-fix

None.

### Should-fix

**S1. A Trezor-level `Cancel` mid-stream still desynchronises Core and the app; the app's next request is lost and, because a progress screen is up during streaming, the app is killed.** (carried from the previous review's S3; documented as "not bounded" in PLATFORM.md §7, not fixed)
- Path: Core's `run()` is at `core/src/apps/extapp/run.py:427` (`ack = await context.call(response, ExtAppMessage)`) while the app is blocked in `wire_request_raw` (`sdk/crates/trezor-app-sdk/src/wire.rs:67-74`) for a `PcztAck`. A host `Cancel` raises `ActionCancelled` out of `run()`. The next host message starts a fresh `run()` (`progress_obj = None`, `run.py:144`) and delivers `WIRE_START`; the app's `call` gets `UnexpectedService` (`service.rs:119-121`), `sign_pczt` returns `Err`, its `Progress` local is dropped → `end_progress()` sees `PROGRESS_SHOWN == true` (`ui.rs`, `end_progress`) and sends `End` → Core hits `run.py:486-487` `die(DataError("Progress not initialized"))` → app stopped; the host's new request fails and the app must be reloaded.
- Not a consent bypass: `IpcRemote::call` refuses a reply from another service, so a forged `Confirmed` cannot arrive as a `WireStart`. Suite does send `Cancel` routinely, though.
- Fix (bounded, either side): Core treats a Progress `STOP` with no screen as an acked no-op (keep `die` for `REPORT`); or the SDK resets `PROGRESS_SHOWN` and stashes the `WireStart` when `IpcRemote::call` sees `UnexpectedService(WireStart)`, then serves it from `wire_receive_wire_start`. The full fix (request ids or an abort IPC) can follow.
- Note the device test `test_sign_pczt_host_cancel` (`sdk/apps/zcash/tests/test_sign_pczt.py:192-204`) exercises the app's own `Cancel` message, which works; it does not exercise the Trezor-level `Cancel`.

**S2. `xtask modular build --production --debug` is still accepted in this series.** (carried; the fix exists on `extapp/xtask-production-rejects-debug` but PLATFORM.md §3.1 leaves it out of the stack)
- `sdk/crates/modular-xtask/src/args.rs:229-249` `resolve_features`: `production` only removes `dev_keys`; `debug` independently enables `trezor-app-sdk/debug`, which turns on the logging transport, the message-printing panic handler (`app_runtime.rs:279-292`), `Error` display with file:line and `uDebug` derives. Nothing secret is printed (the crypto result `uDebug` redacts `Zip32OrchardAccount`, `structs.rs`), but a production-signed image should not carry it.
- Fix: include the one-commit branch (`resolve_features` refuses the combination, with its unit test) in the platform stack.

**S3. Copies of the spending key remain in Core's MicroPython GC heap after the request.** (carried from S1/Sol #3; Rust side now wiped, MicroPython side documented as not wipeable)
- `core/src/apps/extapp/zip32_orchard.py:154-171`: `blake2b(...).digest()` returns immutable `bytes`; every intermediate `I` (master `sk‖c`, three hardened levels) and the returned `memoryview` slice stay in the GC heap until overwritten. `firmware_micropython.rs` (crypto) now wipes the Rust `TrezorCryptoResultRef` and the serialization buffers (`WipedResult`, `WipedBuffers`), but the `bytes` object handed to `ipc_cb` is immutable and unwiped (the bridge comment says so).
- Exposure needs a memory-disclosure bug, but this is the one place seed-derived key material crosses a trust boundary and Core's own HDNode convention is stricter.
- Fix: derive in Rust (`core/embed/rust/src/crypto/…`) into `Zeroizing` buffers and return a `bytearray` that `run.py` `memzero`s after `send_crypto_result`; or add a `blake2b` binding that writes into a caller-supplied `bytearray`.

**S4. (pre-existing on sdk-wip, not introduced by this range) The `SignTypedHash` crypto service signs with secp256k1 for any loaded app, regardless of the app's declared curve and paths.**
- `core/src/apps/extapp/run.py:272-305` and `:628-679`: when `encoded_network`, `encoded_token` or `chain_id` is given, `_sign_typed_hash` builds its keychain from Ethereum's `PATTERNS_ADDRESS` and the network definition (`get_keychain("secp256k1", schemas_from_network, …)`), validates `address_n` against those schemas and signs `data_hash` with no Core screen (`show_progress` only). The app's `curve` and `schemas` from the signed header are not consulted on this branch. So an app entitled only to `zip32-orchard` (this Zcash app, or any other signed app) can obtain an Ethereum signature over an arbitrary 32-byte hash for any Ethereum path.
- This is the sdk-wip design (the Ethereum app draws its own confirmation screens and asks Core to sign the hash), and the lines are unchanged in this range (only `__debug__` guards were added). I record it because the series' key-service story says "run.py allows one curve per app" and the reader will assume the curve is an entitlement; for `SignTypedHash` it is not.
- Fix (upstream proposal, not this series): require `curve == "secp256k1"` and `address_n` to match the app's own schemas on that branch, or restrict the definitions path to apps whose header declares the Ethereum patterns.

### Nits

- N1. `sdk/apps/zcash/src/sign_pczt.rs:40-42, 199-206` `wipe_stack()`: the claim that 24 KiB covers `sign`'s deepest extent (23,484 B per the author's `stack5.py`) is a measurement, not an invariant; a later change to `sign` or to orchard could exceed it silently. Consider a `const _: () = assert!(...)` fed by the stack report in CI, or scrub `stack-size − <current depth>` computed at runtime from the stack pointer.
- N2. `core/src/apps/extapp/zip32_orchard.py:69-77`: the approval is cached before `get_seed()`. If the user holds to confirm and then cancels the passphrase prompt, the next request from the same instance skips the consent screen and goes straight to the passphrase prompt. The user did consent, so this is acceptable; cache after `get_seed()` if you want the screen and the key to be one transaction.
- N3. `core/src/apps/extapp/run.py:76-78` `_path_schemas`: `pattern.split("/")[2]` and `component[-1]` raise `IndexError` (not `DataError`) for a pattern with fewer than three components or an empty one. Header data is signed, so low impact. (carried)
- N4. `run.py:427-437, 516-526`: `ack.instance_id` is never re-checked against the cached instance on `WIRE_CONTINUE`/`WIRE_ERROR` acks. One app instance at a time makes it moot today. (carried)
- N5. `core/embed/api/trezor_api_v1.h:133-134` / `sdk/crates/trezor-app-sdk/src/low_level_api/api.rs` `rng_fill_buffer`: appended to `trezor_api_v1_t` without a version bump; an app built with this SDK on an older coreapp would call `Some(garbage)`. The authors document that v1 is unreleased and has grown in place before (PLATFORM.md §2.10); once v1 freezes this belongs in `coreapp_api_get(2)`. (carried, downgraded)
- N6. `sdk/apps/zcash/signer-tests/tests/bundle.rs:148` `test_unused_fields_parse` (`#[ignore]`): the Session admits non-canonical Sapling/Ironwood `bsk` and a Sapling anchor that upstream `Signer::new` refuses. Both are outside the sighash; the host applies the device's signatures to its own PCZT, so nothing signed is affected. A user decision, as APP.md says.
- N7. `sdk/apps/zcash/src/sign_pczt.rs:262-285` `payment_address`: a host-supplied unified address is shown if it decodes for the network and its Orchard receiver equals the verified one; the host can still add receivers of its own (a transparent one, say) to the shown string. Funds go to the verified receiver, and a user comparing against the recipient's address sees the mismatch; informational. (carried)
- N8. `core/src/apps/extapp/zip32_orchard.py:100-115`: the consent screen shows `image.name()` but not the vendor. In production the header is Trezor-signed; on dev-key firmware the name is attacker-chosen. (carried)
- N9. `sdk/apps/zcash/src/paths.rs:11` `MAX_ACCOUNT = 100` duplicates Core's `account` keyword range (`core/src/apps/common/paths.py:87`); if Core's range ever changes the app's error text ("Forbidden key path") and Core's refusal desynchronise harmlessly.
- N10. `sdk/crates/trezor-app-sdk/src/app_runtime.rs:246-254`: `LlffHeap::init` asserts on a nonzero heap smaller than one block; `heap-size` has no minimum in xtask. (carried)
- N11. Emulator app images built before `dfbb8392d9` carry `data_size = 0` and now get a zero heap, so they die on the first allocation with an opaque "Task stopped". Documented ("must be rebuilt"); a loader-side `TS_EINVAL` with a message would save someone an hour.

## A. Regression hunt: old signer vs new signer

Method: `git show 07a27d1d76:sdk/apps/zcash/ironwood/src/{stream,digest,session,lib,hedge}.rs` diffed against `sdk/apps/zcash/signer/src/{scanner,sighash,session,lib,memo,hedge}.rs` with comments stripped and the `Error::x()` constructors rewritten to the new variants. Result: the grammar (`scanner.rs`), the digest tree (`sighash.rs`) and the per-action/bundle checks (`session.rs`) are line-for-line the same code apart from the items in the table. The old `wire.rs` reference scanner, `effects.rs`, `Engine`/`validate`/`verify_bundle`, `receive/*`, `seed_fingerprint`, `Session::cancel` and the `test` feature are gone; none of them was on the device path.

| Check | Old location (07a27d1d76) | New location (7842d7c42d) | Change |
|---|---|---|---|
| FVK equality for real spends, byte-for-byte, before parse | `session.rs:960-965` `same_bytes` | `session.rs:775-783` `ct_eq` | constant-time compare kept (subtle) |
| Dummy spend parsed with its own wire FVK | `session.rs:960-961` | `session.rs:775-776` | none |
| Identity `rk` refused before parse | `session.rs:981` | `session.rs:786` | none |
| `Spend::parse`/`Output::parse`/`Action::parse` | `session.rs:1097-1134` | `session.rs:877-919` (`parse_with_progress`) | progress only |
| Input/output running totals, `MAX_MONEY` | `session.rs:986-987`, `lib.rs:880-885` `add` | `session.rs:790-791`, `lib.rs:219-224` `add_amount` | none |
| Duplicate nullifier | `session.rs:989-990` | `session.rs:792-796` | none |
| `verify_cv_net` | `session.rs:992` | `session.rs:797` | none |
| ivk cache built once from device FVK, wiped on drop and at review | `session.rs:998-1001, 209-227, 547-549` | `session.rs:799-803, 171-179, 448-450` | `get_or_insert_with` fallbacks → `ok_or(Internal)` (stricter) |
| Nullifier ownership (`verify_nullifier_with_progress(Some(fvk), Some(classifier))`) | `session.rs:1005-1016` | `session.rs:804-807` | none |
| `verify_rk(Some(fvk))` | `session.rs:1018-1021` | `session.rs:809-812` | none |
| `cmx` ↔ note (`verify_note_commitment`) | `session.rs:1024-1027` | `session.rs:814-817` (`_with_progress`) | progress only |
| Dummy needs `spend_auth_sig`; real must not carry one (Policy) | `session.rs:1030-1042` | `session.rs:819-831` | none |
| Sighash action effects | `session.rs:1043-1051`, `digest.rs:113-127` | `session.rs:832-840`, `sighash.rs:106-120` | none |
| Recipient scope via device classifier; outgoing scope | `session.rs:1053-1063` | `session.rs:841-847` | none |
| Note-encryption recovery: pk_d/esk bound to cmx-validated note; OCK check when present; OVK rules (payment must recover under external OVK; change may omit OVK but must not recover under external) | `lib.rs:1142-1235` `verify_encryption` | `memo.rs:112-182` `recover` | none |
| Memo rules: padding empty/zero; change `0xF6` only; payment classified | `lib.rs:1168-1176` | `memo.rs:130-138` | none |
| Memo display filter (`renders_faithfully`, 256-byte budget, NUL, UTF-8, digest otherwise) | `lib.rs:154-216` | `memo.rs:57-107` | none |
| Padding counted, not shown; change not shown | `session.rs:1069-1088` | `session.rs:852-871` | none |
| Header: v6 version/group, branch = NU6.3 from reference height and on wire, coin type = network, lock_time 0, expiry in (ref, ref+window] | `session.rs:804-821` | `session.rs:657-678` | none; `Policy::new` (`lib.rs:93-116`) also checks the branch up front |
| Transparent: no inputs, ≥1 output if present, ≤31, P2PKH/P2SH only, value > 0, redeem/bip32/proprietary empty, `user_address` ignored | `stream.rs` `transparent`/`transparent_output` | `scanner.rs:377-412` | none |
| Action cap 32 shared with transparent outputs; zero actions refused | `stream.rs` `shielded` | `scanner.rs:417-431` | none |
| Transparent rows held until the action count is known | `session.rs:428-429, 447-467` | `session.rs:352-353, 369-381` | none |
| Trailer: flags parse and equal the v3 defaults; anchor valid if present; value_sum ≤ MAX_MONEY, non-negative; note_version V3; no proof; bsk skipped | `stream.rs` `trailer`, `session.rs:571-590` | `scanner.rs:494-508`, `session.rs:464-479` | `expect("valid default")` → `ok_or(Internal)` |
| Dummy signatures verified over the finished sighash | `session.rs:594-602` | `session.rs:481-489` | none |
| ≥1 real spend and ≥1 reviewed output | `session.rs:606-611` | `session.rs:492-497` | none |
| Fee = inputs − outputs − transparent; value_sum = fee + transparent; fee ≤ 0.01 ZEC; balance identity | `session.rs:616-635` | `session.rs:499-520` | none |
| Truncation / trailing bytes / declared length | `stream.rs` `feed` (`start+len+chunk > total`, grammar end == declared end) | `scanner.rs:601-604, 669-672` | none |
| Token = H(session id, counter, network, account, ref height, max fee, window, FVK, sighash, declared len, stream digest) | `session.rs:648-671` | `session.rs:525-544` | field names only |
| Approval: token match, single use, discards stream | `session.rs:691-706` | `session.rs:559-573` | `expect("checked pending")` removed |
| Sign: token re-checked, `ak` re-checked, `rk = ak + [alpha]G` re-checked per record, consent consumed on every exit | `session.rs:718-787` | `session.rs:578-644` | **stricter**: `approved` is cleared *before* signing (`session.rs:589`), so a panic mid-signing cannot leave it set |
| Zeroization: stream, scanner buffer, FVK encoding, records (`take_zeroizing`), ivk cache, token, session id | `Drop`s in `session.rs`, `stream.rs:563-567` | same, `scanner.rs:563-567` | none |
| Hedged nonces: `BLAKE2b("TrezorIrnwdNonce", 0x00‖key)`, per-draw `0x01‖secret‖entropy‖counter‖i` | `hedge.rs` | `hedge.rs` | `from_seed(seed)` → `new(key)`; the app already keyed on the spending key |
| Approval token and consent consumption in the app | `sign_pczt.rs:156-162` | `sign_pczt.rs:128-133` | none |
| Zip32 derivation claims must name the device's seed fingerprint and account path | `lib.rs:411-421` `OwnDerivation::admit` | `session.rs:220-234` `AccountDerivation::check` | none |
| App-side chunk binding: `transfer_id`, `offset`, exact `length`, `PcztAck` or `Cancel` only | `sign_pczt.rs:255-286` | `sign_pczt.rs:220-248` | none |
| App refuses `ConfirmOutput` that is not a payment; `NeedMore` with nothing consumed | `sign_pczt.rs:140-148` | `sign_pczt.rs:110-121` | none |

Also new and stricter: `Policy::new` runs before `account_keys` (`sign_pczt.rs:52-61`), so an out-of-policy request never reaches Core's consent screen; `sign` borrows the spending key by reference and `wipe_stack()` scrubs after `begin` and after `sign` (`sign_pczt.rs:70-73, 131-133`).

What was removed and why it is safe: the `receive/` reimplementation of ZIP-32 PRFs, Sinsemilla and Commit^ivk is replaced by orchard's `address_with_progress` plus a 10-line `dk` derivation (`signer/src/keys.rs:26-40`) and the FF1 module. I checked `dk` against orchard's `derive_dk_ovk` (`orchard/src/keys.rs:377-385`): `PRF^expand(rivk, 0x82 ‖ ak ‖ nk)` with `FullViewingKey::to_bytes() = ak ‖ nk ‖ rivk` (`keys.rs:483-489`), so `encoding.split_at(64)` gives `(ak‖nk, rivk)` and the hash input order `rivk, 0x82, ak‖nk` is right; the first 32 bytes are `dk`. The in-crate test compares `external_receiver` with `fvk.address_at` for three indices; the FF1 test uses orchard's vectors.

## B. `address_n`: can a host pick an undeclared account or coin type, or split shown vs signed network?

No.
- App: `sdk/apps/zcash/src/paths.rs:14-28` accepts exactly `[32', coin', account']` with `coin ∈ {133', 1'}`, hardened account ≤ 100 (`account ^ HARDENED` makes an unhardened or out-of-range index fail the `> MAX_ACCOUNT` test). The network is *derived* from the coin type and used everywhere: `Policy::new(network, …)` (`sign_pczt.rs:52`), the header check `header.coin_type == network.coin_type()` (`session.rs:667`), the account path bound into `AccountDerivation` (`lib.rs:118-125`), the token (`session.rs:531`), labels, address encoding (`unified.rs:13-18`) and the key request `get_zip32_orchard_account(network.coin_type(), account)` (`keys.rs:11`). There is no second network source to disagree with.
- Core: `zip32_orchard.py:80-97` independently requires the `zip32-orchard` curve, `coin_type ∈ {133, 1}`, `0 ≤ account < 2^31`, and a plain `schema.match` against the header's patterns (no relaxed-safety prompt). `run.py:104-110` now parses each pattern with its own coin type (`_path_schemas`), so both networks are declarable; address MACs, which bind one coin type, are refused for such an app (`run.py:316-317, 339-340`), which the Zcash app does not use.
- Regtest shares coin type 1 with testnet, as before.

## C. Repeated-signature output

`Session::sign_records` (`session.rs:599-644`) iterates `pending.records.iter()`, which enumerates the fixed `[Option<Record>; 32]` in index order, pushes one `SignatureRecord { action_index, signature }` per `Real` record and returns `Err` on any failure, so `Signatures` leaves the crate only when every real spend signed. The app maps `records()` 1:1 into `SpendAuthSignature { action_index, signature }` (`sign_pczt.rs:189-196`) and refuses an empty list (`:186-188`). `signer-tests/src/lib.rs:627-647` `assert_signatures_apply` checks the index list equals the real spends of the PCZT and applies each with upstream `pczt::roles::signer::Signer::apply_orchard_spend_auth_signature`, which verifies against the action's `rk` and the upstream sighash; `signing.rs` runs it for 1..=32 actions, with transparent outputs, the SDK redaction view, and every accepted single-byte mutation (`mutations.rs`). I read the test code; I did not run it.

## D. Orchard and sinsemilla fork changes

`git diff e23177a..61704d4` (orchard, the revision I reviewed last time vs the pinned one) touches `src/keys.rs`, `note.rs`, `note/commitment.rs`, `note_encryption.rs` (tests only), `pczt/parse.rs`, `pczt/verify.rs`, `spec.rs`. Every change threads a `progress: &mut dyn FnMut()` parameter down to `sinsemilla::{HashDomain::hash_to_point_with_progress, CommitDomain::{commit,short_commit}_with_progress}`; the old entry points become wrappers with `&mut || {}`. No formula, constant, comparison or error path changes; `from_parts_with_commitment` gains the parameter. The sinsemilla diff `6ca88ff..6607cb0` adds `progress()` after each `K`-bit piece inside the fold (`let acc = (acc + S_chunk) + acc; progress(); acc`) and the unit test `progress_is_reported_after_each_piece` asserts equality with the non-progress result. The number of callbacks is a function of the (public) message length, so the callback adds no secret-dependent timing beyond the existing computation; the keep-alive it triggers (`ui.rs` `Progress::keep_alive`) is gated on wall-clock time, not data. The `computed-generators` feature (`6ca88ff`, not new in this range) is exercised by the authors' `signer-tests --features computed-generators` run against upstream; I did not re-derive it.

## E. Key service

- **Entitlement.** `zip32_orchard.py:89-90`: `curve == "zip32-orchard"`; `run.py:104-107` allows exactly one curve per app, so a key-service app cannot also use BIP-32 services (except the pre-existing typed-hash branch, S4). Path: only `m/32'/{133|1}'/account'`, hardened, `account < 2^31`, matched against the header's schemas (`:91-97`).
- **Order.** Path/curve/coin/account policy → backup type (`has_weak_backup(*mnemonic.get())`, no UI) → consent → `get_seed()` (passphrase prompt if enabled) → seed length. Nothing refusable is checked after the hold except the seed length, which is always 16/32/64 bytes on a device. This fixes the previous review's S4.
- **Consent.** `_confirm_account` (`:100-115`): title `TR.zcash__spending_key` ("Spending key"), text "{app} will receive the spending key of Zcash account #{n} ({Mainnet|Testnet}) and can spend all of its funds." (`core/translations/en.json:3583-3584`), `hold=True`, `verb=TR.buttons__hold_to_confirm`, `ButtonRequestType.Other`, `br_name zip32_orchard_account`. `confirm_action` supports `hold` on both layouts (`eckhart/__init__.py:40-55`). `app_name` is `image.name()` → `mod_extapp_AppImage_name` → `app_arena.c:339` copies `entry->header->app_name` from the header verified at load (fingerprint → Merkle proof → signed root packet). Vendor not shown (N8).
- **Approvals.** Keyed on `(instance_id, coin_type, account)` in the session cache (44 B slot, newest 8; `:118-144`, `cache_codec.py`, `cache_thp.py`). `instance_id` is a fresh `random.uniform(2**32-1)` on every `load()` including reuse of an already-loaded image (`load.py:130-134`), and is checked against the sessionless cache on every request (`run.py:95-100`), so a reload, a different app or a new session re-prompts. Decline → `ActionCancelled` → `[_RESULT_CANCELLED]` (`run.py:391-392`) and nothing cached. Unit tests cover per-instance/per-account keying and the newest-8 rule.
- **Backup rules.** `has_weak_backup` (`:186-206`): BIP-39 `words < 24` (12/15/18/21 weak), SLIP-39 `len(secret) < 32` (128- and 192-bit weak), `None`/unknown type refused. 15/21-word and 192-bit are now weak rather than refused, as the brief says; ZIP 315's rule is "fewer than 256 bits", which this matches. The app shows the ZIP-315 warning when the bit is set (`src/keys.rs:12-14`, `layout.rs:12-14`).
- **Derivation.** Master `BLAKE2b-512("ZcashIP32Orchard", seed)`, child `PRF^expand(c, 0x81 ‖ sk ‖ I2LEOSP_32(i))` per hardened index (`:154-171`); fingerprint `BLAKE2b-256("Zcash_HD_Seed_FP", len ‖ seed)` (`:174-183`). Vectors from `zcash-test-vectors` and orchard's `from_zip32_seed(bip39 seed, 133, 0)` in `core/tests/test_apps.extapp.zip32_orchard.py:27-58`. 16-byte SLIP-39 seeds get the same construction (Trezor-only extension, documented; no other wallet reproduces it).
- **Wiping.** Rust: `WipedResult` zeroes the `spending_key` field of the result and `WipedBuffers` zeroes both serialization buffers on drop (`firmware_micropython.rs` crypto, new `struct WipedResult`/`WipedBuffers`); the SDK's `Zip32OrchardAccount` zeroes on drop with `write_volatile` + fence (`crypto.rs`). The app wraps orchard keys in `Wiped<T: FlatKey>` using `zeroize_flat_type` (`src/keys.rs:32-71`); I checked `SpendingKey([u8;32])`, `FullViewingKey{ak,nk,rivk}` and `SpendAuthorizingKey(reddsa::SigningKey{sk: Scalar, pk: VerificationKey{point, bytes}})` are plain data with no `Drop` (`orchard/src/keys.rs:42,128,169,323`, `reddsa-0.5.1/src/{signing_key.rs:29-32, verification_key.rs:76-79}`), so the `unsafe impl FlatKey` are sound. Not wiped: MicroPython `bytes` (S3), the app's IPC inbox copy and moved-from copies (documented in the SDK).
- **Exceptions.** Internal refusals are bare `ValueError` → `run.py:393-398` → `result = False` → the app sees `ApiError(Failed)`; `ActionCancelled` → `Cancelled`; malformed request bytes → `ValueError` from `access_crypto_request` inside the `try` → `False`. No wire error is used for control flow; no exception detail reaches the host on this path (the `WIRE_START` `die` at `run.py:142` includes `{e}`, which is an IPC error string, not key material).

## F. Platform fixes

- **Untrusted-input validation.** All three request kinds go through a checked `rkyv::api::low::access` (`structs.rs` `access_crypto_request`, `access_ui_request`, `access_progress_request`); crypto and progress also require the archived variant to equal the IPC id, closing the "valid `End` under the `Init` id" path (Sol #5). `firmware_micropython.rs` maps a failure to `ValueError`; `run.py` turns a bad crypto request into the operation's `False` result (`:190-196, 405-408`) and a bad UI/progress request into `die(DataError(...))` (`:170-177, 453-460`). Static asserts pin reply alignment to the device's 4-byte IPC alignment (`structs.rs` `DEVICE_IPC_ALIGNMENT`). SDK unit tests cover truncation, misalignment at every offset, unknown tags, non-UTF-8 and id mismatch (read, not run).
- **No `unwrap!` on app-sized data in the bridges.** `tstr`/`tstr_opt`/`obj_from_proplist`/`obj_from_strextlist` return `Result`; `MAX_MENU_ITEMS` overflow is `ValueError`; the IPC reply callbacks propagate (`ipc_cb.call_with_n_args(...)?`) and `run.py` wraps `send_ui_result`/`send_crypto_result` in `die` (`:185-188, 411-420`). Remaining `unwrap!`s in `new_send_crypto_result` act on Core-built `result` lists (tag, 32-byte fingerprint, bool, ≤111-byte payloads into 200-byte buffers), not on app data.
- **Can an app still crash Core?** Not that I can see from source. The app-controlled inputs into Core are: IPC request bytes (checked access), UI string sizes (allocation errors → `ValueError` → `die`), progress values (clamped at 1000, `run.py:484`), `WIRE_END`/`WIRE_ERROR` payloads (forwarded to the host as data; the app choosing the `Failure` code is pre-existing), and silence (1 s timeout → `die`, or "Task stopped" if the kernel already killed it, `:161-165`). All `io.ipc_send` calls toward the app are wrapped (`:134-142, 430-437, 496-502, 519-526`, plus the two callbacks via `die` at `:185-188, 411-420`). Kernel `ipc_send` refuses a message that does not fit the receiver's queue (`ipc.c:228-234`) rather than writing past it.
- **Inbox-size stop.** `ipc-buffer-size` is validated to a multiple of 8 in `[256, 64 KiB]` (`metadata.rs:173-217`, unit-tested), exported as `TREZOR_APP_IPC_BUFFER_SIZE` per build (`args.rs:274-280`) and consumed by `option_env!` with a `size_of::<usize>()` alignment assert (`sdk lib.rs:106-121`). A host message that does not fit now stops the app with `DataError("Failed to send IPC message")` on all three forwarding paths, so no request is left half-delivered and the next request cannot receive a stale answer. The only cost is the host having to `ExtAppLoad` again after sending an oversized message to a small-inbox app, which only it can cause.
- **Declared-heap enforcement.** xtask writes `heap-size` into the emulator image's `data_size` (`binary.rs:119-133`, both the x86_64 and the macOS Mach-O arm), and `unix/app_loader.c:85-91` grants exactly that or `TS_ENOMEM`. The SDK allocator now initialises from `app_get_heap` (`app_runtime.rs:246-254`); a zero heap leaves `LlffHeap::empty()` so every allocation fails and the app aborts rather than silently using a static. `app_get_heap` returns `(ptr, len)` instead of a `&'static [u8]` that was being written through. Ethereum's manifest goes to 128 KiB because its tests need it; Tron keeps the 16 KiB it effectively had.
- **Progress API.** `PROGRESS_SHOWN` makes `update_progress` fail locally and `end_progress` a no-op when no screen is up, so an app can no longer trip Core's `die("Progress not initialized")` by mis-ordering; `Progress` ends on drop; `wire_respond_raw`/`wire_error_raw` end a screen left open (`wire.rs:26-28, 45-47`). `keep_alive` reports at most every 100 ms against Core's 1 s limit. The remaining way to hit Core's `die` is S1.
- **RNG.** `SYSCALL_RNG_FILL_BUFFER` admitted for applets (`syscall_dispatch.c:128`); the verifier probes write access (`syscall_verifiers.c:856-866`); the STM32 implementation copies word-wise into a `uint8_t*` so any alignment is fine (`rng.c:67-80`); the SDK exposes `crypto::random_bytes(&mut [u8])`. The app only ever uses it inside `Hedged` (`sign_pczt.rs:377-403`) and for the 16-byte transfer id.
- **run.py logging.** Every `log.*` in `run.py` is under `if __debug__:`; PYOPT=1 builds no longer `NameError` inside the crypto reply callback.

## G. Debug and dev code

- The Zcash app has no `cfg(feature = "debug")` code; the only `cfg` gates in `sdk/apps/zcash/src` and `signer/src` are `#[cfg(test)]` unit tests. The `test` feature (`dev_keys`, SDK mock, debug log level) is used only by `xtask unit-tests` and `build.rs::is_unit_test`. Diagnostics (proto ids 9–10, counters) are gone from this range.
- The SDK `debug` feature adds logging, a panic handler that prints the message, and `Error` display; the crypto result `uDebug` redacts the account. Reachable in a production build only via `--production --debug` (S2). `dev_keys` is removed by `--production` (`args.rs:245-247`).
- The test-device commits (dev root key acceptance, in-kernel ML-DSA-44) are **not** in this range; no file under `core/embed/sec/mldsa44`, `root_packet.c` or `kernel.ld` changes here, so the previous review's M1 does not apply.
- Core: `zip32_orchard.py` has no debug paths; `run.py`'s `__debug__` log lines print exception text and paths, never key bytes.
- Python tests: `conftest.py` is Tron's (wipes the device, loads the "all all all" mnemonic, `apply_settings(experimental_features=True)` on the test device); no global setting is changed by the Zcash tests beyond that inherited fixture. Test-only.

## What I verified (by reading, with the scenario traced to code)

- The comment-stripped old→new diffs above (scanner, sighash, session), the app's `sign_pczt.rs`/`paths.rs`/`keys.rs`/`unified.rs`/`get_*`, the new `memo.rs`/`hedge.rs`/`keys.rs`/`prewarm.rs`/`ff1.rs`/`lib.rs`, the proto, translations and manifest.
- The orchard fork diff `e23177a..61704d4` and the sinsemilla diff `6ca88ff..6607cb0` in full; orchard's `derive_dk_ovk`, `to_bytes`, `verify.rs`, `ScopeClassifier`, and the `recover_output_bound_with_*` signatures.
- `run.py` in full, `zip32_orchard.py` in full, `load.py`, the crypto/UI bridge diffs, `structs.rs`/`crypto.rs`/`ui.rs`/`wire.rs`/`app_runtime.rs`/`api.rs`/`lib.rs` diffs, `metadata.rs`/`args.rs`/`binary.rs` diffs, `rng.c`, `syscall_dispatch.c`, the RNG verifier, `unix/app_loader.c`, `ipc.c` send path, the `AppImage.name` binding.
- `zeroize_flat_type`'s contract against the three orchard/reddsa key layouts.
- Test *code* for `signer-tests` (`lib.rs` harness, `signing.rs`, `mutations.rs`, test lists), the Core unit tests for the key service, and the device-test input flows for the consent and cancel paths.

## What I did not verify

- No build, no `cargo test`, no `xtask`, no emulator, no device. All numbers in APP.md/PLATFORM.md (heap 69,632 B, worst stack 28,796 B, `sign` 23,484 B, image size 224,800 B, 75/75 device tests, 68 passed/1 ignored signer-tests, IPC silence ≤ 551 µs) are the authors' unverified claims.
- Byte-equality of `sighash.rs` with upstream `v6_signature_hash`: relies on `signer-tests` applying the device's signatures through upstream's `Signer` (read, not run).
- The `computed-generators` implementation (`sinsemilla@6ca88ff`) beyond the authors' test run; the 3× slowdown on T3T1.
- `zcash_address` 0.13 decode behaviour for edge-case UAs; `prost` behaviour for `required` fields beyond "absent decodes as default".
- The Python UI fixtures, `ui_tests/` tooling, `pb2py`, `build.rs` protobuf generation, `signatures.json`.
- ARM `binary.rs` RAM sizing and the hardware loader (unchanged in this range).
- The stack-scrub coverage of `wipe_stack` on hardware (N1).
- Whether the emulator's arena-size check elsewhere in `app_arena.c` interacts with the new nonzero `data_size` for emulator images (I read only the loader hunk).

## Author claims checked against code

| Claim (APP.md / PLATFORM.md) | Result |
|---|---|
| "No verification, policy, state-machine, consent-binding or zeroization check of the signing path was weakened" | Confirmed by the stripped diff (§A) |
| `Session::sign` clears the approval before signing | Confirmed, `session.rs:589` |
| `Policy::new` runs before the key request | Confirmed, `sign_pczt.rs:52-61` |
| `wipe_stack` scrubs 24 KiB after `begin` and after `sign` | Confirmed as code; coverage is by measurement (N1) |
| `Wiped<T: FlatKey>` is sound for the three orchard keys | Confirmed (§E) |
| `address_n` replaces network+account; network from coin type; accounts 0..100 | Confirmed, `paths.rs` |
| `PcztAck` fields optional so an absent offset is not read as 0 | Confirmed, `zcash.proto:74-80`, `sign_pczt.rs:241-243` |
| Consent: hold, "Spending key" title, template with app name, account #n, network | Confirmed, `zip32_orchard.py:100-115`, `en.json:3583-3584` |
| `app_name` from the header verified at load | Confirmed, `app_arena.c:339` |
| Backup type checked before the screen; seed length after (always valid on device) | Confirmed, `zip32_orchard.py:66-77` |
| 15/21-word BIP-39 and 192-bit SLIP-39 now weak, not refused | Confirmed, `:186-206` and unit tests |
| Rust result and buffers wiped; MicroPython bytes and app inbox not wipeable | Confirmed |
| Progress/UI/crypto requests validated with checked access and id binding | Confirmed, `structs.rs`, bridges |
| Oversized host message stops the app on every forwarding path | Confirmed, `run.py:134-142, 430-437, 519-526` |
| Emulator grants exactly `heap-size` | Confirmed, `binary.rs`, `unix/app_loader.c:85-91` |
| STM32 `rng_fill_buffer` unaligned-safe; verifier checks writability | Confirmed, `rng.c:67-80`, `syscall_verifiers.c:856-866` |
| Cancel-desync not fixed, documented | Confirmed (S1) |
| `xtask-production-rejects-debug` not in the series | Confirmed (S2) |
| Test-device commits not in the series | Confirmed (§G) |
| `test_unused_fields_parse` ignored: bsk/Sapling anchor non-canonical accepted | Confirmed, outside the sighash (N6) |
