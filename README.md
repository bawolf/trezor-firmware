# Zcash shielded support — working notes

This branch (`ironwood/notes`) holds the records for the Zcash shielded (Orchard/Ironwood)
series in this fork. It shares no history with the firmware and is never part of a PR.
Nothing here has been sent to Trezor, and no PR is open.

| Branch / tag | What it is |
|---|---|
| `zcash/ironwood-upstream-v2` | The upstream-shaped series: 12 commits on `upstream/main` `148e530180`. The current head. |
| `archive/*` tags | Commits that hardware sessions and reviews cite: the image now on the test Safe 5 (`cff2c52521`), earlier heads `87dc48e0aa`, `252cda1b52` and `170ea78cf4`, and the first series cut `3e737c17ca`. |

| Note | Contents |
|---|---|
| [zcash/UPSTREAM_SERIES.md](zcash/UPSTREAM_SERIES.md) | The commit series, gates, per-PR buildability, `cargo vet`, what remains before a PR, and PR description drafts |
| [zcash/WIRE_ID_COORDINATION.md](zcash/WIRE_ID_COORDINATION.md) | The 2300–2307 message block and `Capability_Zcash_Shielded = 30`: the request to maintainers |
| [zcash/MERGEABILITY_REVIEW.md](zcash/MERGEABILITY_REVIEW.md) | Code-level audit against Trezor's own rules (2026-09-22) |
| [zcash/TREZOR_CONTRIBUTION_SHAPING.md](zcash/TREZOR_CONTRIBUTION_SHAPING.md) | Why the series is split the way it is |
| [zcash/SHIPPABILITY_CHECKLIST.md](zcash/SHIPPABILITY_CHECKLIST.md) | Everything needed before Trezor could ship it |
| [zcash/reviews/](zcash/reviews/) | Adversarial and clarity reviews of the series, with the model each ran on |

Host-side code lives in `bawolf/trezor-ironwood`: the wallet engine, its napi binding, the
Zebra + Zaino regtest service and the Linux emulator recipe for trezor-user-env. Local
paths in older notes (`/Users/...`) are the author's machine, kept as provenance.
