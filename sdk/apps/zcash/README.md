# Zcash

Shielded Zcash (Orchard/Ironwood) receive addresses, viewing keys and streamed
PCZT signing, with the `ironwood` crate. The app is its own cargo workspace; `xtask modular` finds it
from `core/embed` like the other apps.

The app gets its account keys from Core's ZIP-32 Orchard key service, so the
device tests run against the mnemonic the test harness loads:

```sh
xtask modular build -p zcash -m t3w1 --lang en -d -e
xtask modular device-tests -p zcash -m t3w1 -e
```
