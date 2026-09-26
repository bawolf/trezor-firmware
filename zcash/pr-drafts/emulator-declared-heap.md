# Emulator apps get only the heap they declare

Status: note for the owners of `bieleluk/sdk-wip` (#7516), not a PR. The unix
app loader, xtask and the samples are on the draft SDK branches only.
Stacked on `extapp/sdk-allocator-heap` (without it the SDK never uses the
loader's heap). Kind: feature for the emulator, plus the sample heap it needs.
Reproduction status: Ethereum sample suite at different declared heaps; no new
unit test.
Rerun 2026-09-26 on the pushed tip `d502e1ed4d` (T3W1 non-frozen `--apps`
emulator built at it): full suite at 128 KiB and at 16 KiB
(`pr-drafts-rerun/logs/declheap-full-{128k,16384}.log`), the allocation-heavy
subset at 64, 80 and 96 KiB (`declheap-sweep-*.log`), SDK checks and xtask
tests (`declheap-tip-{sdk,xtask-test}.log`). The "before" is
sdk-allocator-heap's `alloc-subset-tip.log`. Local macOS build fix, adapted:
`15de3664a6` cherry-picked, and its Mach-O arm passes `heap_size` as the
series commit `45fa36ea71` does (without that, a macOS emulator app declares
no heap); not needed on Linux.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/emulator-declared-heap @ `d502e1ed4d`
(`5f235435ed` from `extapp/sdk-allocator-heap`, then two commits)

---

On the device the loader reserves exactly an app's `heap-size`; the unix
emulator gave every app the rest of its 64 MiB arena. An app that outgrew its
declared heap passed every emulator test and failed only on hardware.

- `171f9391b6` fix(extapp): give the ethereum app the heap its tests need
- `d502e1ed4d` feat(core,xtask): give an emulator app only the heap it declares

**Reproduce.** On this branch, set the Ethereum sample's `heap-size` back to
16384 by hand, build it with `-d -e` and run its device suite
(`pytest --app=../target/artifacts/t3w1-emu/ethereum.elf --lang=en tests/`
from `sdk/apps/ethereum`). `access_list[max_count]` and
`data_pagination[True-10000]`/`[False-10000]` now fail with
`DataError: Timeout waiting for message`, the emulator console showing
`Fatal: PANIC at alloc.rs:572`. With the old loader the same 16 KiB app
fails none of them this way. Most other cases stop earlier, at a `TypeError` in the tests' definition
builder. On the allocation-heavy subset
(`-k "bigdata or newcontract or merkl or lifi or huge_data or max_count or pagination"`),
on a fresh emulator per size, 64, 80 and 96 KiB still fail
`data_pagination[*-10000]` this way; the full suite at 128 KiB fails none.

**Fix.**
- Commit 1: the Ethereum sample declares 128 KiB first, so every commit keeps
  its tests passing (on the old loader it changes nothing).
- Commit 2: xtask writes `heap-size` into an emulator image's `data_size`,
  which it left at 0. The heap is the only arena memory an emulator app uses;
  its stack and statics live in the host process. `unix/app_loader.c` grants
  exactly that, or `TS_ENOMEM`. `app_header.h`, `app_get_heap`'s doc comment
  and the guide say so.

**Tests.** The full Ethereum suite at 128 KiB: no allocation failures (59
passed, as on sdk-wip; the one app fatal, `attempt to subtract with overflow
at crypto.rs:80`, occurs on sdk-wip too). `make sdk_check`/`sdk_test` and the
modular-xtask tests pass.

### Notes for QA
Emulator app images built before this change declare no heap and must be
rebuilt. An app that outgrows `heap-size` now fails on the emulator too. The
Ethereum sample's 128 KiB is what its tests need; whether the app should need
that much is a separate question (`data_pagination[10000]` alone needs more
than 96 KiB).
