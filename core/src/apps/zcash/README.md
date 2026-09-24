# Zcash (shielded)

The shielded Zcash app derives Orchard receivers and full viewing keys for
the wallet's ZIP-32 accounts and signs shielded transactions supplied as
PCZTs. Shielded spends are Ironwood actions, which use the same Orchard keys
and receivers (Unified Address typecode `0x03`). The app is built only when
the firmware is configured with `ZCASH_SHIELDED=1` (`cargo xtask build
--zcash-shielded`), on T3B1, T3T1 and T3W1, and reports
`Capability.Zcash_Shielded` in `Features`.

Transparent Zcash transactions are signed by the Bitcoin app, which uses
`signer.py` and `hasher.py` here for v5 (ZIP 244) transactions. The
`unified_addresses` and `f4jumble` modules are shared by both.

## Workflows

- `get_address.py` (`ZcashGetAddress`): derives the Orchard receiver for
  `m/32'/coin_type'/account'` and the requested diversifier index, encodes it
  as a Unified Address (ZIP 316) and always shows it for confirmation.
- `get_viewing_key.py` (`ZcashGetViewingKey`): exports the Orchard-only
  Unified Full Viewing Key behind a hold-to-confirm screen; the ZIP-32 seed
  fingerprint is a separate opt-in with its own warning, because it links
  every account of the seed.
- `helpers.py`: the network, account, path and session rules the workflows
  share, and the ZIP-315 weak-backup warning shown to 12-word and 128-bit
  wallets.

Key derivation and the cryptography run in the native `trezorzcash` module
(`core/embed/rust/src/micropython/zcash.rs`), backed by the `ironwood` crate
in `core/embed/ironwood`.

## Useful links

- [ZIP 32: Shielded Hierarchical Deterministic Wallets](https://zips.z.cash/zip-0032)
- [ZIP 315: Best Practices for Wallet Handling of Multiple Pools](https://zips.z.cash/zip-0315)
- [ZIP 316: Unified Addresses and Unified Viewing Keys](https://zips.z.cash/zip-0316)
