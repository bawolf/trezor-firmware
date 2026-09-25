# Mergeability review — `ironwood/zcash-streaming-signing`

Read-only audit, 2026-09-22. No firmware files were changed.

**Subject.** `ironwood/zcash-streaming-signing` in
`/Users/bryantwolf/conductor/workspaces/trezor-firmware/streaming-signing`, head
`8080208f6a`, against `git merge-base upstream/main HEAD = ebd0468e20`.
111 files, +22,217 / −39, 45 commits.

**Question.** Would a SatoshiLabs maintainer read this as Trezor code? Where does it
read as a private research branch that leaked into the tree?

**Verdict.** The *protocol, app, and glue layers are already upstream-shaped* — proto
header and naming, TR keys, the `USE_*` flag chain, workflow-handler dispatch,
trezorlib's license header, device-test directory layout, changelog fragments. What
would bounce is everything around the edges: three git dependencies, two copies of
`pasta_curves`, 45 commit messages that fail Trezor's own `commit-msg` hook, a wire-ID
block that contradicts `common/protob/protocol.md`, a re-signed `signatures.json`, a
self-declared "not for commit" test file, a personal copyright and MIT donor-license
pair inside GPLv3 Trezor Core, zero CI wiring, and 65 lines of private review-trail
commentary naming an external reviewer.

Companion docs: `TREZOR_CONTRIBUTION_SHAPING.md` (why to split; PR 2472 precedent),
`WIRE_ID_COORDINATION.md` (the 32100 block), `SHIPPABILITY_CHECKLIST.md`.
This document is the code-level audit those three assume.

---

## Part 1 — The yardstick, from Trezor's own code

All citations are `upstream/main`. Paths are relative to the `trezor-firmware` root.

### 1.0 The written rules (start here — several are machine-enforced)

- **`common/protob/protocol.md`** — the message-definition spec. *"Wire identifiers
  are organized in logical blocks with 100 numbers per block… If you are adding a new
  application, pick the lowest unused block and use zero in that block."* And:
  *"Documenting/commenting the messages is mandatory."*
- **`common/protob/check.py:11,35`** — `EXPECTED_PREFIX_RE =
  re.compile(r"messages-(\w+)(?:-.*)?\.proto")`; every `enum` and `message` in
  `messages-<coin>.proto` must start with `<Coin>` or `Debug<Coin>`. Mechanically
  enforced.
- **`docs/misc/contributing.md`** — PR criteria: CI green, `make style_check`,
  generated files refreshed via `make gen`, *"Commits must have concise commit
  messages, we endorse Conventional Commits"*, *"A changelog entry must be part of the
  pull request."*
- **`docs/git/hooks/commit-msg`** — the regex that enforces it:
  `^(build|ci|docs|feat|fix|perf|refactor|style|test|chore|revert)(\((common|core|crypto|legacy|python|storage|tools|vendor)\))?: `
  Those eight scopes are the entire allowlist.
- **`docs/misc/review.md`** — *"File a Pull Request with a number of well-defined
  clearly described commits."* *"The author should never force-push during code
  review"* — use `git commit --fixup <hash>`, then `rebase -i --autosquash` after
  approval. *"GitHub's Rebase and merge should be used, not Squash and merge."*
- **`docs/misc/changelog.md`**, `tools/towncrier.toml` — fragment types are exactly
  `added, changed, deprecated, removed, fixed, security, incompatible`.
- **`docs/core/misc/codestyle.md`** — formatting tools and typing only
  (`int | None` not `Optional[int]`; `from __future__ import annotations`;
  type-only imports under `if TYPE_CHECKING:`). **It says nothing about comments.**
  The comment norms below are therefore descriptive, which means deviations read
  loud rather than being caught by a linter.

### 1.1 What they comment

**Spec references, by number, with a URL, often pinned.** The dominant genre.

- `core/src/apps/bitcoin/keychain.py:49-65` — a block of one-line citations: BIP-45,
  BIP-48, BIP-49, BIP-84, BIP-86, `# SLIP-25 for coinjoin: https://…slip-0025.md`.
- `core/src/apps/stellar/consts.py:47` —
  `# source: https://github.com/stellar/go/blob/a1db2a6b1f/xdr/Stellar-transaction.x#L35`
  (pinned commit).
- `core/src/apps/cardano/helpers/paths.py:14` — `# minting has a specific schema for
  key derivation - see CIP-1855`.
- **Zcash precedent already in tree**: `core/src/apps/zcash/f4jumble.py:1-4` is a
  four-line module docstring that is *only* a ZIP-316 citation plus a link to the
  librustzcash reference implementation. `hasher.py:1-5` does the same for ZIP-244.
  `core/src/apps/zcash/signer.py:54-57` — `# We don't check prevouts, because BIP-341
  techniques / # were adapted in ZIP-244 sighash algorithm. / # see:
  https://github.com/zcash/zips/issues/574`.

**Security invariants, phrased as attacker + prevention.**
`core/src/apps/bitcoin/get_address.py:131-133` and
`sign_tx/change_detector.py:55-57` — both *"could be exploited by an attacker… To
prevent this, we require…"*. `core/src/apps/ethereum/clear_signing.py:747` —
`# attacker-controlled length word drive an unbounded parse loop.` They also
explicitly *de-escalate*: `core/src/apps/bitcoin/sign_tx/approvers.py:170,486` —
`# Sanity check not critical for security.` In Rust the equivalent is
`// SAFETY:` — **236 occurrences** in `core/embed/rust/src/`.

**"Why" for a non-obvious choice, with the cost stated.**
`core/src/apps/common/coininfo.py:5` — `# NOTE: using positional arguments saves 4500
bytes of flash size`. `core/src/apps/zcash/unified_addresses.py:20-21` — `# Saves 50
bytes over 'def prefix(coin: CoinInfo) -> str' / # (throws KeyError instead of
ValueError but it should not matter)`. `core/embed/crypto/build.rs:229-233` — why
`secp256k1-zkp` does not inherit common compile attrs (*"the resulting code growth
overflows the bootloader flash"*).

**Issue/PR links when behaviour is dictated externally** — ~28 across
`core/src/apps/` + `core/embed/rust/src/`. E.g. `core/embed/rust/src/lib.rs:5`
(`// workaround https://github.com/rust-lang/rust-bindgen/issues/3053`),
`core/src/apps/management/authenticate_device.py:68`.

### 1.2 What they never comment — hard negatives

Searched across `core/src/apps/**/*.py` + `core/embed/rust/src/**/*.rs`:

| pattern | matches |
|---|---|
| dated comments (`# … 2024-05-12`) | **0** |
| review-trail (`reviewed by`, `per review`, `as discussed`, `addressed review comment`, `feedback from`) | **0** |
| history (`was X before`, `changed in v2.5`, `legacy behaviour`) | **0** |
| benchmark/measurement reports in comments | 3, each a single line justifying a constant |
| multi-line TODO essays | **0** |

The only dated statement anywhere is in prose docs, not code:
`docs/core/misc/codestyle.md:88` — *"The codebase is fully type-checked, except for
the Monero app (as of 2022-01)."*

**TODO census, `core/src/apps/` (~90k lines): 37 markers — 34 `TODO`, 3 `XXX`,
0 `FIXME`, 0 `HACK`. Every one is a single line. None cites a GitHub issue.** The
only structured tag is a project code: `# TODO(N1W1): design/copy`
(`core/src/apps/management/reset_device/layout.py:238`). Tone is informal —
`core/embed/rust/src/lib.rs:69`, `// TODO: maybe get rid of the re-export pattern
:shrugs:`.

### 1.3 Comment length norms

Python comment density: bitcoin 7.8%, ethereum 5.4%, cardano 3.4%, stellar 4.5%,
solana 1.0%.

Contiguous `#`-runs of ≥10 lines in *all* of `core/src/apps/` — there are exactly
**seven**, and the ceiling is **17 lines**
(`core/src/apps/bitcoin/sign_tx/approvers.py:204`, the payjoin approval invariant,
which later code cites as *"See top comment."*). The other six are bulletproof math
(×3), a BIP-141/341 section header, a Soroban UI-security invariant, and an
index-rotation/passive-attacker note. **A >17-line Python comment block is out of
family unless it is a cross-cutting security invariant.**

Rust tolerates more, but only for API contracts: the longest runs in
`core/embed/rust/src/` are 60 lines (`micropython/runtime.rs:59`, the
`catch_exception!` safety contract with `# Safety` / `# Examples` sections), 39
(`ui/shape/utils/blur.rs:1`), 24 (`micropython/gc.rs:167`). The one acceptable
measurement table is `ui/shape/cache/jpeg_cache.rs:11` — an ASCII table that
*derives the constant declared immediately below it*.

### 1.4 Docstrings

Firmware Python is **bare code with `#` comments**: docstring-delimiter lines vs
`def` count — bitcoin 7/382, solana 5/102, cardano 48/298, stellar 51/120. Monero is
the outlier (216/280) because it was ported wholesale. Module docstrings are rare;
`core/src/apps/zcash/f4jumble.py:1` is one of the few and it is a spec citation.

`python/src/trezorlib/` is the opposite: **732 docstring lines / 1922 defs**, because
it is a public library. Every file opens with the 15-line LGPL-3 header
(`python/src/trezorlib/stellar.py:1-15`); **firmware Python files carry no license
header at all**; C files carry GPL-3; `.rs` files carry none.

Rust `///` concentrates on FFI boundaries (`micropython/` 524 doc lines / 3649;
`protobuf/` 48/1199; `storage/` 0/65). Module-level `//!` is genuinely uncommon —
inside `core/embed/rust/src/` almost every `//!` is a generated-file banner. New
top-level modules typically have none (`thp/mod.rs:1` starts straight with
`mod crypto;`).

Special case: inside `obj_module!{}` / `obj_type!{}`, `///` lines are **Python stub
source**, harvested by `core/tools/build_mocks` into `core/mocks/generated/*.pyi`
(`core/embed/rust/src/translations/obj.rs:180+`).

### 1.5 Crates under `core/embed/`

`core/embed/Cargo.toml`:
```toml
members = [ "models", "rtl", "crypto", "sys", "sec", "io", "upymod", "rust", "projects/*", "xbuild", "xtask" ]
resolver = "3"
```

Crate names are **short, single, lowercase, unhyphenated, unprefixed generic nouns**,
all `version = "0.0.0"`, `edition = "2024"`, and all declare `links = "<name>"`
(e.g. `core/embed/crypto/Cargo.toml:2-5`). Verified package names:
`sys`→`sys`, `sec`→`sec`, `io`→`io`, `models`→`models`, `rtl`→`rtl`,
`upymod`→`upymod`, and `rust`→**`trezor_lib`** (the one exception: also the only
crate with `authors = ["SatoshiLabs <info@satoshilabs.com>"]` and `edition = "2021"`).

Hyphenated / `trezor-`-prefixed crates live **outside** `core/embed/`, in the repo-root
`rust/`: `rust/trezor-thp`, `rust/trezor-tjpgdec`, `rust/trezor-client`,
`rust/pareen` — pulled in by path from `[workspace.dependencies]`.

**The decisive precedent: there is exactly one crypto crate, it is named `crypto`,
and it is coin-agnostic.** Coin specificity is expressed as *features and C sources
inside that crate*, not as new crates. `core/embed/crypto/build.rs:136-142` sets
`USE_ETHEREUM` / `USE_MONERO` / `USE_CARDANO` from the `universal_fw` feature, and
`:150-160` conditionally compiles `cardano.c`, `monero/base58.c`, `monero/serialize.c`,
`monero/xmr.c`. Features `eos`, `nem`, `sphincsplus`, `mldsa`, `secp256k1_zkp`,
`noise`, `thp` all hang off the same crate.

**No crate under `core/embed/` has a `tests/` or `examples/` directory.** Verified:
`git ls-tree -r --name-only upstream/main core/embed/ | grep -E '/tests/|/examples/'`
→ empty. Rust tests upstream are inline `#[cfg(test)] mod tests`.

### 1.6 New-coin app layout

```
core/src/apps/stellar/   README.md __init__.py consts.py get_address.py helpers.py
                         layout.py operations/ sign_soroban_authorization.py
                         sign_tx.py tokens.py writers.py
core/src/apps/cardano/   README.md __init__.py addresses.py ... layout.py seed.py
                         helpers/ sign_tx/
```

- **`__init__.py` holds keychain constants and nothing else.**
  `core/src/apps/stellar/__init__.py` is five lines: `CURVE = "ed25519"`,
  `SLIP44_ID = 148`, `PATTERN = PATTERN_SEP5`.
- **Module names are verbs matching the protobuf message**, and the entrypoint
  function must be named identically to the file:
  `core/src/apps/workflow_handlers.py` returns the dotted module string and derives
  the handler name from the last path component. `apps.stellar.sign_tx` → `sign_tx`.
- **`layout.py` holds every UI confirmation coroutine.** Nothing is prefixed with a
  protocol codename.
- **Every altcoin app has a `README.md` with a fixed header** —
  `core/src/apps/stellar/README.md:1-11`: `# Stellar`, `MAINTAINER = …`,
  `AUTHOR = …`, `REVIEWER = …`, `ADVISORS = …`, `-----`, then specs and a
  "what we support / don't support and why" section. Ten apps have one (cardano,
  ethereum, monero, nem, ripple, solana, stellar, tezos, tron, webauthn); zcash
  does not, yet. **This is where long-form design prose belongs.**
- Function-local imports for anything heavy; `# local_cache_attribute` and
  `# global_import_cache` trailing markers (318 uses in `core/src/`);
  coin-prefixed error strings (`raise DataError("Stellar: At least one operation is
  required")`).

`core/src/apps/zcash/` already exists upstream — `signer.py` subclasses
`apps.bitcoin.sign_tx.bitcoinlike.Bitcoinlike` and is registered through the Bitcoin
coin table, not `workflow_handlers.py`.

### 1.7 `core/embed/rust/src/` and module registration

Top level: `align.rs io.rs lib.rs macros.rs maybe_trace.rs strutil.rs time.rs
trace.rs` + `bootloader/ coverage/ definitions/ micropython/ protobuf/ storage/
thp/ translations/ trezorhal/ ui/ util/`. **Single lowercase word, no prefix, no
underscores. Multi-file domains are directories with `mod.rs`.** `lib.rs:31-57` is a
single alphabetised `mod` list, each under its `#[cfg(feature = …)]`, all private
except `pub mod ui` / `pub mod util` (whose exception is explained in a comment).

The five-step registration chain: Rust `#[no_mangle] pub static mp_module_<name>:
Module = obj_module!{…}` in a **separate `micropython.rs` from the logic**
(`thp/mod.rs` vs `thp/micropython.rs`) → `MP_REGISTER_MODULE` under `#ifdef USE_<X>`
in `core/embed/upymod/rustmods.c` → qstrs auto-harvested by
`librust_qstr.h.mako` → frozen-module paths in `qstrdefsport.h` → feature plumbing
through `rust/Cargo.toml` → `projects/firmware/Cargo.toml` → `project.toml` →
`xtask/src/features.rs`.

### 1.8 Protobuf

`syntax = "proto2"`, `package hw.trezor.messages.<coin>`, `option
java_outer_classname = "TrezorMessage<Coin>"`, `option java_package =
"com.satoshilabs.trezor.lib.protobuf"`; `import "messages-common.proto"` only when
`common.*` types are actually used. Names are `<Coin><Verb>`; `Ack` suffix for a
host→device response to a device request. Doc blocks are `/** */` with
`Request:`/`Response:` and flow tags `@start` / `@next X` / `@embed` / `@end` —
*"Messages flow is checked at the compile time."* Field comments are trailing,
lowercase, column-aligned, and **selective**.

`messages.proto` section comments are bare `// <AppName>` (`:101 // Bitcoin`,
`:212 // Stellar`, `:385 // Tron`). **`reserved` is used aggressively and always
annotated** — `:226 reserved 219;  // omitted: StellarInflationOp`; `:251-254`
`// dropped Sign/VerifyMessage ids 300-302` … `reserved 300 to 304, 309 to 312;`.

Occupied 100-blocks upstream: `0 100 … 1100, 2000 2100 2200, 8000, 9000 9100`.
Recent coins took the next free block in sequence: Nostr 2001-2004, Evolu 2100-2107,
**Tron 2200-2213**.

Coin registration beyond the proto: `common/defs/misc/misc.json` (the coin object:
`name`, `slip44`, `curve`, `shortcut`, `decimals`), `common/defs/support.json`
(`"misc:TRX": "2.10.1"` — first firmware version, per internal model), and
`ALTCOIN_PREFIXES` in `common/tools/cointool.py`. The `check:` line in
`common/protob/Makefile` and `SKIPPED_MESSAGES` in
`legacy/firmware/protob/Makefile`.

### 1.9 Changelog fragments

Filename `<issue-or-PR-number>.<type>` (`core/.changelog.d/7311.added`,
`6969.fixed`, `4084.changed`), multiples suffixed `.1`/`.2`, `noissue` permitted; the
towncrier orphan form `+<slug>.<type>` is used when there is no number
(`core/.changelog.d/+solana_create_space.added` — **underscores in core**;
`python/.changelog.d/+cosi-nonces.changed` — hyphens in python). Body: one line,
sentence case, terminal period, coin-prefixed (`Stellar: Support Protocol 27
delegated authentication.`). Cross-component changes get the **same fragment
duplicated byte-identically** into each subproject. CI fails without one unless the
commit says `[no changelog]`.

### 1.10 Docs

Two homes, and the distinction matters:

1. **`core/src/apps/<coin>/README.md`** — app-internal design prose, with the
   `MAINTAINER / AUTHOR / REVIEWER` header. Not linked from `docs/SUMMARY.md`.
2. **`docs/common/<topic>.md`** — protocol-level flows. `docs/common/bitcoin-signing.md`
   is the exact precedent for a chunked/streamed signing document: *"Trezor cannot
   store arbitrarily large transactions in memory, so both the input data and the
   results must be sent in chunks."* It is registered in **two** places:
   `docs/SUMMARY.md` and the `## Message Workflows` section of
   `docs/common/index.md`.

### 1.11 Tests

- `core/tests/test_<dotted.module.path>.py`, unittest under MicroPython, `from common
  import *  # isort:skip`, `@unittest.skipUnless(not utils.BITCOIN_ONLY, "altcoin")`,
  hand-rolled `core/tests/mock.py`. *"Usage of `assert` is discouraged… Use
  `self.assertXY` instead."*
- `tests/device_tests/<coin>/` with an empty `__init__.py`; module-level
  `pytestmark = [pytest.mark.altcoin, pytest.mark.<coin>]`, plus
  `pytest.mark.models(...)` / `pytest.mark.setup_client(...)`. **Markers must be
  pre-registered in `tests/REGISTERED_MARKERS`** — and `zcash` is already on that
  list (line 23), used today by `tests/device_tests/zcash/test_sign_tx.py:53`.
- **Vector fixtures live in `common/tests/fixtures/<coin>/*.json`**, loaded by
  `parametrize_using_common_fixtures` (`tests/common.py:77-96`). Existing:
  `cardano/`, `ethereum/`, `solana/`, `stellar/`, `tron/`.
- **`python/tests/` is 13 flat `test_*.py` files with no fixture directory and no
  binary blobs.** Binary files under `tests/` are rare and narrow: the signed
  Ethereum definition `.dat` blobs, one `.der` cert chain, two homescreen `.jpg`s.
- `tests/ui_tests/fixtures.json` — ~8 MB of `{model: {suite: {test id:
  screen-hash}}}`. Only hashes are committed.
- No upstream device test invokes a compiler.

### 1.12 Dependencies

**`core/embed/Cargo.lock` on `upstream/main` contains zero `git+` sources**
(`grep -c 'git+'` → `0`) and there is no `[patch.crates-io]`. Three mechanisms only:

1. crates.io version pins in `[workspace.dependencies]`, consumed as
   `foo.workspace = true` (leaf crates carry no version numbers).
2. `version` + `path` dual-spec for in-tree forks —
   `qrcodegen-no-heap = { version = "1.8.1", path = "../vendor/QR-Code-generator/rust-no-heap" }`,
   `pareen = { version = "0.3.3", path = "../../rust/pareen", … }` — paired with
   `[policy.<crate>] audit-as-crates-io = true` in `supply-chain/config.toml`.
3. **git submodules for patched upstreams, forked into the `trezor` GitHub org**, or
   a **renamed crate published to crates.io** — `trezor-noise-protocol` is literally
   a fork of `noise-protocol` published under a new name; `trezor-tjpgdec` and
   `qrcodegen-no-heap` are the same pattern.

cargo-vet: `core/embed/supply-chain/config.toml` imports audit sets from
bytecodealliance, google, isrg, mozilla — **and already imports
`https://raw.githubusercontent.com/zcash/rust-ecosystem/main/supply-chain/audits.toml`**
(lines 19-20). That is a real tailwind. Anything uncovered needs an
`[[exemptions.*]]` (68 exist) or a first-party `[[audits.*]]` naming a human:
```toml
[[audits.trezor-noise-protocol]]
who = "Martin Milata <martin@martinmilata.cz>"
criteria = "safe-to-deploy"
```
That crate carries **two** independent audits (Milata and Sedláček) — the expected
bar for a crypto fork.

---

## Part 2 — Findings

`B` = bounce on sight · `S` = should fix before the PR opens · `N` = nit.

### B1 — Three git dependencies and a `[patch.crates-io]` section

`core/embed/Cargo.toml:96` and `:147-152`:

```toml
trezor-pasta-curves = { package = "pasta_curves", git = "https://github.com/jarys/pasta_curves", rev = "e11dfe40…" }

[patch.crates-io]
sinsemilla = { git = "https://github.com/bawolf/sinsemilla", rev = "6ca88ff5…" }
orchard    = { git = "https://github.com/bawolf/orchard",    rev = "d4792911…" }
```

`core/embed/Cargo.lock:1244,1303,1835`. Upstream has zero (§1.12). Two of the three
point at a personal GitHub account. A `[patch.crates-io]` at the workspace root
silently rewrites the dependency graph for *every* crate in `core/embed/`.
Upstreaming path in Part 5.

### B2 — Two `pasta_curves` link into the same firmware image

`core/embed/Cargo.lock:1301-1303` (`pasta_curves 0.4.1`, jarys git fork, aliased
`trezor-pasta-curves`) and `:1314-1316` (`pasta_curves 0.5.1`, crates.io, aliased
`ironwood-pasta-curves`). `trezor-ironwood-receive` uses the first,
`trezor-ironwood` the second, and `core/embed/rust/Cargo.toml:82-90` enables **both**
under the single `ironwood` feature. A production image therefore carries two
independent copies of Pallas/Vesta field and curve arithmetic. At 98.29% flash on
T3T1 this is the largest saving available, and it is free.

### B3 — 45 of 45 commit messages fail Trezor's own `commit-msg` hook

Tested the branch's subjects against the regex in `docs/git/hooks/commit-msg`:
**0 / 45 pass.** Every subject is of the form `zcash: …`, `xtask: …`,
`docs(zcash): …` — but `zcash` and `xtask` are not Conventional Commit *types*, and
`zcash` is not in the scope allowlist (`common|core|crypto|legacy|python|storage|
tools|vendor`). `docs(zcash):` fails on the scope; the rest fail on the type.

Several also carry review-trail text in the subject line itself —
`8080208f6a` *"…(Fable F1-F4)"*, `d5c74d47dd` *"…(review R2-1)"*. A maintainer
reading `git log` sees the private review process before they see the feature.

This is cheap to fix and expensive to leave: it is the first thing the hook, and the
reviewer, will see. Correct forms: `feat(core): …`, `feat(common): …`,
`feat(python): …`, `docs(core): …`, `test(core): …`, `build(core): …`.

### B4 — Wire IDs at 32100 contradict `common/protob/protocol.md`

`common/protob/messages.proto:411-419` allocates `MessageType_Zcash* = 32100…32109`.
`protocol.md` says: *"pick the lowest unused block and use zero in that block"*, and
blocks are 100 numbers wide. Occupied blocks top out at `2200` (Tron, the most recent
coin). **The block a maintainer will expect is `2300`.** 32100 is ~30 blocks past
anything in the file, and the accompanying comment says so out loud:
*"PROVISIONAL local-only identifiers. Not assigned or reserved upstream."*

The *honesty* is right — see `WIRE_ID_COORDINATION.md`. The placement is not. Ask for
`2300` in the PR, ship with `2300`, and let the maintainer move it if they prefer.
A request inside the documented scheme gets answered; a self-allocated block outside
it reads as a fork that does not intend to merge.

### B5 — `tests/device_tests/zcash/test_ram_sweep.py` says "not for commit"

Line 3, verbatim: `TEMPORARY investigation harness (emulator-ram-sweep-20260918);
not for commit.` 168 lines of env-var-driven (`RAM_SWEEP_NS`, `RAM_SWEEP_OUT`,
`IRONWOOD_REGION_BYTES`) measurement scaffolding in the device-test suite.
**Delete from the PR.**

### B6 — Device tests shell out to `cargo`

`tests/device_tests/zcash/test_ironwood_streaming_sign.py:3-7,33-34,43-45` — the PCZT
fixture is built by invoking `cargo` on
`core/embed/ironwood/examples/ironwood_fixture.rs` (484 lines), and returned
signatures are verified by the same tool. `IRONWOOD_CARGO` env var. No upstream device
test compiles anything (§1.11).

**Disposition:** generate the vectors once and check them in as
`common/tests/fixtures/zcash/sign_pczt.json`, consumed via
`@parametrize_using_common_fixtures("zcash/sign_pczt.json")` — the exact mechanism
cardano, ethereum, solana, stellar and tron already use. Move
`ironwood_fixture.rs` out of the firmware tree; it is the *generator*, and generators
live with the project that owns the corpus.

### B7 — `core/translations/signatures.json` re-signed on a fork

`core/translations/signatures.json:2-6` — `merkle_root` replaced, `datetime`
`2026-09-22T02:23:14`, `commit` `ae5532f0b909…` (a commit that exists only on this
branch). This is a SatoshiLabs-signed release artifact. **Revert the file entirely**
and note in the PR that the six new TR keys need an SL re-sign.

### B8 — Personal copyright and an MIT donor-license inside GPLv3 Trezor Core

- `core/embed/ironwood/NOTICE:2` — `Copyright (c) 2026 Bryant Wolf`
- `core/embed/ironwood/DONOR-LICENSE-MIT:3` — full MIT text, same copyright
- `core/embed/ironwood-receive/NOTICE:2` — same
- `core/embed/ironwood/SOURCE_MAP.md:7-13` — provenance table pointing at
  `github.com/bawolf/trezor-ironwood` commit `12c25c8e` and a
  `bawolf/trezor-firmware` archive path; `:42` mentions *"The experimental Madison
  worktree was used only as a diff guide"* — an internal codename meaningless upstream.
- `core/embed/ironwood-receive/licenses/pasta-curves/{COPYING.md,LICENSE-APACHE,
  LICENSE-MIT}` — 237 lines of vendored license text for a *cargo* dependency.
  Upstream carries no per-dependency license copies; cargo-vet is the mechanism.

Upstream `core/embed/*` crates declare no `license` field and no `NOTICE`; they
inherit `core/COPYING`. A per-crate NOTICE asserting third-party copyright, plus a
second license file, plus a provenance map to personal repos, reads as an unresolved
licensing question and stops review cold. **Drop all of it from the PR**; state the
GPLv3 relicensing and the `zcash-orchard`-branch lineage once, in the PR description,
and sign the CLA.

### B9 — 65 lines of private review-trail commentary, in 24 files

Full list in Part 3. Representative: `core/embed/ironwood/src/prewarm.rs:16-17` —
*"(SHOULD-FIX #3, Fable review 2026-09-19)"*;
`core/embed/rust/src/ironwood_allocator.rs:310` — *"MUST-FIX #1 (Fable review,
2026-09-19)"*; `core/src/apps/zcash/sign_pczt.py:59` — *"(Fable review #S3)"*.
An external reviewer's name, review-round numbers, and ISO dates appear in 24 files
including `core/src/storage/cache_common.py` — a **shared core** file. Upstream has
**zero** comments of any of these three kinds (§1.2).

### B10 — Zero CI wiring

`grep -rn 'ironwood\|zcash' ci/ .github/` → **no matches.** Nothing builds
`--ironwood`, runs `cargo test -p trezor-ironwood` (5,376 lines of equivalence
tests), runs `core/tests/test_apps.zcash.*`, or collects
`tests/device_tests/zcash/`. A 22k-line feature with no CI job is not mergeable —
and the tests are the strongest part of this branch.

### B11 — No UI test fixtures for the new screens

`tests/ui_tests/fixtures.json` is untouched, yet the branch adds six TR keys and new
confirm/warning/progress flows across three layouts (caesar, delizia, eckhart).

### S1 — The two crates do not match any `core/embed/` precedent

Two crates (`core/embed/ironwood`, `core/embed/ironwood-receive`), both `no_std`
Orchard crypto, both consumed only by `core/embed/rust/src/ironwood_*.rs`. Dir names
`ironwood` / `ironwood-receive`, package names `trezor-ironwood` /
`trezor-ironwood-receive` — **neither half matches** (upstream is `sys`/`sys`,
`io`/`io`; `trezor-`-prefixed crates live in repo-root `rust/`). Neither declares
`links = "<name>"`, which every embed crate does. And §1.5's decisive point: coin
crypto upstream is a **feature of the single `crypto` crate**
(`core/embed/crypto/build.rs:136-160` compiles `cardano.c`, `monero/*.c` behind
`USE_CARDANO` / `USE_MONERO`), not a new crate per coin.

Options, in order of how a maintainer would receive them:

1. **Fold into `core/embed/crypto` behind a `zcash` / `ironwood` feature.** Closest to
   precedent. Cost: `crypto` is currently thin Rust over vendored C, and this is 3,500
   lines of pure Rust with a large dependency tree — it would change that crate's
   character. Propose it and let the maintainer say no.
2. **One new crate, `core/embed/ironwood/`, package `ironwood`, with `links =
   "ironwood"` and `receive` as a module.** Matches the naming convention exactly, and
   forces B2 to resolve (one `pasta_curves`). **This is the recommendation.**
3. Two crates only if `ironwood-receive` genuinely ships without `ironwood` — it does
   not: `core/embed/rust/Cargo.toml:79-90` always enables both.

### S2 — `tests/` and `examples/` under `core/embed/`

`core/embed/ironwood/tests/` is 5,376 lines across 8 files; `examples/` is 484. No
crate under `core/embed/` upstream has either (§1.5). `tests/` is defensible —
integration tests genuinely need the directory, and these tests are the crate's
correctness argument — **but only if a CI job runs them** (B10). `examples/` is not:
move it out (B6).

Related: `pczt`, `rand_chacha`, `zip32`, `serde_json` sit in `[workspace.dependencies]`
(`core/embed/Cargo.toml:95,114,141`) purely to serve `[dev-dependencies]` of one
crate. Host-only deps in the firmware workspace manifest will draw questions.

### S3 — `Engine` ships in firmware but is never called

`core/embed/ironwood/src/lib.rs:639` — `pub struct Engine<R>` plus ~500 lines of
non-streaming reference implementation, `pub` and ungated. The device path uses only
`Session` (`core/embed/rust/src/ironwood_signing.rs` calls `session_*` exclusively).
`Engine` exists to be the oracle that `tests/session_equivalence.rs` and
`tests/stream_equivalence.rs` compare against. **Gate it `#[cfg(feature = "test")]`.**
Free flash, and it removes "why are there two implementations?" from review.

### S4 — Three `ironwood_*.rs` at the top of `core/embed/rust/src/`

`core/embed/rust/src/{ironwood_allocator.rs, ironwood_signing.rs,
ironwood_unix_allocator.rs}` plus a `#[path = "ironwood_unix_allocator.rs"]` alias in
`lib.rs:44-47`. Upstream puts multi-file domains in a directory with `mod.rs`
(§1.7). Recommend `core/embed/rust/src/ironwood/{mod.rs, allocator.rs,
allocator_unix.rs, signing.rs}`. Note also that `micropython/ironwood.rs` correctly
follows the `thp/mod.rs` + `thp/micropython.rs` split — keep that.

### S5 — Codename-prefixed modules in `core/src/apps/zcash/`, and no `layout.py`

`ironwood_account.py` and `ironwood_measurement.py` are the only codename-prefixed
modules in any coin app; upstream names by role (`seed.py`, `helpers.py`, `consts.py`,
`layout.py`). Rename `ironwood_account.py` → `account.py` (or fold into a
`seed.py`-shaped module); drop `ironwood_measurement.py` entirely (S7).

All screens are inline in the handler modules — `sign_pczt.py:106-256`
(`_confirm_output`, `_confirm_memo`, `_confirm_totals`, warnings), plus screens in
`get_address.py` and `get_viewing_key.py`. **Extract to
`core/src/apps/zcash/layout.py`.**

### S6 — Missing registrations that upstream convention expects

Untouched by the diff, all of which a reviewer will ask about:

| file | what it wants |
|---|---|
| `docs/common/index.md` | `docs/SUMMARY.md` was updated, but the `## Message Workflows` list — which currently names only `bitcoin-signing.md` — was not. Both are required (§1.10). |
| `common/defs/misc/misc.json` / `common/defs/support.json` | Zcash exists today as a Bitcoin-fork definition. Shielded/Ironwood support needs its own answer here, or an explicit note in the PR that it does not. |
| `core/src/apps/zcash/README.md` | Ten of the altcoin apps carry one, with the `MAINTAINER / AUTHOR / REVIEWER` header. This is the right home for the design prose currently sitting in `SOURCE_MAP.md` and module headers. |
| `tests/device_tests/zcash/*.py` | Missing `pytest.mark.zcash`. The marker is already registered (`tests/REGISTERED_MARKERS:23`) and the sibling file `test_sign_tx.py:53` already uses it. One-line fix. |

### S7 — The measurement lane is production-visible

`core/src/apps/zcash/ironwood_measurement.py` (165 lines),
`core/embed/ironwood/src/bench.rs` (168), the `ironwood-measurement` feature across
five manifests, the `--ironwood-measurement` xtask flag
(`core/embed/xtask/src/options.rs:216-220`), a `session_region_high_water` binding, a
`debug_timings` trailer, and the sharp edge:
`core/src/apps/zcash/get_address.py:35-47` — **an all-`0xff` `diversifier_index` on
the production `ZcashGetAddress` message routes to `_op_timing_bench()`.**

The gating is genuinely correct (default-OFF, `ImportError` path in production, no
symbol links). But a maintainer reads "magic sentinel in a wire field triggers a
benchmark" and stops. Upstream's instrument for this is DebugLink
(`DebugLinkGetGcInfo`) and the `memperf` feature. **Cut the whole measurement lane
from the upstream PR** — ~600 lines, six feature declarations, one wire-field
sentinel, zero reviewer value.

### S8 — The signing doc's numbering is inherited from a private note

`docs/common/zcash-ironwood-signing.md:11-14` — *"Section numbers are stable and are
what the source cites as '§N'. Sections 1, 2, 8, 9 and 12 of the originating design
note cover goals, prior art, flash estimates, the conformance strategy and known
limits; they are not normative for the code and are not reproduced here."* The
document starts at `## 3.` and jumps `7 → 10 → 11`.

The document itself is excellent and correctly placed. But the gaps advertise a
document the reader cannot see. **Renumber 1..N, delete the meta-paragraph, update
the ~12 `§N` citations in source.**

### S9 — The `PROVISIONAL` banners belong in the PR description

`common/protob/messages-zcash.proto:8-33` (a 26-line banner),
`common/protob/messages.proto:411-415`, `python/src/trezorlib/zcash.py:17-20`,
`python/tests/fixtures/zcash/MANIFEST.json:3-4`, and the word "provisional" in three
of four changelog fragments. Upstream message definitions carry no meta-commentary
about their own status. Keep one line at `messages.proto` (`// Zcash — block
requested, see PR #NNNN`), move the rest to the PR body.

Note `messages.proto:418` — `reserved 32107, 32108;  // omitted: whole-PCZT download
pair (signed chunk + ack)`. The annotated `reserved` is **exactly** upstream style
(cf. `:226 reserved 219;  // omitted: StellarInflationOp`). Keep it; just renumber
with the block (B4).

### S10 — Changelog fragments are stale, over-hedged, and incomplete

`python/.changelog.d/+ironwood-zcash-host-api.added` — *"Added provisional
`trezorlib.zcash` host bindings … **Device handlers remain follow-up work.**"* The
device handlers shipped 40 commits ago. And **there is no fragment for the signing
feature itself** — the headline change. Also: core's orphan-slug convention uses
underscores (`+solana_create_space.added`); ours uses hyphens. Once PR numbers exist,
rename to `<number>.added` and duplicate cross-component fragments byte-identically
into each subproject.

### N1 — `USE_IRONWOOD` vs `USE_ZCASH`, and terminology generally

`core/embed/upymod/modtrezorutils/modtrezorutils.c:855`,
`core/src/trezor/utils.py:31`, `core/embed/upymod/rustmods.c:55`. Mechanically
perfect (matches `USE_THP` / `USE_MINISCRIPT`). But "Ironwood" is a Zcash
network-upgrade codename and every other `USE_*` names a capability. Term counts
across the diff: `ironwood` 682, `zcash` 730, `orchard` 233 — balanced enough that no
reader can tell which is the primary noun. Decide once and apply to the flag, the
Cargo feature, the xtask flag, the crate name, the module name (`trezorironwood`) and
the app module names. Recommendation: **`zcash` for anything user- or
build-facing** (it is the coin, it survives the next NU), **`ironwood` only where the
protocol version is genuinely load-bearing** (the wire grammar, the digest
personalizations, the crate).

### N2 — `utils.USE_IRONWOOD` repeated three times

`core/src/apps/workflow_handlers.py:245-252`. Upstream's surrounding blocks are bare
`if msg_type == …`. Wrap once. Also `# zcash / ironwood` vs upstream's single-word
`# solana`.

### N3 — `ironwood-measurement` feature inside a crate named `ironwood`

`core/embed/ironwood/Cargo.toml:21`. Should be `measurement`. Moot if S7 lands.

### N4 — Missing `links = "<name>"`; inconsistent `license`

Every `core/embed/*` crate declares `links` (`core/embed/crypto/Cargo.toml:2-5`);
neither of ours does. `core/embed/ironwood-receive/Cargo.toml:5` declares
`license = "GPL-3.0-only"` while `core/embed/ironwood/Cargo.toml` declares none —
and upstream embed crates declare none at all.

### N5 — Unrelated whitespace change

`core/embed/rust/Cargo.toml` — the trailing blank line at EOF is deleted. Revert.

### N6 — `xtask` doc comment carries measurements that will age

`core/embed/xtask/src/options.rs:287-305`. The architectural reason is model prose
(F4 parts share a 191 KiB AUX1_RAM; a 96 KiB boot-lifetime region cannot coexist with
a workable MicroPython heap). But *"none has been measured"* and *"(T3T2's 1640 KiB
slot is not the obstacle — the T3B1 image fits in 1463.5 KiB.)"* are point-in-time
measurements. Trim to the reason.

---

## Part 3 — The comment purge list

**65 lines across 24 files.** Every one is history, a review trail, a measurement, or
a pointer to a document that does not exist in `trezor-firmware`. Finder:

```
grep -rnE 'Fable|MUST-FIX|SHOULD-FIX|#S[0-9]|#M[0-9]|R2/V1|docs/decisions|RAM-RETENTION|20[0-9][0-9]-[01][0-9]-[0-9][0-9]|phase [0-9]|phase-[0-9]|review finding|finding [0-9] of|streaming design|streaming-core review|pending .*review|Raised 8|pinned by a test before|Residual \(deferred'
```

### 3a — Strip the marker, keep the sentence (48 lines)

These comments are *good*: each states an invariant or a why. Only the parenthetical
citation goes. Mechanical: delete `(Fable review R1)`, `(MUST-FIX #1)`,
`(SHOULD-FIX #4)`, `(#S3)` and close the sentence.

| file:line | marker | what survives |
|---|---|---|
| `core/embed/ironwood/Cargo.toml:19` | `(Fable review R1)` | "production builds exclude it entirely." |
| `core/embed/ironwood/src/lib.rs:29` | `(Fable review R1)` | same |
| `core/embed/ironwood/src/lib.rs:40` | `(SHOULD-FIX #4)` | the prewarm contract |
| `core/embed/ironwood/src/lib.rs:942` | `(MUST-FIX #3)` | the `scope_for_address` note |
| `core/embed/ironwood/src/lib.rs:970,1040,1048` | `MUST-FIX #4/#2`, `MUST-FIX #1` | the cmx-validation invariants |
| `core/embed/ironwood/src/session.rs:158,261,281,407,441,469,680` | `MUST-FIX #1` ×7 | "boxed so the CAP-sized `Records` never land on this frame" — a real invariant |
| `core/embed/ironwood/src/session.rs:193,239,241,458` | `MUST-FIX #3` ×4 | the ivk-cache zeroization invariant |
| `core/embed/ironwood/src/session.rs:828` | `MUST-FIX #4/#2` | note-reuse invariant |
| `core/embed/ironwood/src/prewarm.rs:1,16,44` | `SHOULD-FIX #4`, `SHOULD-FIX #3, Fable review 2026-09-19`, `(Fable review #S1)` | the fragmentation argument — keep, it is the module's whole reason to exist |
| `core/embed/ironwood/src/bench.rs:84` | `(SHOULD-FIX #4)` | moot under S7 |
| `core/embed/projects/firmware/Cargo.toml:42` | `(Fable review R1)` | "production builds omit it." |
| `core/embed/rust/Cargo.toml:96` | `(Fable review R1)` | "no region telemetry reachable pre-consent." |
| `core/embed/upymod/Cargo.toml:50` | `(Fable review R1)` | "production builds freeze no measurement Python." |
| `core/embed/upymod/build.rs:1273` | `(Fable review R1)` | same |
| `core/embed/xtask/src/options.rs:218` | `(Fable R1)` | "Default-OFF; excluded from production builds." |
| `core/embed/rust/src/micropython/ironwood.rs:267,294,329` | `(Fable review R1/#2)`, `(Fable review R1)` ×2 | the measurement-gating contract |
| `core/embed/rust/src/ironwood_allocator.rs:310` | `MUST-FIX #1 (Fable review, 2026-09-19)` | "the region now lives for the boot, not the session" |
| `core/embed/rust/src/ironwood_signing.rs:253` | `SHOULD-FIX #4:` | the prewarm rationale |
| `core/src/apps/zcash/get_address.py:42` | `(Fable review R1)` | the production `ImportError` path |
| `core/src/apps/zcash/sign_pczt.py:59,281` | `(Fable review #S3)` ×2 | "no MicroPython/parse text leaks to the host" — a security invariant, keep |
| `core/src/apps/zcash/sign_pczt.py:337` | `(Fable review #M1 / R1)` | "no `utime.ticks_ms` in the hot path" |
| `core/src/apps/zcash/sign_pczt.py:470,475` | `(Fable review #M1)`, `(Fable review #S5)` | `totals[8]` semantics |
| `core/src/apps/zcash/ironwood_measurement.py:79,118` | `(Fable review #S5)`, `(Fable review SHOULD-FIX #4)` | moot under S7 |
| `core/src/storage/cache_common.py:28` | `(Fable review R2/V1)` | "avoid re-copying the mnemonic onto the GC heap" — keep; it justifies a shared-core change |
| `core/tests/test_apps.zcash.ironwood_account.py:92` | `Fable R2/V1:` | the assertion's reason |
| `core/embed/ironwood/tests/session_equivalence.rs:333,1342` | `Streaming-core review finding 1` ×2 | "constructed, not bit-flipped" |
| `tests/device_tests/zcash/test_ram_sweep.py:143` | `(Fable review #2)` | file deleted anyway (B5) |

### 3b — Delete the whole comment (12 lines)

| file:line | text | why |
|---|---|---|
| `core/embed/ironwood/src/wire.rs:10-15` | *"Raised 8 -> 32 to admit 16- and 32-action bundles… NEW change pending Fable review."* | The justification (O(1) growth, ~+5 KB at CAP=32) is worth keeping; "raised 8 → 32" and "pending review" are git history. Rewrite as a statement of the current bound. |
| `core/embed/ironwood/src/session.rs:24-26` | *"`lib.rs` exports `Session` unconditionally for the phase-2 retained link… the device wire handler is design §10 phase 4 and does not exist yet."* | False as of this branch (the handler exists) **and** project-phase language. |
| `core/embed/ironwood/src/stream.rs:2` | *"(phase 1 of the streaming design)"* | phase language |
| `core/embed/ironwood/src/session.rs:22` | *"(finding 2 of the streaming-core review)"* | review trail |
| `core/embed/ironwood/src/bench.rs:3-11` | *"The device latency breakdown (signing-latency-instrumentation, 2026-09-18) showed per-action verification dominates at ~23.3 s…"* | dated measurement report; file deleted under S7 |
| `core/src/apps/zcash/ironwood_account.py:78-81` | *"Residual (deferred, #S2): that one read still allocates… change out of scope here."* | backlog note |
| `core/tests/test_apps.zcash.sign_pczt.py:86` | *"was pinned by a test before -- the device-level autolock test is deferred."* | history |
| `tests/device_tests/zcash/test_ironwood_streaming_sign.py:526-551` | *"Autolock teardown (Fable review #S2) — DEFERRED."* + a 25-line skip block | Write the test or drop it. A permanently-skipped test with a rationale essay is a backlog item living in the test suite. |

### 3c — Dangling document references (5 lines) — must be fixed, not merely trimmed

`docs/decisions/` does not exist in `trezor-firmware`. Neither does
`RAM-RETENTION-ANALYSIS.md`. A reviewer who follows these gets nothing.

| file:line | dangling ref | disposition |
|---|---|---|
| `core/embed/rust/src/ironwood_allocator.rs:40-41` | `docs/decisions/2026-09-18-cross-session-region-lifetime.md` | inline the two-sentence conclusion; delete the path |
| `core/embed/rust/src/micropython/ironwood.rs:154` | same file | same |
| `core/src/apps/zcash/ironwood_measurement.py:13` | `docs/decisions/2026-09-18-signing-latency-*` | file deleted under S7 |
| `core/embed/ironwood/src/prewarm.rs:8-9` | `RAM-RETENTION-ANALYSIS.md, lever R2` | inline the conclusion |
| `core/embed/rust/src/ironwood_signing.rs:259` | `RAM-RETENTION-ANALYSIS.md §2, lever R2` | same |

Anything from those documents a future maintainer needs belongs in
`docs/common/zcash-ironwood-signing.md` §7 (Resource model) or the new
`core/src/apps/zcash/README.md`.

### 3d — Provenance / meta files (B8)

`core/embed/ironwood/{NOTICE,DONOR-LICENSE-MIT,SOURCE_MAP.md}`,
`core/embed/ironwood-receive/{NOTICE,SOURCE_MAP.md,licenses/**}`, and the
`limitations` / `source_evidence` / `provisional_wire_id_revision` fields of
`python/tests/fixtures/zcash/MANIFEST.json`. ~450 lines of internal audit trail →
PR description or the Ironwood repo.

### What to keep, verbatim

Part 2 is a list of complaints and the ratio misleads. The following are *exactly*
what Trezor comments and should survive untouched:

- `core/embed/ironwood/src/digest.rs:1-8` — module header naming the pinned upstream
  constructions and saying why the anchor is deliberately absent.
- `core/embed/ironwood/src/digest.rs:24-33` — `// Private in
  zcash_primitives::transaction::txid; retyped here.` One line, explains a duplication.
- `core/embed/ironwood/src/stream.rs:23-56` — the three `*_BUDGET` derivations, field
  by field. This is the `jpeg_cache.rs:11` pattern: a table that derives the constant
  below it, which is the one measurement form upstream accepts.
- `core/embed/ironwood/src/wire.rs:16-21` — `USER_ADDRESS_BUDGET`: "a unified address
  with every receiver type is about 213 characters."
- `core/embed/ironwood/src/error.rs:1-4` — "Deliberately carries no diagnostic string:
  detailed text would enlarge the firmware interface without being stable or
  actionable at the transport."
- `core/embed/ironwood/src/bench.rs:33-45` — ZIP-224/225 personalizations with the
  bit-length derivations (move with the file, don't delete the reasoning).
- `core/src/apps/zcash/sign_pczt.py:1-9` — module docstring. The `f4jumble.py` shape:
  what it does, what consent means, one doc citation.
- `core/embed/xtask/src/options.rs:287-300` — the F4-exclusion rationale (minus N6).
- `common/protob/messages-zcash.proto:36-39,45-52` — the `@start` / `@next` blocks and
  the "no Regtest value" note. Matches `protocol.md`'s mandatory-documentation rule.

One calibration note: `core/embed/ironwood/src/prewarm.rs:1-30` is a 30-line `//!`
module header. That would be far out of family in Python (17-line ceiling, §1.3) but
is **within Rust norms** — upstream's `micropython/runtime.rs:59` runs to 60 lines
for a safety contract. Keep it; just strip the markers and the dangling file
reference.

---

## Part 4 — The commit series

Current: 45 commits on one branch, 22,217 lines, one PR, **0 of 45 subjects passing
the `commit-msg` hook** (B3). Upstream precedent for the one-big-branch shape is
PR 2472 — 67 files, 73 review threads, never merged (see
`TREZOR_CONTRIBUTION_SHAPING.md`).

Line counts from `git diff --numstat ebd0468e20..HEAD`, bucketed by path.

### Four PRs, nine commits

Titles below are Conventional Commits with scopes from the enforced allowlist
(`common|core|crypto|legacy|python|storage|tools|vendor`).

**PR A — Protocol surface.** Mergeable alone; nothing depends on device code.
Reviewer: protocol owner. ~2,700 lines, almost all generated.

1. `feat(common): add Zcash Ironwood message definitions`
   `common/protob/messages-zcash.proto`, the `MessageType` block in `messages.proto`
   (**at 2300**, B4), `common/protob/Makefile`, `legacy/firmware/protob/Makefile`
   (`SKIPPED_MESSAGES`), `common/tools/cointool.py` (`ALTCOIN_PREFIXES`),
   `common/defs/` if shielded support needs an entry (S6), changelog fragment.
   *Open this first and let it sit — the block allocation is the long-pole external
   dependency and it blocks nothing else.*
2. `feat(common): regenerate protobuf bindings for Zcash`
   `core/src/trezor/messages.py`, `core/src/trezor/enums/*`,
   `python/src/trezorlib/messages.py`, `rust/trezor-client/**`. Pure `make gen`
   output — a reviewer checks the generator invocation, not the diff.

**PR B — Host library.** Depends on A. Reviewer: trezorlib owner. ~2,300 lines.

3. `feat(python): add trezorlib.zcash host bindings` + changelog fragment.
4. `test(python): cover the Zcash host transfer protocol`
   `python/tests/test_zcash_protocol.py`, `test_zcash_client.py`. **Fixtures
   inlined** — drop `python/tests/fixtures/zcash/` (§1.11: no precedent, and 13.7 KB
   of binary `.pczt` will be questioned).

**PR C — Receive + viewing key.** Depends on A. The small, complete, demonstrable
feature — and it should be the first device-side PR: it settles the crate layout, the
build gating, the UI conventions and the CI story on ~1,500 lines instead of 12,000.
Reviewers: core + build.

5. `feat(core): add the Ironwood receiver derivation crate`
   The merged crate (S1) — `ff1.rs`, `generators.rs`, `keys.rs`, `sinsemilla.rs`,
   inline unit tests, `links = "ironwood"`. **One `pasta_curves`, from crates.io**
   (B1/B2 resolved here, or this PR does not open).
6. `feat(core): expose the Ironwood receiver to MicroPython`
   `core/embed/rust/src/ironwood/` (S4), `micropython/ironwood.rs`, `lib.rs`,
   `rustmods.c`, `qstrdefsport.h`, `librust_qstr.h.mako`, `modtrezorutils.c`,
   `core/src/trezor/utils.py`, `core/mocks/generated/*`, the `ironwood` Cargo feature
   across four manifests, the xtask flag + `IRONWOOD_MODELS` + its inline tests,
   `core/embed/supply-chain/config.toml`.
7. `feat(core): derive and confirm Zcash Unified Addresses and export the Orchard FVK`
   `get_address.py`, `get_viewing_key.py`, `account.py`, `layout.py` (S5),
   `unified_addresses.py` delta, `core/src/storage/cache_common.py`,
   `workflow_handlers.py`, TR keys, `core/tests/test_apps.zcash.*`,
   `tests/device_tests/zcash/` (with `pytest.mark.zcash`),
   `tests/ui_tests/fixtures.json`, `core/src/apps/zcash/README.md`, **a CI job**,
   two changelog fragments.

**PR D — Streaming signing.** Depends on C. ~12,000 lines; the one that takes months.
Reviewers: crypto + core + security.

8. `feat(core): add the Ironwood streaming approval core`
   `src/{wire,stream,digest,effects,error,session,prewarm}.rs`, `Engine` gated behind
   `test` (S3), the equivalence/conformance suite, **and the CI job that runs it**.
   ~3,000 src + 5,376 test.
9. `feat(core): stream a PCZT and return Zcash spend authorization signatures`
   `core/embed/rust/src/ironwood/{allocator,signing}.rs`,
   `core/src/apps/zcash/sign_pczt.py`, TR memo keys,
   `docs/common/zcash-ironwood-signing.md` + `docs/SUMMARY.md` +
   `docs/common/index.md` (S6), `core/tests/test_apps.zcash.sign_pczt.py`,
   `tests/device_tests/zcash/test_ironwood_streaming_sign.py` with vectors from
   `common/tests/fixtures/zcash/` (B6), UI fixtures, changelog fragment.

**In no PR:** `ironwood_measurement.py`, `bench.rs`, the `ironwood-measurement`
feature and flag, `session_region_high_water`, `debug_timings`, the `0xff` sentinel,
`test_ram_sweep.py`, `examples/ironwood_fixture.rs`,
`NOTICE` / `DONOR-LICENSE-MIT` / `SOURCE_MAP.md` / `licenses/`,
`python/tests/fixtures/zcash/`, `core/translations/signatures.json`.
That is ~2,000 lines off the top before anyone reads a line of cryptography.

### Ordering rationale

A first (external dependency, blocks nothing). B alongside C (different reviewers).
C before D because C is the smallest change that puts *working Zcash on a Trezor* in
a maintainer's hands — ~1,500 lines they can build, flash, and see an address on. D
then lands against a tree where the crate, the flag, the CI job and the UI
conventions are already settled, so review is about the cryptography and nothing else.

Per `docs/misc/review.md`: no force-pushing during review — answer comments with
`git commit --fixup <hash>`, reply with the fixup hash, and `rebase -i --autosquash`
only after approval.

---

## Part 5 — The three forks: upstreaming plan

Upstream `core/embed/Cargo.lock` has **zero** `git+` sources. All three must become
registry versions (or vendored paths) before PR C opens.

### 5.1 `jarys/pasta_curves` @ `e11dfe40` (pasta_curves 0.4.1)

**Used by:** `trezor-ironwood-receive` only (`core/embed/Cargo.toml:96`,
`core/embed/ironwood-receive/Cargo.toml:10`).
**What the fork provides:** krnák's 2022 embedded fork from PR 2510 — `no_std`,
uninlined-portable field arithmetic for the original `zcash-orchard` branch, which
`ironwood-receive` was re-expressed from.
**What the code actually needs:** `uninline-portable` — **which is a published
feature of crates.io `pasta_curves` 0.5.1**, already used by the sibling alias
`ironwood-pasta-curves` (`core/embed/Cargo.toml:82`).

**Path: delete the fork.** Port `ironwood-receive` to `pasta_curves 0.5.1`. This is
not an upstreaming problem, it is a rebase: the fork is three years stale and its
reason for existing shipped upstream. Doing it also resolves B2 (one `pasta_curves`
in the image), removes `[policy.pasta_curves] audit-as-crates-io`, removes
`core/embed/ironwood-receive/licenses/pasta-curves/` (B8), and is the precondition
for merging the two crates (S1). API drift 0.4 → 0.5 is mostly `group`/`ff`
trait-version churn. **Do this first — highest leverage item in this document.**

### 5.2 `bawolf/sinsemilla` @ `6ca88ff5` (patches crates.io `sinsemilla` 0.1.0)

**Used by:** `trezor-ironwood`, via `[patch.crates-io]`
(`core/embed/Cargo.toml:149-150`), surfaced as the `computed-generators` feature
(`core/embed/ironwood/Cargo.toml:17`).
**What the fork provides:** exact-base *computed* Sinsemilla generators instead of the
precomputed table — ~100× runtime for a large flash saving. This is the lever that
makes the image fit at all.

Four paths, best first:

1. **Upstream the feature into `zcash/sinsemilla`.** A `computed-generators` (or
   `no-tables`) cargo feature, default-off, with a test vector showing byte-identical
   output against the table path. Precedent is strong — `pasta_curves` already carries
   `uninline-portable` for exactly this class of consumer, and Ledger's
   `ledger_zcash_crypto/src/hashtocurve.rs` shows a second hardware vendor needs the
   same thing. Cite that. `sinsemilla` is at 0.1.0; a `0.1.1` with an additive,
   default-off feature is a small ask.
2. **Publish a renamed fork to crates.io** — `trezor-sinsemilla`, or
   `sinsemilla-no-tables`. **This is a shape SatoshiLabs uses themselves:**
   `trezor-noise-protocol` is a published fork of `noise-protocol`, audited in-tree
   with `who = "Martin Milata …"`. `trezor-tjpgdec` and `qrcodegen-no-heap` are the
   same pattern.
3. **Vendor under `core/vendor/sinsemilla/`** with a `version` + `path` dual-spec and
   `[policy.sinsemilla] audit-as-crates-io = true` — exactly how `qrcodegen-no-heap`
   is handled today (`core/embed/Cargo.toml:93`).
4. Fork into the `trezor` GitHub org and add it as a submodule under `core/vendor/`
   — the mechanism used for `stm32u5xx_hal_driver` and `micropython`.

Any of 1–4 is mergeable. The git dep is not.

### 5.3 `bawolf/orchard` @ `d4792911` (patches crates.io `orchard` 0.15.3)

**Used by:** `trezor-ironwood` and `core/embed/rust`
(`core/embed/Cargo.toml:151-152`, `rust/Cargo.toml:31`).
**What the fork provides:** "Sinsemilla-dedup" — routing `orchard`'s internal
Sinsemilla use through the single `sinsemilla` crate instance, so the
computed-generators choice in 5.2 actually takes effect and the code is not
duplicated in flash.
**Path:** depends on 5.2 landing first, then a PR to `zcash/orchard` making it consume
the external `sinsemilla` crate. Frame it as **deduplication** — a win for every
consumer, not just embedded ones — and show the flash delta. `orchard` and
`sinsemilla` share maintainers, so 5.2 and 5.3 are one conversation.
**Fallback:** vendoring `orchard` is far less palatable than vendoring `sinsemilla`
(large, security-critical, and `supply-chain/config.toml:19-20` already imports
Zcash's audit set for the *registry* version — vendoring throws that away). If 5.3
cannot land, the honest fallback is to ship the table generators and find the flash
elsewhere.

### 5.4 The cargo-vet question

The diff adds exactly one line to `core/embed/supply-chain/config.toml`
(`[policy.pasta_curves] audit-as-crates-io = true`) and **no `[[exemptions.*]]`
blocks**, despite pulling ~20 crates (`orchard`, `pczt`, `sapling-crypto`,
`zcash_primitives`, `zcash_protocol`, `zcash_transparent`, `zcash_note_encryption`,
`aes`, `blake2b_simd`, `rand_core`, `sinsemilla`, …). Most are plausibly covered by
the already-imported `zcash/rust-ecosystem` audits (§1.12) — but the two
`[patch.crates-io]` forks are not, and cargo-vet treats a git patch as third-party
unless told otherwise.

**Run `cargo vet` in `core/embed/` and put the result in the PR description.** If it
fails today, that is B1 restated as a CI failure — which is how a maintainer will
first meet it. Note also the expected bar for a crypto fork: `trezor-noise-protocol`
carries **two** independent first-party `[[audits]]` entries. Budget for that.

### 5.5 Flash

98.29% of the T3T1 slot. Levers available before asking upstream for anything:

| lever | est. saving | ref |
|---|---|---|
| one `pasta_curves`, not two | large | B2 / 5.1 |
| gate `Engine` behind `test` | ~500 lines of Rust | S3 |
| drop `bench.rs` + measurement bindings | ~170 lines + bindings | S7 |

A maintainer asks "what is the margin on the smallest supported model" before "is the
cryptography right." Land B2 and S3, re-measure all three models
(T3B1 / T3T1 / T3W1), and put the table in the PR description.

---

## Appendix — What is already right

- **Protobuf.** `common/protob/messages-zcash.proto` — correct `proto2` header,
  correct `java_package`, `<Coin><Verb>` names (passes `check.py`'s mechanical prefix
  rule), `@start` / `@next` annotations, per-field trailing comments, a documented
  enum, and an annotated `reserved` in upstream's own idiom. Registered in all four
  places.
- **trezorlib.** `python/src/trezorlib/zcash.py:1-15` — the LGPL header is
  byte-identical to `stellar.py:1-15`. `from .tools import workflow`, `Session`
  typing, `from __future__ import annotations`, `import typing as t` — the
  *newer* of the two live upstream styles.
- **Feature-flag plumbing.** `USE_IRONWOOD` follows `USE_THP` / `USE_MINISCRIPT`
  exactly through `modtrezorutils.c` → `utils.py` → `rustmods.c` →
  `qstrdefsport.h` → `librust_qstr.h.mako` → generated `.pyi`. The
  `mp_module_trezorironwood` symbol matches the `mp_module_<name>` convention, and
  `micropython/ironwood.rs` correctly mirrors the `thp/mod.rs` + `thp/micropython.rs`
  logic/FFI split.
- **Translations.** `zcash__*` keys, appended to `en.json` in sorted position and to
  `order.json` at 1297-1302. Textbook, and the `<prefix>__` naming is what drives
  altcoin gating in `librust_qstr.h.mako`.
- **UI.** Everything goes through `trezor.ui.layouts`
  (`sign_pczt.py:114,146,185,225,322`). **No new Rust UI components, no new layout
  code, three layouts served by the model-agnostic facade.** This is the single
  biggest reason this branch is more mergeable than PR 2472.
- **xtask.** `IRONWOOD_MODELS` as the single source of truth, with the rejection
  message and both the positive and negative tests derived from it
  (`options.rs:305`, `features.rs:222-320`), written as inline `#[cfg(test)] mod
  tests` — the upstream pattern.
- **trezor-client.** Feature flag, `build_messages` entry, `trezor_message_impl!`
  block, README line. Exactly like the other altcoins.
- **Device tests.** `tests/device_tests/zcash/` with `__init__.py`, correct
  `models` / `setup_client` markers, public "abandon…about" mnemonic. (Add
  `pytest.mark.zcash`, S6.)
- **`docs/common/zcash-ironwood-signing.md`.** Right directory, next to
  `bitcoin-signing.md`, registered in `docs/SUMMARY.md`, and substantively the
  document a reviewer needs (modulo S8's numbering and S6's missing `index.md`
  entry). Citing it by `§N` from source comments instead of restating rules is good
  practice and should survive.
- **The comments themselves.** Strip the review-trail markers and the *content* is
  closer to Trezor's standard than most contributions: it derives budgets
  arithmetically, names the upstream constructions it reproduces, states security
  invariants in attacker terms, and explains why. The problem is citation hygiene
  and placement, not quality.
