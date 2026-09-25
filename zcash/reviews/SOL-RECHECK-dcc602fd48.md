At `dcc602fd48`:

1. **Resolved.** `_payment_address` rejects NUL and every other non-ASCII-alphanumeric character before decoding (`core/src/apps/zcash/sign_pczt.py:120–124`); the regression test is at `core/tests/test_apps.zcash.sign_pczt.py:420–426`. The decoder can no longer validate only a NUL-terminated prefix of the displayed string.

2. **Resolved for the reported failure.** The length check runs before `decode` (`sign_pczt.py:114–124`), and a short, valid-checksum input now has a `DataError` test (`core/tests/test_apps.zcash.sign_pczt.py:428–434`). The floor is correct on both networks: any UA carrying the 43-byte Orchard receiver needs at least its typecode, length, and 16-byte padding; `encode` uses the network-specific prefix (`core/src/apps/zcash/unified_addresses.py:20–25, 68–89`). I found no valid such UA that this floor or character check wrongly refuses. The character check is deliberately broader than the Bech32 alphabet; the decoder still checks the remainder.

3. **Resolved as a coverage finding.** The device test constructs 213- and 512-character addresses and checks that the complete address is collected across UI pages (`tests/zcash_tests/test_sign_pczt.py:165–206`; paging helper `tests/zcash_tests/common.py:40–69`). It uses mainnet vectors, not a separate testnet long-address vector.

**New problems:** None identified by this read-only review. I did not run the tests. **Model ID:** not exposed to this session.