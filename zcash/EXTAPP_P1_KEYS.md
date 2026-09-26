# Extapp P1: ZIP-32 Orchard account keys for the Zcash app

Date: 2026-09-25. Phase P1 of `EXTAPP_PLAN.md`.
- **Worktree:** `/Users/bryantwolf/conductor/workspaces/trezor-firmware/extapp-keys`, branch
  `zcash/extapp-keys`.
- **Base:** `1f71e245e4`, which is `bieleluk/sdk-wip` `4cd93ff4d8` plus the two P0 local fixes.
- Nothing has been pushed.

**Commits.** Local, rewritten after review, author Bryant Wolf, no trailers:
- `c549ab5f18` feat(core): give extapps ZIP-32 Orchard account keys with consent;
- `91fc95c0ac` fix(core): validate extapp IPC requests before reading them.

**Pre-review history.** The original commits `96e21568dc` and `bd69375943` are kept on the local
branch `backup/extapp-keys-pre-review`.

**Checks per commit.**
- `c549ab5f18` builds and passes on its own:
  - SDK `make sdk_fmt_check sdk_check sdk_clippy sdk_test`: 2/2;
  - ruff clean;
  - `build_unix PYOPT=0`;
  - `test_apps.extapp.zip32_orchard.py` and `test_storage.cache.py`: OK.
- The tree at `91fc95c0ac` is byte-identical to the tree on which every check in §4 ran.

## 1. API (the contract for track B)

```rust
// sdk/crates/trezor-app-sdk/src/crypto.rs
pub struct Zip32OrchardAccount {
    pub spending_key: [u8; 32],      // Orchard sk (ZIP 32); zeroed on drop
    pub seed_fingerprint: [u8; 32],  // ZIP-32 seed fingerprint
    pub weak_backup: bool,           // ZIP-315 weak-backup bit
}
pub fn get_zip32_orchard_account(coin_type: u32, account: u32) -> Result<Zip32OrchardAccount>;
// Err(Error::Cancelled) if the user declines; Err(ApiError(Failed)) otherwise.
```

- **Wire.** Crypto op 7 is `GetZip32OrchardAccount { coin_type, account }`. The results are
  `Zip32OrchardAccount { … }` and the new `Cancelled`.
- **Tags.** Core's serializer uses explicit tags:
  - `[0, public_key]`;
  - `[1, sk, fp, weak]`;
  - `[2]` for cancelled.

  In `run.py` the tags are named constants: `_RESULT_PUBLIC_KEY`,
  `_RESULT_ZIP32_ORCHARD_ACCOUNT` and `_RESULT_CANCELLED`.
- **Cancellation.** When the app propagates `Error::Cancelled` with `wire_error_raw`, the host
  sees `Failure(ActionCancelled)`.
- **The app must validate `sk`.** Use `orchard::keys::SpendingKey::from_bytes(sk)` and fail on
  `None`. Core has no Pallas code, so it cannot reject the ~2^-250 invalid-key case. The SDK
  function doc says so.
- **Declaring paths.**
  - An app may now declare both `m/32'/133'/[…]'` and `m/32'/1'/[…]'`.
  - The `account` path keyword means only 0–100. Declare `[0-2147483647]'` for the full range.

## 2. Derivation (core, `core/src/apps/extapp/zip32_orchard.py`)

The derivation is unchanged from the first version and checked against ZIP 32. It uses the seed
from `apps.common.seed.get_seed()` and BLAKE2b only.

- **Master:** `I = BLAKE2b-512("ZcashIP32Orchard", seed)`.
- **Hardened child:** `I = BLAKE2b-512("Zcash_ExpandSeed", c ‖ 0x81 ‖ sk ‖ I2LEOSP_32(i|2^31))`.
- **Seed fingerprint:** `BLAKE2b-256("Zcash_HD_Seed_FP", I2LEOSP_8(len) ‖ seed)`.
- **Seed lengths:** 32–252 bytes, or a 16-byte restored SLIP-39 seed (same construction, with
  length byte 16).
- **`weak_backup`:** BIP-39 with 12 or 18 words, or SLIP-39 with 16 bytes. Other backups are
  refused.
- **Checked against:** the 4 ZIP-32 Orchard test vectors, the zip32 fingerprint vector, and the
  `orchard` 0.15.5 FVK cross-check for "abandon ×11 about", account 0 (§4).

## 3. Consent, entitlement and policy

- **Consent (core-owned).**
  - **When it asks.** The first request for a given app instance, network and account shows a
    `confirm_action` screen:
    - title "Zcash account";
    - text "Allow {app name from header} to use your Zcash account #{account+1} ({Mainnet|Testnet})?
      It can see your balance and create transactions for you to confirm.";
    - verb "Allow";
    - `ButtonRequestType.Other`, br_name `zip32_orchard_account`.
  - **Where approval is stored.** In a new session-cache field, `APP_EXTAPP_ZIP32_APPROVALS`
    (44 B, in both the THP and codec caches, non-BTC only). It holds the app instance id plus the
    newest 8 (coin, account) entries.
  - **When it resets.**
    - A reload gives a new random instance id, so earlier approvals no longer match.
    - Ending the session clears the cache.
  - **Rejection.** Rejecting raises `ActionCancelled`, which reaches the app as `Cancelled`.
    Nothing is cached, so the next request prompts again.
  - **The screen is shown only after the policy checks pass.** A disallowed request is refused
    with no prompt.
- **Entitlement.** Still the pseudo-curve `zip32-orchard` (now `_ENTITLEMENT`, private).
  - It sits in the header, which Trezor's ring admission signs.
  - It is not a separate "direct key access" field. That decision stays with Trezor.
  - `run.py`'s one-curve rule remains, so the app cannot also use BIP-32/secp256k1 operations.
- **Paths.**
  - Each declared pattern is parsed with its own coin type (`_coin_type`).
  - The account path must match one of the patterns (plain `schema.match`, which never falls
    back to a prompt).
  - Address MAC operations bind a single SLIP-44 value, so they now refuse (op result `False`)
    when the patterns name several coin types. Previously such an app failed on every message.
  - Other operations keep their checks unchanged.

## 4. Tests run (final tree `91fc95c0ac`)

| Check | Result |
|---|---|
| SDK `make sdk_fmt_check sdk_check sdk_clippy sdk_test` | exit 0; 4/4 tests. The new `crypto_request_must_match_the_operation_id` covers all 8 ops against ids 0–8. The new `malformed_crypto_requests_are_refused` covers an intact control, empty input, truncation, a misaligned buffer, an unknown tag, out-of-bounds pointers in both directions, an oversized length, a bad bool, and non-UTF-8 |
| `make -C core build_unix PYOPT=0` (T3W1, `--apps`) | exit 0 |
| `test_apps.extapp.zip32_orchard.py` | 16/16. Adds the approval-cache tests (per instance and account, new instance, keeps the newest 8) and both networks declared by one app |
| Full core unit suite (`run_tests.sh`, Tropic model on private port 21377, `TREZOR_UDP_PORT=21381`) | **136/136** |
| End-to-end over real IPC: uncommitted probe app `zip32probe` (paths `m/32'/133'/[0-9]'` and `m/32'/1'/[0-9]'`) on emulator UDP 21351 / Tropic 21357 | **10/10**, detailed below |
| Emulator log grep for the three derived keys | 0 matches |
| Ethereum sample subset (`test_sign_verify_message`, `test_getpublickey`, `test_sign_typed_data`), same private ports, app rebuilt on the new SDK | **36 passed** |
| ruff 0.15.10 (anaconda, repo config), check and format | clean |
| Not run | pyright; `make style_check`; nix toolchain; `cargo vet` (not installed) |

**End-to-end cases (10/10):**
- **Consent.** The first request shows one `ButtonRequest(Other)` with title "Zcash account" and
  text containing "Zip32Probe", "#1" and "Mainnet". Then:
  - a repeat request for the same account gives `ExtAppResponse` only, with no prompt;
  - account 1 prompts again with "#2";
  - testnet account 0 prompts with "Testnet";
  - both approved accounts are then served with no prompt.
- **Rejection.** Rejecting gives `exceptions.Cancelled`. The next request for that account prompts
  again, and the key returned after approval is correct.
- **Policy refusals.** Six refusals with no prompt: (133,10), (1,10), (133,2^31), (133,2^32−1),
  (60,0), (0,0). The app still works afterwards.
- **Malformed crypto requests.** These are raw bytes sent through a scratch SDK function built
  into the probe only.
  - Control: a 96-byte archived op-7 request, accepted as op 7.
  - Refused with `Boolean(false)`, and after each one the app still returns account 0:
    - the same bytes under id 1, 0 or 99;
    - the request truncated to 95 or 12 bytes;
    - empty input;
    - a trailing extra byte;
    - tag 0xFF;
    - 64 garbage bytes;
    - a GetPublicKey whose pointer points out of bounds.
- **Malformed UI request.** Core stops the app with `Failure("Invalid UI request: …")`.

**Scratch copies.** The probe, its test and the scratch SDK patch are in
`/tmp/extapp-keys/probe-src/`; the harness is `/tmp/extapp-keys/probe_e2e.py`. They are
reproducible from those files, and are not in the worktree. Logs are in `/tmp/extapp-keys/*.log`.

## 5. Flash cost (T3W1 hardware, `make -C core build_firmware BOOTLOADER_DEVEL=1`)

| Tree | `firmware.bin` | `.flash` | Δ vs base |
|---|---|---|---|
| base `1f71e245e4` | 2,356,224 | 2,353,664 | — |
| first P1 version (`bd69375943`) | 2,358,272 | 2,355,712 | +2,048 |
| + checked rkyv validation (bytecheck in core; crypto, UI and progress) | 2,361,344 | 2,358,784 | +5,120 (**validation +3,072**) |
| final `91fc95c0ac` (+ consent, approval cache, per-path coin types, cancel tag, guards) | 2,362,368 | 2,359,808 | **+6,144** |

Hardware `--apps` builds need `BOOTLOADER_DEVEL=1`: upstream has no production root-packet keys.

## 6. Review resolutions

The reviews were Sol (GPT-6 Sol, adversarial, verdict REJECT) and the clarity review (Opus 5.5).
Both reviewed `bd69375943`; they have **not** re-reviewed the rewritten commits.

| Finding | Resolution |
|---|---|
| **Sol must-fix 1:** unchecked rkyv on app bytes | **Fixed** (`91fc95c0ac`).<br>• Crypto, UI and Progress bridges now use `rkyv::api::low::access`, with bytecheck enabled through the SDK dependency.<br>• `access_crypto_request` also requires the archived variant to equal the IPC message id; `run.py` passes `message_id`.<br>• A bad crypto request gets the normal `False` result. A bad UI or Progress request `die`s with `DataError`.<br>• Tests: SDK unit tests and the end-to-end malformed cases (§4).<br>• Cost: +3,072 B.<br>• Side effects: an unknown op id now gets `False` instead of stopping the app. Three crates enter `core/embed/Cargo.lock`: `bytecheck`, `bytecheck_derive`, `simdutf8`. See follow-ups. |
| **Sol must-fix 2:** consent | **Fixed** with core-owned consent per app instance and account, cached for the session (§3). The "signed direct-key entitlement" part is **not** implemented: the header's pseudo-curve (signed through ring admission) is still the entitlement. That is a Trezor platform decision. |
| **Sol should-fix:** one curve / one coin | **Coin: fixed** (per-path coin types, both networks in one app; address MACs keep the single-coin requirement). **Curve: not changed.** The one-curve rule stands, so transparent plus shielded in one app is still blocked. Follow-up. |
| **Sol should-fix:** secret copies | **Partly fixed.**<br>• Derivation slices are `memoryview`s, so the only Python copies are the four immutable 64-byte `blake2b.digest()` outputs (master plus three children). `digest()` cannot write into a buffer and `utils.memzero` cannot write `bytes`, so these are not wiped.<br>• Rust side: `WipedResult` and `WipedBuffers` drop-guards zero the key copy and the serialization buffers on every exit path, and the key is copied last so an early error leaves no copy.<br>• **Remaining, documented copies:**<br>&nbsp;&nbsp;– the digest `bytes` on the core GC heap;<br>&nbsp;&nbsp;– the Python `bytes` that `ipc_cb` creates for the reply;<br>&nbsp;&nbsp;– the app's 16 KiB IPC inbox (kernel-owned; not wiped before `ipc_message_free`);<br>&nbsp;&nbsp;– Rust move copies in the app.<br>• The full fix is a Rust derivation into zeroizing buffers. It needs a BLAKE2b binding in `core/embed/crypto`. Follow-up. |
| **Sol nit:** ZIP-32-invalid intermediates | Documented: the app must validate `sk` (SDK doc, §1). |
| **Clarity S1–S4:** out-of-tree references | Removed from code, test and commit text. The fingerprint docstring and the test comment now use the reviewer's wording. |
| **Clarity S5:** helper naming and re-indent | Renamed to `with_crypto_reply`; kept `let result = match …; Ok(result)` inside the closure. |
| **Clarity N1:** `failed()` | Removed; the two new sites inline the error, as the old wrappers do. |
| **N2** | Both result tags named (plus the cancel tag). |
| **N3** | Log reads "Failed to get ZIP-32 Orchard account". |
| **N4** | Scalar `curve`, as the reviewer suggested. |
| **N5** | `secret.count(b" ") + 1`. |
| **N6** | Seed-length comment rewritten. |
| **N7** | Kept: both functions are public and tested. |
| **N8** | Kept the volatile-write `Drop`: the SDK has no `zeroize` supply-chain entry. |
| **N9** | `H_()`; inlined the mnemonic. |
| **N10** | Canonical vector source cited. |
| **N11** | Subjects are 65 and 60 characters. |
| **N12** | Each commit builds and behaves on its own: the handler ships with the op; validation comes separately. |
| **N13** | Variant doc shortened to one line. |

## 7. Follow-ups and open questions

1. **Re-review.** Run a Fable/Sol adversarial pass and a clarity pass on `c549ab5f18` and
   `91fc95c0ac`.
2. **`cargo vet` for `core/embed`.** Its `supply-chain/config.toml` has no entries for the rkyv
   family at all on this base (`rkyv`, `rancor`, `munge`, …). The newly added `bytecheck`,
   `bytecheck_derive` and `simdutf8` need exemptions or audits. The SDK's own vet config already
   exempts them.
3. **Consent copy is hardcoded English.** It needs `TR` strings and translations before any
   upstream proposal. Trezor's UX team may want a different screen, e.g. showing the app vendor.
4. **Entitlement form and multi-curve apps.** Trezor's call.
5. **Remaining key copies.** Move to Rust derivation plus zeroizing transport, and have the SDK
   wipe kernel-owned replies before freeing them (§6).
6. **UI and Progress ids.** Those services' requests are validated but not matched against their
   message ids (the UI service uses id 0 for every screen).
7. **Approval cache size.** It holds the newest 8 accounts per session. An app that cycles through
   more accounts re-prompts for older ones. That is safe, but noisy.
