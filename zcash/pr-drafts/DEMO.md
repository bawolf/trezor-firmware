# Reproducing the demo from a clean clone

For a maintainer who wants to see the Zcash app run. Emulator first; hardware
only on a development device.

## Branches

- **`zcash/extapp-series` @ `cbce6b97e2`**: what we propose. It is
  `bieleluk/sdk-wip` @ `4cd93ff4d8`, plus two local commits that are not
  proposed (`15de3664a6` builds and packs emulator apps on macOS;
  `1f71e245e4` updates the Ethereum sample's test definitions to trezorlib's
  magic/version split), plus 30 commits: 23 platform fixes and features, the
  3 key-service commits, 3 app commits and 1 CI commit.
- **`zcash/extapp-demo-v2` @ `7a438a4399`**: the series plus five demo-only
  commits, never proposed: `e88f2a9e6f` and `906d99cbac` (development devices
  only: accept the dev app root key; verify app root packets in the kernel for
  a device running a released secmon), two debug-build diagnostics commits,
  and `7a438a4399` (`xtask modular build --features`).

## Setup

```sh
git clone -b zcash/extapp-series https://github.com/bawolf/trezor-firmware
cd trezor-firmware
git submodule update --init --recursive
nix-shell          # the repo's environment, as CI uses
uv sync
```

Cargo fetches the orchard and sinsemilla forks from github.com/bawolf (pinned
revisions in `sdk/apps/Cargo.toml`). Our runs used macOS arm64 without nix:
Rust nightly-2026-03-16, ARM GCC 13.3, protoc 31.1, `HOST_CC=gcc-15`,
`BINDGEN_EXTRA_CLANG_ARGS=-D__float128=double`.

## Emulator with a dev-signed app

```sh
# Emulator firmware with app loading (T3T1: --model T3T1)
uv run xtask build firmware --emulator --frozen --model T3W1 --apps \
  --pyopt false --disable-animation --dbg-console vcp --debug-link

# The app for the emulator (T3T1: -m t3t1). Writes zcash.elf, its .proof and a
# root packet signed with the dev key to sdk/apps/target/artifacts/t3w1-emu/
uv run xtask modular build -p zcash -m t3w1 --lang en -e
```

The emulator firmware is `core/build-xtask/artifacts/T3W1/firmware-emu`; a
non-production build accepts the dev app root key. Start it as usual
(`core/emu.py`, see `sdk/doc/testing.md`). A T3W1 emulator also needs the
Tropic model. We started both with trezorlib's
`_internal.emulator.CoreEmulator(..., headless=True, debug=True,
disable_animation=True, workdir=<repo>/core/src, tropic_model_port=<port+6>)`
and `TropicModel(..., configfile=<repo>/tests/tropic_model/config.yml)`.

## Tests

```sh
# Zcash device tests with UI fixtures (the fixtures load the app; conftest runs pb2py)
cd sdk/apps/zcash
TREZOR_PATH=udp:127.0.0.1:21324 uv run pytest \
  --app=../target/artifacts/t3w1-emu/zcash.elf --lang=en -rA tests/ --ui=test
#   expected: 77 passed, 1 xfailed, 0 UI diffs (T3W1 and T3T1)
#   equivalent xtask wrapper, not what we ran: xtask modular device-tests -p zcash -m t3w1 -e --ui --lang en
cd ../../..

# App unit tests, signer tests (with and without computed generators)
uv run xtask modular unit-tests -p zcash -m t3w1 --lang en      # 11 passed (also -m t3t1)
make extapp_zcash_signer_test                                   # signer 21; signer-tests 88 + 88
make extapp_vet                                                 # passes on exemptions only
(cd sdk/apps/zcash/signer-tests && cargo vet --locked)          # same

# Core unit tests: a non-frozen emulator, plus the Tropic model for T3W1
uv run xtask build firmware --emulator --model T3W1 --apps --pyopt false --disable-animation --debug-link
cd core/tests && MICROPYTHON='../build-xtask/artifacts/T3W1/firmware-emu -X heapsize=2M' ./run_tests.sh
#   expected: 138/138
```

The Ethereum sample passes 95 of its 336 device tests on the series (81 on
sdk-wip); the rest fail in the tests themselves, mostly a translation key
missing from trezorlib (`words__cancel_and_exit`).

`xtask modular fmt-check` and modular-xtask's `cargo fmt --check` fail on two
lines that are already unformatted on sdk-wip (Ethereum `helpers.rs:360`,
`postbuild.rs:88`).

## Hardware

Development devices only, never a device holding funds.

- **The series** builds for hardware only with `--bootloader-devel`, so it
  needs a development bootloader:
  `uv run xtask build firmware --model T3W1 --frozen --apps --bootloader-devel`
  (FLASH 2308.5 of 3336.0 KB; T3T1 1583.5 of 1664.0 KB). Without
  `--bootloader-devel` the T3T1 build stops at `'MODEL_ROOT_PACKET_KEYS'
  undeclared` (`root_packet.c`): sdk-wip has no app root key for other
  hardware builds.
- **The demo branch** carries the test-device commits instead: the dev app root
  key, and root-packet verification in the kernel for a device with a released
  secmon. A `--production` build of it stops at `#error "In-kernel ML-DSA-44
  verification is for development devices only"`. Build:
  ```sh
  git checkout zcash/extapp-demo-v2
  CARGO_PROFILE_RELEASE_PANIC=abort uv run xtask build firmware --model T3W1 --frozen --apps --emit-memory-analysis
  uv run xtask modular build -p zcash -m t3w1 --lang en
  ```
  The device must accept unofficial firmware. The app is loaded over the wire
  with `trezorlib.extapp.load`, from the `.elf`, `.proof` and root packet in
  `sdk/apps/target/artifacts/t3w1/`.
- A T3T1 hardware image of the demo was not built, and nothing was run on a
  Safe 5.
