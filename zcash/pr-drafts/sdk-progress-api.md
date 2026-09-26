# Progress: show the Ethereum data progress, track the screen in the SDK, add `ui::Progress`

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The SDK
exists only on the draft SDK branches. `ui::Progress` overlaps vojczejk's
`9ee009365b feat(sdk): add library-owned progress to modui` on
`vojczejk/sdk-wip-modui`; the owners should pick one.
Kind: two bug fixes and a feature.
Reproduction status: standalone (a subset of the Ethereum sample's tests), plus
an SDK unit test after the fix.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/sdk-progress-api @ `65166b3176`
(three commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

Core stops an app that reports or ends a progress screen it never initialized,
and the Ethereum sample did exactly that, so its staking and access-list
signing tests failed with the app stopped.

- `9ef5174775` fix(extapp): show the ethereum data progress before reporting it
- `2ac810dbb5` fix(sdk): track whether a progress screen is shown
- `65166b3176` feat(sdk): add ui::Progress with a keep-alive

**Reproduce.** From `sdk/apps/ethereum`, app built with
`xtask modular build -p ethereum -m t3w1 --lang en -d -e`, T3W1 `--apps` emulator:

```sh
pytest --app=../target/artifacts/t3w1-emu/ethereum.elf --lang=en -rA \
  -k "test_signtx_staking_eip1559 or test_signtx_eip1559_access_list or (test_signtx and unknown_token_unknown_chain)" \
  tests/test_signtx.py
```

```
before: 15 failed (8 of them "DataError: Progress not initialized", app stopped)
after:  8 passed, 7 failed
```

The 7 left: 6 `unknown_token_unknown_chain` cases stop at a test-side
`Translation key 'words__cancel_and_exit' not found`, and
`access_list[max_count]` needs `extapp/sdk-allocator-heap`.

**Cause.** `get_progress_indicator` reported progress without `init_progress`,
and the SDK did not know whether a screen was shown, so an out-of-order report
or end reached Core.

**Fix.**
1. The Ethereum sample shows the screen when the indicator is created, as
   Core's Ethereum app does.
2. The SDK tracks the screen. `update_progress` without one fails locally with
   `DataError`; `end_progress` without one does nothing; a response or an error
   ends a screen left open. The doc examples use the 0..=1000 scale Core
   expects.
3. `ui::Progress` shows a screen for the duration of a computation and ends it
   on drop. `keep_alive()` repeats the last value at most every 100 ms, so a
   long computation stays within Core's 1 s IPC limit. The guide gains a short
   "Long computations" section.

**Tests.** SDK test `progress_without_a_screen`; the subset above.

### Notes for QA
This unmasks sample gaps that passed only because Core stopped the app:
`test_signtx_error[vault_*_with_eth]` and `[EIP-7702 - Uniswap …]` now reach
the sample's own checks (its `prepare_vault_tx` checks `value.is_empty()` where
Core checks `value != 0`). `ui::Progress` has no in-tree user yet.
