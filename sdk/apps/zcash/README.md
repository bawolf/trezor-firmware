# Zcash

Shielded Zcash (Orchard/Ironwood) receive addresses and viewing keys, with the
`ironwood` crate. The app is its own cargo workspace; `xtask modular` finds it
from `core/embed` like the other apps.

The app cannot get account keys from Core yet, so the device tests need a build
that derives them from the public test mnemonic (`dev-test-seed`):

```sh
xtask modular build -p zcash -m t3w1 --lang en -d -e --features dev-test-seed
xtask modular device-tests -p zcash -m t3w1 -e
```
