# Zcash app: shielded addresses, viewing keys and PCZT signing

Status: **a feature proposal, last in the order; not a PR yet.** Trezor asks
for product agreement before a new coin (#6962 puts Zcash after the
modularization; #6770 invites third parties). Based on the draft SDK
(`bieleluk/sdk-wip`, #7516), so it cannot target `main`. It depends on the
whole platform part of our series, including the key service
(`key-service.md`), which needs design agreement first.
Kind: feature. Reproduction status: not applicable.

Branch: https://github.com/bawolf/trezor-firmware/tree/zcash/extapp-series @ `cbce6b97e2`, commits:
- `1a9708f824` feat(extapp): add zcash signer crate
- `ead470546a` feat(extapp): add zcash app with address and viewing key
- `bc13c3a72d` feat(extapp): sign PCZTs in the zcash app
- `cbce6b97e2` ci(sdk): test the zcash app and its signer

---

Adds a Zcash app that shows the account's shielded unified address, exports its
Orchard viewing key, and signs PCZTs that spend from the Ironwood pool. The host
streams the PCZT to the app, which verifies it one action at a time. Related:
#6962, #6770.

**Please read first.**
- **Merge blocker: forked dependencies.** `sdk/apps/Cargo.toml` patches
  `orchard` to github.com/bawolf/orchard @ `61704d4` and `sinsemilla` to
  github.com/bawolf/sinsemilla @ `6607cb0`. Released orchard already supports
  Ironwood; the forks add only APIs no release has: the `*_with_progress`
  functions (progress inside Sinsemilla hashes, to stay within Core's 1 s IPC
  limit), `ScopeClassifier`, `recover_output_bound_with_*`, `wipe`, and
  sinsemilla's `computed-generators`. They must be replaced by released crates
  before merge.
- **Dependencies are exempted, not audited.** `cargo vet --locked` passes on
  exemptions only: 62 new in `sdk/apps/supply-chain` (58 `safe-to-deploy`, 4
  `safe-to-run`; the two forks marked `audit-as-crates-io` and exempted at
  `safe-to-deploy`) and 149 `safe-to-run` in `signer-tests/supply-chain`. No
  new crate is audited; the exemptions only pin the set. The CI job's
  `cargo audit` step has never been run.
- **Safe 5 signing is slower.** The Sinsemilla generator table does not fit the
  T3T1 app arena, so the T3T1 build computes the generators
  (`model_t3t1 = ["zcash-signer/computed-generators"]`, −66,112 B). On the
  host, verifying 32 actions takes 384–395 ms computed against 114–120 ms with
  the table, about 3.4×. Scaled by 260–440× that is an estimated 100–170 s of
  verification for a 32-action PCZT on a Safe 5, not measured on a Safe 5. On a
  T3W1 development device (table; an earlier build of this app), a 32-action
  sign took 81.5 s wall, including
  the user's time on its 34 screens, and the longest IPC silence was 213 ms of
  the 1,000 ms limit. The estimated worst silence on a Safe 5 is 0.26–0.36 s.
- **Sizes.** The T3T1 arena exists only in sdk-wip's WIP commit `534e35daa9`.

  | | T3T1 | T3W1 |
  |---|---|---|
  | release image, code + data | 225,024 of 236,544 B (11,520 B left) | 291,296 of 393,216 B (101,920 B left) |
  | declared stack / worst static stack | 32,768 / 28,804 B | 32,768 / 28,796 B |
  | declared heap | 69,632 B | 69,632 B |
  | IPC inbox | 2,048 B | 2,048 B |

  Heap peak while signing: 57,632 B on the T3W1 emulator; 57,452 B on a T3W1
  device with the earlier build.

- **App id.** `zcash.trezor.com` is a placeholder, and the vendor is "Bryant
  Wolf"; the id and production signing are Trezor's.

**What it does.**
- `GetAddress`, `GetViewingKey` for `m/32'/{133,1}'/[0-100]'`; other paths are
  refused before any screen. The address is always shown for the user to
  confirm; the viewing key is behind a hold-to-confirm privacy warning, and the
  seed fingerprint is returned only on request, behind a second one.
- `SignPczt`: the app requests the PCZT in 1 KiB chunks (at most 64 KiB and 32
  actions) and checks each action against the account's keys (value
  commitment, nullifier, `rk`, note commitment, note encryption, outgoing
  recovery). It shows every payment and its memo, then the totals, and returns
  one RedPallas signature per real spend, with nonces hedged by the spending
  key. The fee is capped at 0.01 ZEC and the expiry at 100 blocks after the
  host's height.
- Keys come from Core's ZIP-32 Orchard key service, which asks the user once
  per app instance, verified image and account. The app never sees the seed.
- `zcash-signer` (`sdk/apps/zcash/signer`): the no_std verifier and signer.
  `signer-tests` checks it against librustzcash and generates the device-test
  fixtures; it has its own lockfile and vet store, because its pre-release
  `digest` conflicts with the SDK's test mock.
- CI: zcash joins the extapp matrices (English only), plus
  `make extapp_zcash_signer_{test,audit,vet}` in a job of its own.

**Tests.**
- Device tests with UI fixtures (`pytest --app=… tests/ --ui=test` from
  `sdk/apps/zcash`) on T3W1 and T3T1 `--apps` emulators: 77 passed, 1 xfailed,
  0 UI diffs on each.
- `xtask modular unit-tests -p zcash -m {t3w1,t3t1}`: 11 each.
- Signer: unit tests 21 (also 21 with `computed-generators`); tests against
  librustzcash in `signer-tests` 88 (also 88 with `computed-generators`);
  `make extapp_zcash_signer_test` runs them. Signatures are applied with
  the upstream `pczt` `Signer`, which checks them against the upstream sighash,
  for 1–32 actions; every single-byte mutation is cross-checked with the
  upstream parser and `Verifier`.

**What the tests do not show.** Device signatures are checked for count, order
and length only: RedPallas signatures are randomized, so fixtures cannot hold
them. Their validity is checked on the host with the same signer crate. A
Trezor-level `Cancel` during signing leaves the app running, but the next
request gets the abandoned request's failure (the `xfail`; see
`extapp/host-cancel`).

**Not included.** Transparent inputs, Sapling or Orchard spends, Regtest,
`show_display=false` or an address MAC on `GetAddress`, languages other than
English, Suite/Connect integration.

### Notes for QA
- The device tests use the standard "all all … all" 12-word seed, so every
  request shows the app's ZIP-315 weak-backup warning.
- The first key request per app instance shows Core's "Spending key" screen
  (hold to confirm).
- Expect signing on a Safe 5 to be several times slower than on a Safe 7.
