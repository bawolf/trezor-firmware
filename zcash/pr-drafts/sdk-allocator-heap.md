# fix(sdk): back the global allocator with the app heap

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The SDK
exists only on the draft SDK branches.
Reproduction status: standalone, deterministic (xtask's size report).
Conflicts trivially with `extapp/ipc-buffer-size` in the sample manifests
(adjacent lines).
Rerun 2026-09-26 on sdk-wip and on the pushed tip `5f235435ed`: size reports
(`pr-drafts-rerun/logs/app-eth-hw-{sdkwip,allocator}.log`, no local changes),
the allocation-heavy subset with the base and tip apps on the same sdk-wip
T3W1 emulator (`alloc-subset-{base,tip}.log`), SDK checks at the tip
(`alloc-tip-sdk.log`). Local macOS build fix only (`15de3664a6` cherry-picked
for the emulator app); not needed on Linux.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/sdk-allocator-heap @ `5f235435ed`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

The SDK's global allocator used a fixed 16 KiB static in the app's `.bss` and
ignored `heap-size`. The heap the loader reserves went unused, while the static
counted against the app's RW segment. `sdk/doc/development.md` already warned
about it.

**Reproduce.** `xtask modular build -p ethereum -m t3w1 --lang en`:

```
before: Data segment: 49.1 KB (incl. 16.0 KB stack, 1.0 KB heap)
after:  Data segment: 48.1 KB (incl. 16.0 KB stack, 16.0 KB heap)
```

Before, the 16 KiB static hides in RW and the declared 1 KiB heap is unused.
On the emulator the declared size makes no difference: the unix loader gives
an app the rest of its arena, so after this change an emulator app gets far
more than 16 KiB (see `extapp/emulator-declared-heap`).

**Cause.** `applet_main` initialized the allocator from a static array instead
of from `app_get_heap`.

**Fix.**
- `applet_main` hands the `app_get_heap` region to the allocator. On hardware
  that is exactly `heap-size` (aligned).
- `low_level_api::app_get_heap` returns `(ptr, len)`.
- With a zero heap the allocator is not initialized and every allocation fails;
  if `app_get_heap` fails, the app exits with "Failed to get the app heap".
- The Ethereum and Tron samples declared 1 KiB and relied on the static; they
  now declare the 16 KiB they had.
- The guide drops the warning.

**Tests.** `make sdk_fmt_check sdk_check sdk_clippy sdk_test` pass. The
Ethereum sample's allocation-heavy `test_signtx` cases
(`-k "bigdata or newcontract or merkl or lifi or huge_data or max_count or pagination"`),
on the same sdk-wip emulator: before, three fail with
`DataError: Timeout waiting for message`, the emulator console showing
`Fatal: PANIC at alloc.rs:572`; after, none do (the app now gets the rest of
the emulator's arena). The three are `data_pagination[True-10000]`, `[False-10000]` and
`access_list[max_count]`. Most other cases stop at a `TypeError` in the tests'
definition builder before reaching the app. After the change,
`data_pagination[*]` fail on a UI text assertion and `access_list[max_count]`
on `Progress not initialized`, which `extapp/sdk-progress-api` fixes.

### Notes for QA
`xtask modular build` reports the heap outside RW, as declared in the manifest.
