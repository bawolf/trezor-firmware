# Shippability checklist — Trezor Ironwood

Date: 2026-09-19. Track 4 (provenance / contribution prep). Docs-only; no firmware,
fork, or upstream mutation. This is the ordered list of everything that must be true
before SatoshiLabs could reasonably ship Zcash Ironwood signing on a Trezor device,
each with current status and owner.

Sources: `.context/product-scaffold/integration-health/RECON.md`,
`docs/PRODUCT_ROADMAP.md`, `experiments/fork-migration/signing-feasibility/DEPENDENCY-FORKS.md`,
`.context/product-scaffold/receive-viewing-review/FABLE-REVIEW.md`,
`.context/product-scaffold/cross-session-fix/FABLE-REVIEW.md`, `docs/UPSTREAM.md`,
`docs/STATUS.md`.

## Legend

- Status: **done** / **in-progress** / **blocked-on-Trezor** (SatoshiLabs must act) /
  **blocked-on-external** (outside anyone here, e.g. testnet infra).
- Owner: **us** (this project) / **SatoshiLabs** / **external**.
- "Track" points at whichever parallel track owns the actual edit, since Track 4 is
  docs-only. The firmware worktree
  `/Users/bryantwolf/conductor/workspaces/trezor-firmware/streaming-signing` and the
  forks are owned by other tracks.

---

## A. Feasibility gates (must be true to have a thing to ship)

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| A1 | Safe 5 signs a real Ironwood tx within RAM/flash bounds | **done** | us | O(1) RAM proven on hardware at 2 and 16 actions (45,760 B), 32 processed fault-free, fits flash with ~50 KB free. |
| A2 | Device output slots into the standard Zcash pipeline with no glue | **done** | us | Signature record IS the librustzcash `SpendAuthSignature{ValuePool::Ironwood,…}`; applied via standard `pczt` Signer. Verified offline end-to-end (real device sig verifies host-side at v6 sighash). |
| A3 | Desktop end-to-end (construct→prove→sign→extract→verify) | **in-progress** | us | Host spike ran the whole chain, exit 0, node-free. Now promoted to a `desktop-e2e` driver. Redaction/retention wiring (host keeps unredacted PCZT) known-sound, not yet wired. |
| A4 | Cross-session signing region lifetime fix | **in-progress** | us | `.buf`-rooted region implemented; Fable ACCEPT-WITH-MUST-FIX (0 blockers for measurement, 1 production must-fix: freed-block residue persists for the boot). Needs device session confirming 2-signs-per-boot + Bitcoin heap coexistence. |

## B. Provenance of the delivery vehicles (the two forks)

The forks are **SHA-pinned delivery vehicles, not source of truth** (see
`DEPENDENCY-FORKS.md`). Only the firmware needs them; the host/desktop side needs no
forks (Ironwood is in the published `zcash_primitives` 0.30.1 / `pczt` 0.9.3 /
`orchard` crates — see item D3).

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| B1 | `sinsemilla` fork pin pushed and reachable | **done** | us | `bawolf/sinsemilla` `ironwood/computed-generators-v0.1.0` = `6ca88ff5…` matches the Cargo pin; branch is pushed. Base `206f7a9` (v0.1.0) + 1 commit. |
| B2 | `orchard` fork pin pushed and reachable | **done (verified)** | us | Cargo pin `d4792911…` (3rd commit). RECON flagged this as unpushed → clean-checkout/CI build break; now fixed. Verified 2026-09-19: `git branch -r --contains d4792911` resolves to `bawolf/ironwood/direct-scope-classifier`. Caveat: pushed under the un-suffixed branch name (no `-v0.15.3`) — cosmetic, see B6. |
| B3 | Review of the two later orchard commits | **blocked / in-progress** | us | `5f527f8` (Sinsemilla dedup on verify) and `d479291` (wipe ScopeClassifier ivk cache) are UNREVIEWED. Only `19f6100` (scope classifier) had an Opus 5 review. Genuine Fable owed on all fork deltas. |
| B4 | cargo-vet delta audit recorded for both fork commits | **blocked-on-Trezor track (another track)** | us | POINTER ONLY — the edit lives in the firmware worktree, not here. `core/embed/supply-chain/config.toml` today adds only `[policy.pasta_curves] audit-as-crates-io = true`. **Delta still needing recording:** a `cargo vet` git-delta / `audit-as-crates-io` + delta-audit entry for each of `sinsemilla@6ca88ff` and `orchard@d4792911` (the zcash imported audits cover the published crates, not the fork deltas). Track 4 does not make this edit. |
| B5 | `git format-patch` series + `apply.sh` reproduce each fork tree onto its base | **done** | us | Series stored under `experiments/fork-migration/dependency-patches/<crate>/`; both re-applied onto their bases 2026-09-16, reproduced the pushed trees (receipts under `dependency-patches/receipts/`). |
| B6 | Local branch names carry the `-vX.Y.Z` suffix used on pushed branches | **in-progress** | us | Cosmetic; local `ironwood/computed-generators`, `ironwood/direct-scope-classifier` drop the suffix. Tighten before handoff. |
| B7 | Fork exit path defined (upstream merge → plain crates.io pin, branch retired) | **done (documented)** | us | Contract in `DEPENDENCY-FORKS.md`. Exit itself is gated on upstream acceptance (item E) and requires user authorization for contact. |

## C. Protocol / wire IDs

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| C1 | Wire message-type IDs 32100–32106 + 32109 assigned (32107/32108 reserved, not claimed) | **blocked-on-Trezor** | SatoshiLabs | Provisional local-only block of eight live IDs; proto says so explicitly. Trezor message-type IDs are assigned by SatoshiLabs maintainers in `trezor/trezor-firmware` `common/protob` — no external ZIP/registry governs *wire* IDs. Isolated at 32100+, collision risk low. See `WIRE_ID_COORDINATION.md`. |
| C2 | Minimal message-ID-reservation PR drafted (not opened) | **done (draft)** | us | Draft PR description in `WIRE_ID_COORDINATION.md`. Parked pending user authorization; no PR opened, no maintainer contact. |
| C3 | `messages-zcash.proto` schema stable | **done** | us | Response contract settled 2026-09-21 (firmware `dd6701f76b`): `ZcashSpendAuthSignatures` records (32109) are the single canonical response; the legacy whole-PCZT download pair was deleted and its IDs 32107/32108 reserved/freed, never reused. Remaining churn is only a maintainer relocation of the base offset (C1). |

## D. Signing / build integrity

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| D1 | Production vendor signing | **blocked-on-Trezor** | SatoshiLabs | Images are dev-signed under the `UNSAFE, DO NOT USE!` vendor header (`unsafe_signed_prod`, `headertool -h -D`; SAFE5_HARDWARE_GATES gate 2). A locked retail bootloader rejects this vendor ("Install restricted"); unlocking irreversibly erases secrets. Retail shipping needs SatoshiLabs production vendor signing — outside our control. |
| D2 | Firmware builds from a clean tree / CI | **in-progress** | us | Gated on B2 (orchard pin reachable). Once B2 verified, the git+rev sources resolve. |
| D3 | Desktop side needs no forks | **done** | us | Construction + proving use published `zcash_primitives` 0.30.1 (`DeferredPcztBuilder`), `pczt` 0.9.3 Prover, `orchard`. Ironwood is in the PUBLISHED crates. Fork dependency is firmware-only (device Sinsemilla-dedup perf). |

## E. Reviews (feature-freeze / Fable)

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| E1 | Receive (`ZcashGetAddress`) Fable review | **in-progress** | us | Fable 5.1: **ACCEPT-WITH-MUST-FIX**, 1 blocker R1 — the op-timing bench hook (reserved `0xff` diversifier index → blocking compute + region telemetry, pre-consent, not build-gated) leaked into the receive path. Also makes legitimate index `2^88−1` underivable. |
| E2 | R1 fix: build-gate the bench + telemetry behind a measurement feature | **blocked-on-Trezor track (another track)** | us | Firmware edit, not Track 4. Closes the backdoor AND recovers flash margin (bench scaffolding helped push the image to 98.17%). Do after the device session. |
| E3 | Re-run receive evidence after R1/bench-gating | **blocked (on E2)** | us | Fable noted evidence staleness: RESULT.md is for tree `e384a4d38d`; the hook entered in `c3244fe65a`. The validated receive handler is not the one in the tree. Must re-run receive evidence on the gated tree. |
| E4 | Viewing export (`ZcashGetViewingKey`) Fable review | **done** | us | Fable 5.1: **ACCEPT**, 0 must-fix; never releases spend authority (S2). Two should-fix (V1, V2). |
| E5 | Cross-session region fix Fable review | **done (accept-with-must-fix)** | us | 0 blockers for measurement; 1 production must-fix (freed-block residue). Safe to flash for measurement (dev-signed, public wallet, no consent-logic change). |
| E6 | Signing (integrated streaming firmware) genuine Fable review | **in-progress** | us | Streaming *design note* got a real Fable 5.1 review (1 blocker + 7 must-fixes, dispositioned). The integrated streaming firmware still owes a genuine adversarial Fable pass. |
| E7 | Feature-freeze reviews (spend-key scalar zeroization + clarity) | **in-progress** | us | Zeroization + feature-freeze Fable + clarity reviews are Chunk-1 remaining items (PRODUCT_ROADMAP). |

## F. Design-boundary rejections (must stay true through mixed-pool flows)

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| F1 | Sapling by-design rejection preserved | **done (by design)** | us | No Sapling wallet support; device rejects unsupported pools; THREAT_MODEL checks integer accounting across transparent/Sapling/Orchard/Ironwood value balances. `DEPENDENCY-FORKS` forbids a Sapling path; any future empty-pool sighash helper must only specialize proven-empty pools while retaining device rejection. Keep rejection intact through mixed-pool flows. |

## G. Upstream acceptance path (fork exit; long-pole)

| # | Item | Status | Owner | Notes |
|---|------|--------|-------|-------|
| G1 | Rebase currency vs. upstream | **done (with staleness caveat)** | us | Our 14-commit stack sits directly on the fetched upstream tip (`dc99b24…`); nothing to rebase. Mirror last fetched Sep 16; a fresh `git fetch upstream` is needed to confirm real trezor/main has not moved (network op, not done). If it moved, expect mechanical conflicts in `Cargo.lock`, `MessageType`/`messages.proto` enum inserts, translations, `workflow_handlers.py`. |
| G2 | Upstream-shaped, independently-reviewable change sequence | **done (documented)** | us | See `TREZOR_CONTRIBUTION_SHAPING.md`. |
| G3 | Maintainer-acknowledged issue before any librustzcash PR | **blocked-on-Trezor / not started** | SatoshiLabs + us | librustzcash `AGENTS.md` requires a maintainer-acknowledged issue before a PR; none recorded; no push/maintain access. BACKLOG M1.3 "first upstream proposal" Pending. **No upstream contact authorized.** |
| G4 | Parked upstream PRs assessed | **done** | us | PR 2472 (Model T Orchard, predates Safe 7 build; memory-alloc + Python review still listed) and PR 2510 (Rust crypto primitives) assessed as reference, not transplant. See `docs/TREZOR_REUSE.md`. |

---

## Top blockers, ranked

1. **Production vendor signing (D1)** — blocked-on-Trezor / SatoshiLabs. Structural
   gate to any retail ship; not in our control. The single hardest gate.
2. **Wire-ID assignment (C1)** — blocked-on-Trezor / SatoshiLabs. "Provisional" blocks
   a shippable protocol claim; needs maintainer allocation in `common/protob`.
   Collision risk low (isolated 32100 block). We can hand them a ready reservation PR.
3. **Fork provenance + review debt (B2/B3/B4, E1–E3, E6)** — ours, fixable now.
   Verify the orchard pin is reachable, review the two later orchard commits, record
   the cargo-vet deltas (another track's edit), build-gate the receive bench (R1) and
   re-run receive evidence, and land the integrated-signing Fable pass.

Everything in the top 3 that is ours is cheap and near-term; the two hardest gates
(D1, C1) are SatoshiLabs decisions we can only prepare for. No item is blocked-on-external
except broadcast infra (NU6.3 testnet v6 lightwalletd/zebrad), which blocks only
networked broadcast, not the cryptographic end-to-end.
