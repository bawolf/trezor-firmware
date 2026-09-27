# Progress: show the Ethereum data progress, track the screen in the SDK, add `ui::Progress`

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The SDK
exists only on the draft SDK branches. `ui::Progress` overlaps vojczejk's
`9ee009365b feat(sdk): add library-owned progress to modui` on
`vojczejk/sdk-wip-modui`; the owners should pick one.
Kind: two bug fixes and a feature.
Reproduction status: standalone (a subset of the Ethereum sample's tests), plus
an SDK unit test after the fix.
Rerun 2026-09-26 on sdk-wip and on the pushed tip `ba03520ba7` (T3W1 non-frozen
`--apps` emulator and Ethereum app built at each): receipts
`pr-drafts-rerun/logs/progress-subset-{base,tip}.log`,
`signtx-error-{base,progress-tip}.log`, `progress-tip-sdk.log`. Local macOS
build fix only (`15de3664a6` cherry-picked for the emulator app); not needed on
Linux. "As Core's Ethereum app does": its `_get_progress_indicator`
(`core/src/apps/ethereum/helpers.py`) creates the screen with the indicator.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/sdk-progress-api @ `ba03520ba7`
(three commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

Core stops an app that reports or ends a progress screen it never initialized,
and the Ethereum sample did exactly that, so its staking and access-list
signing tests failed with the app stopped.

- `13a088d67f` fix(extapp): show the ethereum data progress before reporting it
- `93f4b2483d` fix(sdk): track whether a progress screen is shown
- `ba03520ba7` feat(sdk): add ui::Progress with a keep-alive

**Reproduce.** From `sdk/apps/ethereum`, app built with
`xtask modular build -p ethereum -m t3w1 --lang en -d -e`, T3W1 `--apps` emulator:

```sh
pytest --app=../target/artifacts/t3w1-emu/ethereum.elf --lang=en -rA \
  -k "test_signtx_staking_eip1559 or test_signtx_eip1559_access_list or (test_signtx and unknown_token_unknown_chain)" \
  tests/test_signtx.py
```

```
before: 15 failed, 224 deselected in 75.74s (0:01:15)
after:  7 failed, 8 passed, 224 deselected in 76.03s (0:01:16)
```

Before, the 6 staking cases and `access_list[single_entry]` and
`[larger_list]` fail with `DataError: Progress not initialized` (Core stops
the app); these 8 pass after. In both runs the 6 `unknown_token_unknown_chain`
cases stop at a test-side `Translation key 'words__cancel_and_exit' not found`
(the key is not in Core's `en.json`), and `access_list[max_count]` times out
on the app's `PANIC at alloc.rs:572`, which `extapp/sdk-allocator-heap`
addresses.

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
This unmasks sample gaps that passed only because Core stopped the app. The
three `test_signtx_error[vault_*_with_eth]` cases and
`[EIP-7702 - Uniswap - fails because we have safety checks]` now fail with
`DID NOT RAISE`: the sample completes transactions the tests expect it to
refuse. For the vaults, its `prepare_vault_tx` checks `value.is_empty()` where
Core checks `value != 0`; the EIP-7702 case is a separate gap. `ui::Progress` has
no in-tree user yet.
