# Zcash

Shielded Zcash: unified addresses and viewing keys. The keys come from
Core's ZIP-32 Orchard key service.

```sh
xtask modular build -p zcash -m t3w1 --lang en -e
xtask modular device-tests -p zcash -m t3w1 -e
xtask modular unit-tests -p zcash -m t3w1
```

The signer crate's unit tests, and its tests against librustzcash, which
have their own lockfile:

```sh
cargo test -p zcash-signer
cargo test --manifest-path signer-tests/Cargo.toml
```
