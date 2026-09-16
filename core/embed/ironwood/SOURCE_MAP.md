# File-level source map

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
