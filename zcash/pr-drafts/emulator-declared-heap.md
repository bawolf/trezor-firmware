# Emulator apps get only the heap they declare

Status: note for the owners of `bieleluk/sdk-wip` (#7516), not a PR. The unix
app loader, xtask and the samples are on the draft SDK branches only.
Stacked on `extapp/sdk-allocator-heap` (without it the SDK never uses the
loader's heap). Kind: feature for the emulator, plus the sample heap it needs.
Reproduction status: Ethereum sample suite at different declared heaps; no new
unit test.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/emulator-declared-heap @ `d502e1ed4d`
(`5f235435ed` from `extapp/sdk-allocator-heap`, then two commits)

---

On the device the loader reserves exactly an app's `heap-size`; the unix
emulator gave every app the rest of its 64 MiB arena. An app that outgrew its
declared heap passed every emulator test and failed only on hardware.

- `171f9391b6` fix(extapp): give the ethereum app the heap its tests need
- `d502e1ed4d` feat(core,xtask): give an emulator app only the heap it declares

**Reproduce.** With the second commit and the Ethereum sample still at 16 KiB,
its device suite (`pytest --app=../target/artifacts/t3w1-emu/ethereum.elf --lang=en tests/`
from `sdk/apps/ethereum`, app built with `-d -e`) has 51 tests killed by
`PANIC at alloc.rs:572`. On a fresh emulator per size, 64, 80 and 96 KiB still
kill `data_pagination[*-10000]`; 128 KiB kills none.

**Fix.**
- Commit 1: the Ethereum sample declares 128 KiB first, so every commit keeps
  its tests passing (on the old loader it changes nothing).
- Commit 2: xtask writes `heap-size` into an emulator image's `data_size`,
  which it left at 0. The heap is the only arena memory an emulator app uses;
  its stack and statics live in the host process. `unix/app_loader.c` grants
  exactly that, or `TS_ENOMEM`. `app_header.h` and the guide say so.

**Tests.** The full Ethereum suite in one app instance at 128 KiB: 0 kills.
`make sdk_check`/`sdk_test` and the modular-xtask tests pass.

### Notes for QA
Emulator app images built before this change declare no heap and must be
rebuilt. An app that outgrows `heap-size` now fails on the emulator too. The
Ethereum sample's 128 KiB is what its tests need; whether the app should need
that much is a separate question (`data_pagination[10000]` alone needs more
than 96 KiB).
