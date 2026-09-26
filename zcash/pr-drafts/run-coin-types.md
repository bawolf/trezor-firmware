# Extapp path patterns: several coin types, malformed patterns

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches.
Reproduction status: commit 1, standalone repro of the extracted function;
commit 2, a Core unit test that ends Core before the fix and passes after.
`extapp/typed-hash-entitlement` adds to the same new test file, so it goes
after this one.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/run-coin-types @ `4640bbee69`
(two commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`run.py` refused every message of an app whose path patterns name more than
one coin type, such as a mainnet and a testnet pattern. And a pattern with
fewer than three components ended Core.

- `82ee9d3322` fix(core): allow extapp path patterns with different coin types
- `4640bbee69` fix(core): refuse a malformed extapp path pattern with a DataError

**Reproduce.**
- Several coin types: `_extract_slip44_id`, copied verbatim from `run.py` and
  run with `python3` on the patterns `["m/44'/60'/0'"]` and then on two
  patterns with different coin types:
  ```
  single: 60
  two-network: REFUSED -> Expected the same coin type in every allowed path
  ```
  `run()` calls it for every `ExtAppMessage`, so such an app can do nothing.
- Malformed pattern: the new `test_malformed_pattern`, run without the second
  commit on a non-frozen T3W1 `--apps` emulator
  (`cd core/tests && ./run_tests.sh test_apps.extapp.run.py`):
  ```
  test_malformed_pattern ...Task #1 terminated.
  Fatal: Assert at qstr.c:198
  ```
  On this MicroPython build a list index out of range is a fatal assert, so
  catching `IndexError` would not help.

**Cause.** One coin type per app was used both to parse the patterns and to
bind address MACs. The parser indexed `pattern.split("/")[2]` without
checking the length.

**Fix.**
- `_path_schemas` parses each pattern with its own coin type. It returns the
  coin type the patterns share, or `None` if they name several.
- Address MACs bind one coin type, so an app whose patterns name several gets
  the operation's normal failure result for them. Single-coin apps are
  unchanged.
- A pattern with fewer than three components or an empty component gets
  `DataError("Invalid path pattern: …")`, as one without a numeric coin type
  does.

**Tests.** New `core/tests/test_apps.extapp.run.py`: `test_one_coin_type`,
`test_coin_type_per_pattern`, `test_no_patterns`,
`test_pattern_without_coin_type`, `test_malformed_pattern` (7 patterns). 5/5 on
the branch.

### Notes for QA
Ethereum and Tron behave as before, address MACs included. An app with a
mainnet and a testnet path pattern now works on both.
