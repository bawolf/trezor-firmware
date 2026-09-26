# ZIP-32 Orchard account keys for extapps

Status: **a design proposal, not a PR.** It needs the platform team's agreement
first: a Core service that hands an app a spending key is new, and Trezor
decides whether it may exist. Based on the draft SDK (`bieleluk/sdk-wip`,
#7516), so it would be a note for the SDK owners even once agreed.
There is no `extapp/*` branch: the commits sit in our integrated series on top
of the platform fixes. They extend the crypto bridge lines that
`extapp/bridge-validate-untrusted-input` and `extapp/bridge-raise-not-rsod`
change, and the app's two networks need `extapp/run-coin-types`.
Kind: feature. Reproduction status: not applicable.

Branch: https://github.com/bawolf/trezor-firmware/tree/zcash/extapp-series @ `cbce6b97e2`, commits:
- `f8a8261663` feat(sdk): check that reply types fit the device IPC alignment
- `5f7e86b229` feat(sdk): add crypto::get_zip32_orchard_account
- `5620cc3c57` feat(core): derive ZIP-32 Orchard account keys for extapps

---

An app that declares the pseudo-curve `zip32-orchard` and the path
`m/32'/coin_type'/account'` can ask Core for that account's Orchard spending
key, derived from the wallet seed as ZIP 32 specifies, after the user allows it
on a hold-to-confirm screen. Core has no Pallas arithmetic, and adding it would
cost flash on every model, so the app does all Orchard work itself; Core only
derives the key with BLAKE2b.

- SDK: Crypto request `GetZip32OrchardAccount { coin_type, account }` (id 7)
  and replies `Zip32OrchardAccount { spending_key, seed_fingerprint,
  weak_backup }` and `Cancelled`. `crypto::get_zip32_orchard_account` returns a
  struct that zeroes the key on drop; the key never goes into the cloneable
  `TrezorCryptoResult`.
- SDK: a compile-time check that reply types fit the device's 4-byte IPC
  alignment (a reply aligned to 8 would work on the 64-bit emulator only).
  Generic; it could go with `extapp/bridge-validate-untrusted-input`.
- Core, `apps/extapp/zip32_orchard.py`, in this order: the app must declare the
  curve and the path, with coin type 133 or 1 and an account below 2^31; the
  backup type must be known; the user confirms; then `get_seed()` (which may
  ask for a passphrase). Nothing that can be refused is checked after the
  confirmation.
- Consent: title "Spending key", text "{app} will receive the spending key of
  Zcash account #{n} ({Mainnet|Testnet}) and can spend all of its funds.",
  hold to confirm. `{app}` is the name from the app header verified at load.
  Asked once per session, app instance and account. An approval is keyed on
  the instance id and the SHA-256 fingerprint of the verified header, so
  another instance, or the same id with another image, asks again. Up to 8
  accounts are kept.
- `weak_backup` follows ZIP 315: fewer than 256 bits of backup (BIP-39 under 24
  words, SLIP-39 secrets under 32 bytes). Unknown backup types are refused.
- Storage: a 76-byte session-cache slot in both codecs (760 B of RAM with the
  codec's 10 sessions, 1,520 B with THP's 20).
- Strings: `extapp__spending_key`, `extapp__spending_key_template`,
  `extapp__mainnet`, `extapp__testnet`, left out of bitcoin-only builds
  (`extapp` added to `ALTCOIN_PREFIXES`).

**Questions for Trezor.**
1. May a Core service hand an app a spending key at all? The alternative is
   RedPallas signing in Core, which needs Pallas arithmetic in the firmware.
2. Is a pseudo-curve in `curves` the right entitlement, or should it be a
   separate manifest capability?
3. Is the consent screen right, and should it name the app's vendor?
4. The translation strings name Zcash under generic `extapp__` keys. Keep them
   generic or move them under a `zcash__` prefix?

**Not wiped.** The Rust result and serialization buffers and the SDK copy are
zeroized. BLAKE2b digests in MicroPython are immutable `bytes`, so the
intermediate keys (including the master key) and the bytes passed to the IPC
callback stay until collected; the 64-byte seed sits in the session cache
anyway. Each is stated in a one-line comment where it happens.

**Tests.**
- `core/tests/test_apps.extapp.zip32_orchard.py`, 15 tests: ZIP-32 Orchard
  vectors (zcash-test-vectors), an account key checked against orchard's
  `SpendingKey::from_zip32_seed`, the seed fingerprint vector (zip32 0.2.1), the
  16-byte SLIP-39 seed, seed lengths, the policy refusals, approvals per
  instance and image (`test_approval_is_per_app_instance_and_account`), the
  newest eight kept, weak-backup rules. Core unit tests 138/138.
- SDK: `replies_are_read_at_the_device_ipc_alignment`.
- End to end through the Zcash app's device tests on T3W1 and T3T1 emulators,
  including `test_account_request_asked_once` and
  `test_decline_account_request`; UI fixtures cover the consent screen.

Flash: not measured on its own. All platform commits of the series together
add 7.0 KB on T3T1 (1576.5 → 1583.5 KB of 1664.0) and 7.5 KB on T3W1 (2301.0
→ 2308.5 KB), `--apps --bootloader-devel`, of which 2.5 KB and 3.0 KB are the
request validation.

### Notes for QA
Load an app that declares `zip32-orchard` and request an account: the
hold-to-confirm screen appears once, and not again for the same app instance
and account in that session. Declining returns `Cancelled` to the app. Another
account, app instance or image asks again.
