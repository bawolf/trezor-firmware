# File-level source map

## Approval core (`src/`)

This GPLv3 Trezor Core component is a reviewed re-expression of project sources
published under the MIT License. `DONOR-LICENSE-MIT` preserves that donor
license. The hashes below are SHA-256 digests of complete donor source files,
not line-level attribution or hashes of the current destination files.

Primary source repository: <https://github.com/bawolf/trezor-ironwood>
Source commit: `12c25c8eb9d9045483a2dbfebc0483a779bd0944`

The additional preserved donor file is committed at
<https://github.com/bawolf/trezor-firmware/tree/8f36fc7aff9b88c9e2e853511a3aceeb8905ac1b/archive/ironwood-build-inputs/crates/approval>
(`ironwood/reconstruction-7105338`).

| Destination | Source at the commit above | Complete-file SHA-256 | Re-expression |
| --- | --- | --- | --- |
| `src/wire.rs` | `crates/approval/src/wire.rs` | `5b05163b419f90d861f1ba5432c62ce0a5690bc747d53c4a9a863804b40413ce` | Admission algorithm and field order retained; failures split into typed malformed, policy, and capacity classes. |
| `src/effects.rs` | `crates/approval/src/effects.rs` | `1eed05b5b9c8831577f564d071d46d25873530a96df5890a24b3ee33e870f1ef` | Upstream v6 transaction assembly and hashing retained; typed failures added. |
| `src/lib.rs` | `crates/approval/src/lib.rs` | `3500810c65dd3550bca2efe84248b38d3e725755eccd95cc3f48b061bbb6d5c8` | State and verification semantics retained; request/limit provenance, borrowed FVK, token binding, compact error taxonomy, standard internal-change OVK recovery, and zeroization only of crate-owned token/session binding arrays plus the temporary FVK encoding re-authored. The retained PCZT/FVK material, RNG state, and ASK remain explicit cleanup gaps. |
| `src/lib.rs` stack boundary | preserved donor `archive/ironwood-build-inputs/crates/approval/src/lib.rs` at `8f36fc7a…` | `dd8e198a0443ab876cb640527342c7cca51bdacb9253bdaacfe673d5e3967bc1` | `#[inline(never)]` verification stack boundary retained; no donor test-only counter-construction API is exposed. |
| `tests/conformance.rs` | `crates/approval/tests/conformance.rs` | `3992da7f3852f158c573c12ab3e0454a257a811da61cfb99e335e56f6d467f34` | Complete hostile/conformance corpus adapted to typed production policy. |
| `tests/common/mod.rs` | `crates/approval/tests/common/mod.rs` | `ad225b031d9dc954a82d7046d1766bbf8ae011f5f652117e525c7909268c8c53` | Synthetic key, RNG, local-consensus/regtest-compatible fixture, and account-9 construction retained only in tests; real Mainnet and Testnet parameter fixtures separately exercise production policy. |
| `tests/action_bounds/mod.rs` | `crates/approval/tests/action_bounds/mod.rs` | `3f66e3782b65bc842f7bf530e13d0a21d0137b4791e57e62db385cb62eaccf0f` | One-through-eight action signature and field-preservation oracle retained; OCK fixtures now use the receiver-appropriate external/internal scope. |
| `tests/randomness/mod.rs` | `crates/approval/tests/randomness/mod.rs` | `7f7daa86cb188ac001fd0136a511aba3f2590698b6f33a0ef68abe9af3ce9e58` | Synthetic host-only entropy fault tests retained and corrected to assert the entropy error class; panic/unwind is a proxy for the device RNG fail-stop contract. |
| `tests/request_disposal.rs` | new integration glue | n/a | Exercises logical terminal consumption and test-only observation of crate-owned token-binding storage. |

Lines not described as retained algorithms above are new integration glue for
the current Trezor workspace. The experimental Madison worktree was used only
as a diff guide and is not a source branch for this component.

## Receiver derivation (`src/receive/`)

This GPLv3 Trezor Core component is a current Rust re-expression of the
allocation-free Orchard receiver implementation from Trezor's historical
`zcash-orchard` branch.

Historical source repository: <https://github.com/trezor/trezor-firmware>
Source commit: `d562254c2195d688ca59296f048ed905047317fe`

| Destination | Historical source | Complete-file SHA-256 | Re-expression |
| --- | --- | --- | --- |
| `src/receive/keys.rs` | `core/src/apps/zcash/orchard/keychain.py` | `f9f2a6c63c6528f2ec63af66540726d9fe46536d328154c04a20e6ba46cff789` | ZIP-32 master and hardened path `m/32'/coin_type'/account'`. |
| `src/receive/keys.rs` | `core/src/apps/zcash/orchard/crypto/keys.py` | `e57fa44881428a171c9ab20ecca90dfff151c7c887682f7f92f900c9c642665d` | Orchard spending-key expansion, FVK validation, diversifier key, and IVK derivation. |
| `src/receive/ff1.rs` | `core/src/apps/zcash/orchard/crypto/ff1.py` | `9d0112791a8101720d589fd2f0717fcc6d1c11aa5127d9aee2e88872fce61b88` | Fixed 88-bit, empty-tweak FF1 used by Orchard diversifier derivation. |
| `src/receive/sinsemilla.rs` | `core/src/apps/zcash/orchard/crypto/sinsemilla.py` | `9a738cf56b7a98f9d8dbda58735d315995f5a5c967e41862fb057ff814075d86` | Fixed 510-bit `CommitIvk` and Orchard diversify hash. |
| `src/receive/generators.rs` | `core/embed/rust/src/zcash_primitives/pallas/generators.rs` | `d48fd241b72b8e6b8b519d9dfdb28480444969040145dd53da6997f96ad8e9c2` | Receive-relevant Pallas generator constants. |

The six end-to-end receiver goldens in `tests/receive.rs` were frozen from the
current `orchard` 0.15.3 implementation using the direct
`SpendingKey::from_zip32_seed` / `FullViewingKey::address_at` path in the
allocator-dependent comparison candidate. The complete oracle test file has
SHA-256 `aebfb562b424d3bf11ede866aee819f9e9cc26a9a597a1ea3756f894eddc0301`.
The fixed inputs are seed bytes `00..1f`, Testnet coin type, account 9, external
scope, and the six full-width diversifier indices listed in the test.

The restored 128-bit SLIP-39 golden uses the same Orchard 0.15.3 oracle with
seed `[0xa5; 16]`, Testnet, account 0, and index 0. It preserves the historical
Trezor mapping for existing wallets as permitted by ZIP 315. Product callers
must show the weaker-backup warning before allowing receive, viewing, or spend;
the native primitive deliberately performs no UI or backup-type policy.

The viewing-key fixtures use published `orchard` 0.15.3 directly through
`SpendingKey::from_zip32_seed` and `FullViewingKey::to_bytes`, then published
`zcash_address` 0.13.0 through `Ufvk::try_from_items` and `Encoding::encode`.
The public seed is bytes `00..1f`; fixtures cover Mainnet accounts 0 and 9 and
Testnet account 7. The independent oracle parses every raw 96-byte value back
through `FullViewingKey::from_bytes` before encoding it. These APIs establish
that the serialized component is `ak || nk || rivk` and contains no spending
key or spend-authorizing scalar.

The Pallas implementation is published `pasta_curves` 0.5.1 with
`default-features = false` and the `alloc` and `uninline-portable` features, the
same instance the signing path links through `orchard`. `uninline-portable` is
the embedded lever the historical work needed a fork for; it has been a
published feature since 0.5.1.

Derivation is byte-identical to the previously pinned
`jarys/pasta_curves` e11dfe4089d0: `from_bytes_wide` is `from_uniform_bytes`
verbatim, and simplified-SWU hashing to the curve is the same algorithm with the
hasher returned as a boxed closure rather than applied in place. Every golden in
`tests/receive.rs` is unchanged and passes.
