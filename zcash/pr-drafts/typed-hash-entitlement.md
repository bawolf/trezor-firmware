# fix(core): sign typed hashes only on an extapp's own paths

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR.
Security-relevant, but in unreleased WIP code with no users, and the fix, its
commit message and test are already public on our fork, so it is handled like
the `bridge-*` notes (README, "Security handling"). `_sign_typed_hash` is
identical on `bieleluk/sdk-wip`, `bieleluk/stabby`, `cepetr/apptool` and
`vojczejk/sdk-wip-modui`; this patch is against sdk-wip.
Rerun 2026-09-26 on sdk-wip and on `cdafac07e0` (same tree as the current tip; T3W1 non-frozen
`--apps` emulator built at each): repro before/after
(`pr-drafts-rerun/logs/typed-hash-{base,tip}.log`), `test_apps.extapp.run.py`
9/9 and the full Core unit suite 136/136 at the tip
(`typed-hash-tip-unit-{run,full}.log`), Ethereum sample suite at base and tip
(`eth-full-{sdkwip,typed-hash}.log`; during the base run the worktree was
switched for about 30 s to a tree with byte-identical Python and tests, see
`INCIDENTS.txt`). Local macOS build fix only (`15de3664a6`
cherry-picked for the emulator app); not needed on Linux.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/typed-hash-entitlement
@ `23599d27b6`. It is on `extapp/run-coin-types` @ `7b126357ca`, so it **depends on
run-coin-types** (2 commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`), and adds its test class
to that branch's `test_apps.extapp.run.py`. It touches only `core/src/apps/extapp/run.py`
and that test file.

---

With Ethereum definitions (a network, a token or a chain id), `SignTypedHash`
built its secp256k1 keychain from Ethereum's path patterns instead of the
app's. So any loaded app, whatever curve and paths its header declares, got an
Ethereum signature over any hash it chose, with no screen.

**Reproduce.** The Tron sample declares only `m/44'/195'/…`. With
[repro/typed-hash-entitlement/get_address.patch](repro/typed-hash-entitlement/get_address.patch),
its `get_address` asks for
`crypto::sign_typed_hash(&[44|H, 60|H, H, 0, 0], &[0x11; 32], None, None, Some(1), false)`
on an empty path and returns the signature in `mac`. Build it with
`xtask modular build -p tron -m t3w1 --lang en -e`, load it on a non-frozen
T3W1 `--apps` emulator, and run
[test_zz_typed_hash_repro.py](repro/typed-hash-entitlement/test_zz_typed_hash_repro.py)
from `sdk/apps/tron/tests/`. It checks the signature against Core's own
`EthereumGetPublicKey` for m/44'/60'/0'/0/0 (hex trimmed, line wrapped):

```
before: Failed: the Tron app got a signature by 0x73d0385F4d8E00C5e6504C6030F47BF6212736A8
        (m/44'/60'/0'/0/0) over 1111…1111: 1c4a76a4…d92f16b; verifies: True
after:  refused: UnexpectedMessage:          (Core log: Failed to sign typed hash)
```

**Cause.** `_sign_typed_hash` checked the app's schemas only without
definitions; with them it used `_schemas_from_network(PATTERNS_ADDRESS, …)`.

**Fix.**
- `_sign_typed_hash` takes the app's curve and schemas, and refuses any curve
  but secp256k1.
- Without definitions, the path must match the app's schemas, as before.
- With definitions, the path with its coin type replaced by 60 must match the
  app's schemas, and then the path must also be the network's, as before. This
  is how Core's Ethereum app uses one path layout across EVM networks,
  including its path warning under relaxed safety checks.

**Tests.** `TestExtappSignTypedHash` in `core/tests/test_apps.extapp.run.py`:
`test_ethereum_app` (with and without a chain id),
`test_ethereum_app_on_the_network_of_the_definitions` (Ethereum Classic),
`test_app_without_ethereum_paths`, `test_app_with_another_curve`. 4/4, and
the full Core unit suite passes. The Ethereum sample passes the same 59 of
its 336 device tests on sdk-wip and on this branch; most of the rest stop at
a `TypeError` in the tests' definition builder, before reaching the app.

### Notes for QA
The Ethereum sample signs as before. An app that asks for a typed-hash
signature on a path it does not declare for Ethereum gets an error.
