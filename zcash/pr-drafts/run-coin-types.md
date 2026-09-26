# Extapp path patterns: several coin types, malformed patterns

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches.
`extapp/typed-hash-entitlement` adds to the same new test file, so it goes
after this one.
Rerun 2026-09-26: commit 1's repro on sdk-wip's `run.py`
(`pr-drafts-rerun/logs/coin-types-extract-base.log`); the tip's test file with
`run.py` at commit 1 `82ee9d3322` (`coin-types-commit1-with-tip-test.log`) and
at the pushed tip `4640bbee69` (`coin-types-tip.log`), T3W1 non-frozen
`--apps` emulator built at each. No local changes. The Ethereum sample suite
ran on typed-hash-entitlement, which contains this branch
(`eth-pass-set-sdkwip-vs-typed-hash.txt`; the base run's 30 s worktree
switch is in `INCIDENTS.txt`); the Tron suite was not run.
Open for the next push: the body of `4640bbee69` says an empty component also
"indexed past the end of a list"; only the short pattern does.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/run-coin-types @ `4640bbee69`
(two commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

`run.py` refused every message of an app whose path patterns name more than
one coin type, such as a mainnet and a testnet pattern. And a pattern with
fewer than three components ended Core.

- `82ee9d3322` fix(core): allow extapp path patterns with different coin types
- `4640bbee69` fix(core): refuse a malformed extapp path pattern with a DataError

**Reproduce.**
- Several coin types: [repro/run-coin-types/extract_slip44_id.py](repro/run-coin-types/extract_slip44_id.py)
  runs sdk-wip's `_extract_slip44_id` from `run.py` on the host:
  ```
  ["m/44'/0'/account'"] -> 0
  ["m/44'/0'/account'", "m/44'/1'/account'"] -> DataError: Expected the same coin type in every allowed path
  ```
  `run()` calls it for every `ExtAppMessage`, so such an app can do nothing.
- Malformed pattern: the new `test_malformed_pattern` without the second
  commit, on a non-frozen T3W1 `--apps` emulator
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
- A pattern with fewer than three components or an empty component, and one
  that `PathSchema.parse` rejects, gets `DataError("Invalid path pattern: …")`.
  So does one without a numeric coin type, whose message was "Invalid coin
  type in path pattern".

**Tests.** New `core/tests/test_apps.extapp.run.py`: `test_one_coin_type`,
`test_coin_type_per_pattern`, `test_no_patterns`,
`test_pattern_without_coin_type`, `test_malformed_pattern` (7 patterns). 5/5 on
the branch. The Ethereum sample passes the same 59 device tests on sdk-wip
and with these commits (and typed-hash-entitlement's) applied; the Tron
sample's were not run.

### Notes for QA
Single-coin apps behave as before. An app with a mainnet and a testnet path
pattern can now be used on both (covered by the unit tests only).
