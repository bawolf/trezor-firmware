# fix(core): sign typed hashes only on an extapp's own paths

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR. The code
exists only on the draft SDK branches (identical on `bieleluk/stabby`,
`cepetr/apptool`, `vojczejk/sdk-wip-modui`). Security-relevant but unreleased;
consider sending it to the owner directly rather than in public.
Reproduction status: standalone end-to-end repro on the emulator (scratch
Tron change), before and after; Core unit tests after.

**Blocker: rebuild the branch before sending.** `3e668c7caf` has three hunks
that its series commit `b77222b9ae` does not have and its message does not
mention: the `SignDigest`, `GetAddressMac` and `CheckAddressMac` arms build
their keychain with `"secp256k1"` instead of the app's `curve`. For a nist256p1
or ed25519 app, `SignDigest` would then sign with a key derived on the wrong
curve. `b77222b9ae` cherry-picks cleanly onto `extapp/run-coin-types` (it
adds its test class to that branch's test file), so recreate the branch that
way and rerun the checks below; the current branch also conflicts with
`extapp/run-coin-types` in both files.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/typed-hash-entitlement @ `3e668c7caf`
(one commit on `bieleluk/sdk-wip` @ `4cd93ff4d8`; to be rebuilt on
`extapp/run-coin-types`, see above)

---

With Ethereum definitions (a network, a token or a chain id), `SignTypedHash`
built its secp256k1 keychain from Ethereum's path patterns instead of the
app's. So any loaded app, whatever curve and paths its header declares, got an
Ethereum signature over any hash it chose, with no screen.

**Reproduce.** Scratch change to the Tron sample (declares only
`m/44'/195'/…`): in `get_address`, an empty path calls
`crypto::sign_typed_hash(&[44|H, 60|H, H, 0, 0], &[0x11; 32], None, None, Some(1), false)`
and returns the signature in `mac`. Build with
`xtask modular build -p tron -m t3w1 --lang en -e`, load it on a non-frozen T3W1
`--apps` emulator, and request that address. A test then checks the signature
against Core's own `EthereumGetPublicKey` for m/44'/60'/0'/0/0. Observed:

```
before: Failed: the Tron app got a signature by 0x73d0385F4d8E00C5e6504C6030F47BF6212736A8
        (m/44'/60'/0'/0/0) over 1111…1111: 1c4a76a4…d92f16b; verifies: True
after:  refused: UnexpectedMessage   (Core log: Failed to sign typed hash)
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
  including its path warning under relaxed safety checks. A first version that
  required the path itself lost two sample tests that sign on
  `m/44'/66666'/…` for a network from an external definition.

**Tests.** `TestExtappSignTypedHash` in `core/tests/test_apps.extapp.run.py`:
`test_ethereum_app` (with and without a chain id),
`test_ethereum_app_on_the_network_of_the_definitions` (Ethereum Classic),
`test_app_without_ethereum_paths`, `test_app_with_another_curve`. 4/4. The
Ethereum sample suite passes the same 81 tests with and without the fix.

### Notes for QA
The Ethereum sample signs as before. An app that asks for a typed-hash
signature on a path it does not declare for Ethereum gets an error.
