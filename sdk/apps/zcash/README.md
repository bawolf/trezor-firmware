# Zcash / Ironwood app

This app runs on the Trezor and handles shielded Zcash payments through Ironwood.
It displays receiving addresses, exports viewing keys with the user's consent,
and checks and signs payments after on-device approval. The desktop wallet
prepares the transaction and creates its zero-knowledge proof.

The app receives transactions in PCZT (Partially Created Zcash Transaction)
format. Core's [ZIP-32 service](../../../core/src/apps/extapp/zip32_orchard.py)
derives an account spending key and passes it to the app. The app is responsible
for protecting that key and enforcing payment approval; the seed stays in Core.

**Development requirements:** this app runs with the project's custom firmware
and desktop software. Use test wallets and test coins for development.

## SDK integration

Trezor's extension SDK provides Rust APIs and tools for building device apps
separately from Core firmware. Apps use Core's screen, communication and
cryptographic services, and declare their memory and key-path requirements.
Trezor labels the [SDK as work in progress][sdk-status]; interfaces and tooling
can change without notice.

This prototype uses fixed SDK versions and adds a ZIP-32 account-key service,
communication/progress APIs and development app signing. It declares privileged
`ring 0` access for its account-key service; the [inspected SDK guide][sdk-guide]
documents normal apps at `ring 2` and leaves privileged rings unimplemented.
These are development choices to exercise a complete payment flow. Production
key permissions, app admission and host interfaces need agreement with Trezor.

The goal is to contribute a Zcash app and focused SDK/firmware improvements that
Trezor can review and maintain. The app depends on two patched forks:
[Orchard](https://github.com/bawolf/orchard/tree/ironwood/sinsemilla-progress) and
[Sinsemilla](https://github.com/bawolf/sinsemilla/tree/ironwood/sinsemilla-progress)
(`ironwood/sinsemilla-progress` branches), which fit signing in device memory.
The desktop wallet used with it is not public; see the
[repository overview](../../../README.md#related-repositories).

## Models and exact source

| Model | Scope |
| --- | --- |
| **Safe 7 — `t3w1`** | Working extension reference. Mainnet receive/return demonstrations on 16 and 19 October 2026 ran firmware `7a438a4399e427965a414f43ddb5d2230811b2b7` (this branch), with custom desktop software. |
| **Safe 5 — `t3t1`** | Primary delivery target, **diagnostic-only today**. The extension loaded, but requests failed and Core crashed. |

Both demonstrations used unpublished host engine
`8421ad7ed48a4c1a779635eef8ad81149a961ec2` and private desktop `6fbaef2c0a4ab191ca1370072ec24d325d502e56`.
The retained extension's SHA-256 is
`78cdbf7f1e89ad6b06d2b6835569914b2ec3a3cbadb626942cb21e01c5337b3a`.
These identify the recorded combination, not a reproducible or security-approved
build. Keep the recorded source combination together when reproducing that run. The
[pinned hardware history][hardware] records the Safe 5 failure.

The commands below were checked on 16 October against the source of the sibling
Safe 5 branch **`zcash/extapp-demo-v2-arena-stack` at
`998e716f9be636c5203bee1c43c3f5534f45242f`**, **not executed as clean builds or
tests**. That branch carries the same app series plus a Safe 5 app-loading fix and
smaller app, UI and test changes; its Orchard/Sinsemilla pins match this revision. Safe 7
work should use this branch's revision, `7a438a4399e427965a414f43ddb5d2230811b2b7`.
Use a separate disposable checkout at the chosen revision, with its recorded
submodules; do not rebuild over retained demo artifacts. These instructions do not reproduce
the complete mainnet demo, whose desktop integration is private.

## Development environment

Start at the firmware repository root, not this app directory. Follow the
[repository setup](../../../docs/core/build/index.md), using this revision's
[`shell.nix`](../../../shell.nix): Rust **nightly 2026-03-16**, `rust-src` and LLVM
tools, ARM GCC 13, Clang/libclang, `protoc` 31, SDL3 and `uv`. Root Python requires
3.11+. Nix supplies these dependencies; a manual installation must match them,
not just use whichever stable Rust or SDL2 happens to be installed.

```sh
cd /absolute/path/to/trezor-firmware
# Verify this is the separate checkout at the revision you chose (see above).
git rev-parse HEAD
git submodule update --init --recursive
nix-shell
# Inside that shell, still at the repository root:
uv sync --frozen
. .venv/bin/activate
```

Keep the checked-in Cargo and uv lockfiles. [The app workspace](../Cargo.toml)
and [signer-tests](signer-tests/Cargo.toml) pin Orchard to
[`61704d46`](https://github.com/bawolf/orchard/commit/61704d46253e9ac9dc97358b0e17f82bc2b7fc1f)
and Sinsemilla to
[`6607cb01`](https://github.com/bawolf/sinsemilla/commit/6607cb019a587869a183b04172e20b4d2719a8e7); network access or populated caches
are needed for dependencies. The latter workspace is separate because its Zcash
dependencies conflict with the SDK test mock's `digest` dependency.

## Build for an emulator

From the same Nix/uv environment, use the Cargo alias in
[`core/embed/.cargo/config.toml`](../../../core/embed/.cargo/config.toml).
Its [dispatcher](../../../core/embed/xtask/src/main.rs) switches modular commands
to `sdk/apps`; running these commands from this README's directory is wrong.

```sh
cd /absolute/path/to/trezor-firmware/core/embed
cargo xtask build firmware -m t3w1 -e --apps -d
cargo xtask modular build -p zcash -m t3w1 --lang en -e
```

`-e` here means **host/emulator build**, not hardware. Without it, the app targets
`thumbv8m.main-none-eabihf`; no hardware installation commands are provided here.
Use `t3t1` in both commands for Safe 5 diagnostic emulator work only. Its
[model feature](Cargo.toml) selects computed Sinsemilla generators to reduce size;
that does not fix or validate its physical-device runtime.

With default Cargo target directories, the app is published as
`sdk/apps/target/artifacts/t3w1-emu/zcash.elf` (a packaged app image despite the
suffix). The [packager](../../crates/modular-xtask/src/postbuild.rs) also creates
app-admission Merkle proofs and development-signed root packets. These admit an
app to the development loader; they are **not transaction zero-knowledge proofs**
and do not confer production admission. Do not mix artifacts across revisions,
models or host architectures. The pinned xtask's inner build commands do not
pass `--locked`; retain and inspect lockfile changes rather than calling this a
bit-for-bit reproducible build.

## Host tests — no device

```sh
cd /absolute/path/to/trezor-firmware/core/embed
cargo xtask modular unit-tests -p zcash -m t3w1 --lang en
cargo xtask modular unit-tests -p zcash -m t3t1 --lang en

cd /absolute/path/to/trezor-firmware/sdk/apps
cargo test --locked -p zcash-signer

cd /absolute/path/to/trezor-firmware/sdk/apps/zcash/signer-tests
cargo test --locked
cargo test --locked --features computed-generators
```

App unit tests use the SDK mock, not Core or real hardware. The separate signer
suite checks against librustzcash and the tracked public test fixtures, including
mutations and approval binding. Leave `ZCASH_UPDATE_FIXTURES` unset: those tests
should compare fixtures, not rewrite the expected answers.

## Emulator integration tests

Live integration needs a matching debug Core emulator built above, a disposable
profile and an explicit local UDP transport. See
[`core/emu.py`](../../../core/emu.py) for `--executable`, `--temporary-profile` and
`--port`; select the exact model artifact, not an ambiguous `latest` symlink.

**The [test fixture](tests/conftest.py) initializes and wipes its target.**
`device-tests -e` in this SDK selects emulator artifacts but does **not** constrain
transport discovery. Do not run it with a physical wallet connected. A live test
must be isolated to a disposable emulator with an explicit `TREZOR_PATH`, never
an auto-discovered USB device. No runnable reset or live-device recipe is given
here; the command below **only collects tests** and does not run their fixtures.

```sh
cd /absolute/path/to/trezor-firmware/sdk/apps/zcash
TREZOR_PATH=udp:127.0.0.1:21324 TREZOR_TRANSLATIONS_DIR="$PWD/translations" \
  uv run --frozen pytest --collect-only \
  --app=../target/artifacts/t3w1-emu/zcash.elf --lang=en -q tests
```

Pytest generates the ignored Python protobuf bindings at session start from
tracked `protob/` sources, even for collection, so `protoc` is required. It uses
this checkout's `python/` trezorlib, not a PyPI substitute. The path assumes default
Cargo target directories. Safe 5 requires its own matching emulator and artifact.

For future isolated emulator execution, preserve pytest's real exit status:
the pinned [xtask wrapper](../../crates/modular-xtask/src/device_tests.rs) accepts
pytest exit code 1 (failed tests) as success. Collection is not a test pass, app
unit tests do not exercise Core, and neither establishes MCU resource bounds or
UI screenshot-baseline acceptance. Safe 5 remains diagnostic even if emulator
tests pass. No emulator or physical device was started for this documentation edit.

## Reproduction and review limits

The source-checked commands above still need a clean-checkout run. Nix was not
available on the documentation-checking shell; no dependency installation, build,
test suite or artifact regeneration was attempted. Complete demo reproduction
also needs the exact compatible desktop source and retained admission artifacts.
At the [15 October upstream SDK snapshot][upstream-sdk], the runtime uses
`app_runtime2`/traits and the build tool is `trezor-app-tool`; this fork uses
`modular-xtask` and its own low-level API. Port explicitly rather than mixing
those interfaces with this binary. The custom key service, ring-0 manifest and
development admission policy still need review, not just an SDK version bump.

Follow the repository's [contribution](../../../docs/misc/contributing.md) and
[review](../../../docs/misc/review.md) guidance, including style, generation,
changelog and CI requirements. Passing these tests would not resolve the Safe 5
runtime crash, establish secret cleanup or authorize Trezor release signing.

[sdk-status]: https://github.com/trezor/trezor-firmware/blob/9f2c11d7a3001d5e19914e8b2aeeb707ab4d5cf7/sdk/doc/README.md
[sdk-guide]: https://github.com/trezor/trezor-firmware/blob/9f2c11d7a3001d5e19914e8b2aeeb707ab4d5cf7/sdk/doc/development.md
[upstream-sdk]: https://github.com/trezor/trezor-firmware/blob/9f2c11d7a3001d5e19914e8b2aeeb707ab4d5cf7/sdk/crates/trezor-app-sdk/src/lib.rs
[hardware]: https://github.com/bawolf/trezor-firmware/blob/bdd7a850757583b3ae22c02d06dfa81c8a6b9305/zcash/EXTAPP_PLAN.md#hardware-log
