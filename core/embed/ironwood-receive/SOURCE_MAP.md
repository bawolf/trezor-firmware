# Source map

This GPLv3 Trezor Core component is a current Rust re-expression of the
allocation-free Orchard receiver implementation from Trezor's historical
`zcash-orchard` branch.

Historical source repository: <https://github.com/trezor/trezor-firmware>
Source commit: `d562254c2195d688ca59296f048ed905047317fe`

| Destination | Historical source | Complete-file SHA-256 | Re-expression |
| --- | --- | --- | --- |
| `src/keys.rs` | `core/src/apps/zcash/orchard/keychain.py` | `f9f2a6c63c6528f2ec63af66540726d9fe46536d328154c04a20e6ba46cff789` | ZIP-32 master and hardened path `m/32'/coin_type'/account'`. |
| `src/keys.rs` | `core/src/apps/zcash/orchard/crypto/keys.py` | `e57fa44881428a171c9ab20ecca90dfff151c7c887682f7f92f900c9c642665d` | Orchard spending-key expansion, FVK validation, diversifier key, and IVK derivation. |
| `src/ff1.rs` | `core/src/apps/zcash/orchard/crypto/ff1.py` | `9d0112791a8101720d589fd2f0717fcc6d1c11aa5127d9aee2e88872fce61b88` | Fixed 88-bit, empty-tweak FF1 used by Orchard diversifier derivation. |
| `src/sinsemilla.rs` | `core/src/apps/zcash/orchard/crypto/sinsemilla.py` | `9a738cf56b7a98f9d8dbda58735d315995f5a5c967e41862fb057ff814075d86` | Fixed 510-bit `CommitIvk` and Orchard diversify hash. |
| `src/generators.rs` | `core/embed/rust/src/zcash_primitives/pallas/generators.rs` | `d48fd241b72b8e6b8b519d9dfdb28480444969040145dd53da6997f96ad8e9c2` | Receive-relevant Pallas generator constants. |

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
