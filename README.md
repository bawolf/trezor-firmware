# Zcash / Ironwood for Trezor

This firmware fork runs a Zcash app on the Trezor. The app displays receiving
addresses, checks payments and signs them after on-device approval. It uses
Trezor's experimental extension SDK and a firmware service that provides access
to the wallet's account keys.

A modified Trezor Suite, with a Rust wallet engine, handles transaction
preparation, proof generation and broadcasting. That desktop side is not public.

The SDK is a work in progress. This prototype uses fixed versions, a custom
account-key service and development app signing to test the complete workflow.
See [SDK integration](sdk/apps/zcash/README.md#sdk-integration) for those choices.
Our goal is to contribute Zcash support that Trezor can review, maintain and ship.
Trezor's acceptance of the design and changes remains open.

## Development status

**Unofficial experimental software, not a Trezor release. Use test wallets and
test coins; keep savings off these builds.**

This branch is the exact firmware source, `7a438a4399`, that ran on a Safe 7 for
two small mainnet receive/return demonstrations: on 16 October 2026 and again,
filmed, on 19 October 2026
([in](https://zecblock.com/tx/313387d4dfc283af5b1bed36b76aaea171321b96fb3e9f165b94c37e3dedfe09),
[out](https://zecblock.com/tx/f7f97e2912064ef4c54cb84da4afe59aad07be7f3fb45720d5d692fd546d66f7)).
Safe 5 is the primary product target; its extension is blocked by failed requests
and a Core runtime crash. Nothing here has had an independent security audit.

The [Zcash app guide](sdk/apps/zcash/README.md) gives exact revisions,
prerequisites and build/test commands.

## Related repositories

- [Orchard fork](https://github.com/bawolf/orchard/tree/ironwood/sinsemilla-progress)
  and [Sinsemilla fork](https://github.com/bawolf/sinsemilla/tree/ironwood/sinsemilla-progress):
  changes to fit signing in device memory. This revision's
  [`sdk/apps/Cargo.toml`](sdk/apps/Cargo.toml) pins Orchard
  [`61704d46`](https://github.com/bawolf/orchard/commit/61704d46253e9ac9dc97358b0e17f82bc2b7fc1f)
  and Sinsemilla
  [`6607cb01`](https://github.com/bawolf/sinsemilla/commit/6607cb019a587869a183b04172e20b4d2719a8e7),
  the tips of those `ironwood/sinsemilla-progress` branches. Use the pins, not a
  later branch tip.
- The modified Trezor Suite and wallet engine used for the demonstrations are
  not public. Suite's license (T-RSL) does not clearly permit sharing a modified
  version.

The general firmware documentation follows.

---

# Trezor Firmware

![img](https://repository-images.githubusercontent.com/180590388/968e6880-6538-11e9-9da6-4aef78157e94)

## Repository Structure

* **[`ci`](ci/)**: Ancillary files, data, and scripts for the CI pipeline
* **[`common/defs`](common/defs/)**: JSON coin definitions and support tables
* **[`common/protob`](common/protob/)**: Common protobuf definitions for the Trezor protocol
* **[`common/tools`](common/tools/)**: Tools for managing coin definitions and related data
* **[`core`](core/)**: Trezor Core, a firmware implementation for Trezor T, Trezor Safe 3, Trezor Safe 5, and Trezor Safe 7
* **[`crypto`](crypto/)**: Stand-alone cryptography library used by both Trezor Core and the Trezor One firmware
* **[`docs`](docs/)**: Assorted documentation
* **[`legacy`](legacy/)**: Trezor One firmware implementation
* **[`python`](python/)**: Python [client library](https://pypi.org/project/trezor) and the `trezorctl` command
* **[`storage`](storage/)**: NORCOW storage implementation used by both Trezor Core and the Trezor One firmware
* **[`tests`](tests/)**: Firmware unit test suite
* **[`tools`](tools/)**: Miscellaneous build and helper scripts
* **[`vendor`](vendor/)**: Submodules for external dependencies

## Contribute

See [CONTRIBUTING.md](docs/misc/contributing.md).

Using [Conventional Commits](COMMITS.md) is strongly recommended and might be enforced in the future.

Also, please have a look at the docs, either in the `docs` folder or at [docs.trezor.io](https://docs.trezor.io) before contributing. The [misc](docs/misc/index.md) chapter should be read in particular, as it contains useful assorted knowledge.

## Security Vulnerability Disclosure

Please do NOT create publicly viewable issues for suspected security vulnerabilities. See the [Security tab](https://github.com/trezor/trezor-firmware/security/policy) for reporting instructions.

## Documentation

See the `docs` folder or visit [docs.trezor.io](https://docs.trezor.io).
