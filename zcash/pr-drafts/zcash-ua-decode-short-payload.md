# fix(core): raise DataError for malformed Zcash unified addresses

Status: **a PR against `main`** (the code is Core's Zcash app on `main`).
Trezor asks outside contributors to open an issue first; open one, then replace
`#N` below. Same shape as merged PR #7750.
Reproduction status: two new unit tests fail on `main` and pass with the fix.
Based on `main` @ `8760d111b6`; merges cleanly with `main` @ `3227ea8110`
(2026-09-26).

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/zcash-ua-decode-short-payload @ `935383396b`

---

It is possible to craft a unified address that fails with the wrong error
type. A bech32m payload shorter than f4jumble's 48-byte minimum reached
`f4unjumble()`, whose `ensure()` raised a bare `AssertionError`, and 5-bit data
with nonzero leftover bits made `convertbits()` raise a `ValueError`. On the
transparent `SignTx` output path both reached the host as an unexpected failure
instead of a `DataError`.

Fixes #N

**Reproduce.** On a non-frozen emulator of `main`:

```sh
cd core/tests && ./run_tests.sh test_apps.zcash.unified_addresses.py
```

```
test_decode_short_payload ... failed
  File "../src/apps/zcash/f4jumble.py", line 52, in f4unjumble
  File "../src/trezor/utils.py", line 197, in ensure
AssertionError:
test_decode_nonzero_padding_bits ... errored:
  File "../src/trezor/crypto/bech32.py", line 125, in convertbits
ValueError:
Ran 4 tests (1 failed, 1 errored)
```

The short address is `u1qqqsyqcyq5rqwzqf6qaezy` (valid checksum, 10-byte
payload).

**Cause.** `decode()` converted only the `bech32_decode` failure, and called
`f4unjumble()` before checking the payload length.

**Fix.** Convert the `convertbits` `ValueError` to `DataError`, and refuse a
payload shorter than 48 bytes with `DataError("Invalid address length")` before
`f4unjumble()`. Every valid unified address (a shielded receiver plus 16 bytes
of padding) is longer.

**Tests.** `test_decode_short_payload`, `test_decode_nonzero_padding_bits`.
4/4 with the fix.

### Notes for QA
A `SignTx` output to a malformed unified address gets a `DataError` instead of
an unexpected failure. Valid addresses are unaffected.
