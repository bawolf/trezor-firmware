# Trezor contribution shaping — how to make this upstream-reviewable

Date: 2026-09-19. Docs-only; no PR opened, no upstream contact. This maps the whole body
of work into small, independently-reviewable, upstream-shaped changes in the sequence and
dependency order a SatoshiLabs maintainer would want, with each mapped to its
branch/commit and current review status.

## Framing: what upstream has seen before

Two precedents shape what "reviewable" means here.

**krnák's 2022 attempt — [PR 2472](https://github.com/trezor/trezor-firmware/pull/2472).**
The prior Zcash-shielded effort (jarys/krnák) was a single large branch: 67 changed files,
a custom Orchard transaction-v5 protocol, custom protobuf message IDs and fields, patched
Pasta dependencies, its own Rust/MicroPython wrappers. GitHub shows 73 review threads
(8 unresolved), a 2022 cryptography review that explicitly limited its coverage, and
requests for memory-allocation review, address/viewing-key tests, and rebasing. Its stacked
Rust-primitives PR ([2510](https://github.com/trezor/trezor-firmware/pull/2510), 43 files,
13 threads) is a dependency of it. **Lesson: one big branch carrying protocol + crypto +
UI + deps at once did not converge.** It predates the Safe 7 build system, so it cannot be
transplanted; it is a reference, and a cautionary tale about size and coupling.

**The Ledger precedent — LedgerHQ/app-zcash.** Ledger is shipping an Ironwood HWW signing
app (closest shipped analogue). Its design is instructive for *shape*: a chunked PCZT
parser (`src/parser/pczt/ironwood.rs`), table-free exact arithmetic
(`ledger_zcash_crypto/src/hashtocurve.rs`), streamed state. We reuse it as comparison
material and hostile-host test ideas (turn cancellation, duplicate release, late/ambiguous
delivery) — **not** by copying APDU codes, payload bounds, or its secure-element API into
Trezor wire. It proves the streaming-signing shape is viable and audited by a peer vendor,
which is exactly the framing a maintainer wants: "this is how the other hardware wallet did
it, here is how ours differs and why."

**Design principle for our contribution: the opposite of 2472.** Small, layered,
independently-reviewable PRs; published upstream crates wherever possible; the fork deltas
isolated as SHA-pinned, cargo-vet'd, single-purpose commits with a documented exit.

## The dependency order a maintainer would want

Reservation → primitives/deps → trusted read paths → signing → protocol response
settling. Each layer is reviewable and mergeable before the next, and each later layer
depends only on earlier ones.

### Stage 0 — Wire-ID reservation (namespace only)

- **Contents:** the `MessageType` block (32100–32106 + 32109; `reserved 32107, 32108;`)
  + message shapes. No behavior.
- **Why first:** it is a maintainer decision on a shared namespace; stabilizing the
  numbers before feature review prevents on-wire churn. See `WIRE_ID_COORDINATION.md`.
- **Branch/commit:** `ironwood/zcash-streaming-signing` `a341010237`
  "feat(zcash): add provisional protocol and host bindings" (proto + host bindings).
- **Review status:** proto reviewed as part of receive/viewing Fable passes; IDs
  provisional, unassigned. **Blocked-on-Trezor** for final assignment.

### Stage 1 — Fork dependency deltas (SHA-pinned delivery vehicles)

Reviewed and cargo-vet'd *before* the firmware that consumes them, because they are the
supply-chain surface. The forks are delivery vehicles, not source of truth; exit is
upstream merge → plain crates.io pin (see `DEPENDENCY-FORKS.md`).

- **`sinsemilla@6ca88ff`** (v0.1.0 + 1 commit, default-off computed generators).
  Branch `bawolf/sinsemilla ironwood/computed-generators-v0.1.0`, pushed, pin matches.
  Review: Opus 5 adversarial ACCEPT_WITH_FINDINGS (S-1..S-8 dispositioned); **genuine
  Fable still owed**.
- **`orchard@d4792911`** (v0.15.3 + 3 commits). Branch
  `bawolf/orchard ironwood/direct-scope-classifier-v0.15.3`. Commit map:
  - `19f6100` direct scope classifier — Opus 5 reviewed.
  - `5f527f8` Sinsemilla dedup on verify path — **UNREVIEWED**.
  - `d479291` wipe ScopeClassifier ivk cache — **UNREVIEWED**, and this is the pinned SHA.
  Review: the two later commits owe review; pin reachability must be verified (RECON
  flagged unpushed; task says orchard now pushed — confirm).
- **cargo-vet:** delta audit for each fork commit still needs recording in
  `core/embed/supply-chain/config.toml` (that edit is another track's; pointer only here).
- **Upstream shape:** each behavior change is one focused commit starting at the exact
  published-crate source boundary from `.cargo_vcs_info.json`, so it can be offered
  upstream as an isolated patch. This is the anti-2472 move: deps as reviewable
  single-purpose commits, not a bulk patch set.

### Stage 2 — Trusted receive (`ZcashGetAddress`)

- **Contents:** on-device UA derivation + confirmation; allocator-free native receiver.
- **Branch/commits:** `81935a9b8e` (allocator-free receiver derivation), `62b784bae1`
  (native receiver adapter), `ae248e1630` (trusted receive workflow), `849576c01e`
  (harden receive), `75343a5553` (native receive adapter tests), `72de16a243` (SLIP39
  receiver mapping), `e302ab4795` (receiver provenance), `f331c1d736` (portable approval
  core), `3769a23afc` (opt-in Safe 5 build feature).
- **Review status:** Fable 5.1 **ACCEPT-WITH-MUST-FIX** — 1 blocker **R1** (op-timing
  bench reachable from `ZcashGetAddress` pre-consent, not build-gated; also makes index
  `2^88−1` underivable). R1 fix (build-gate the bench/telemetry) is a firmware edit owned
  by another track; **evidence must be re-run** after gating (current RESULT.md predates
  the hook commit `c3244fe65a`). Depends on Stage 0 (proto) only.

### Stage 3 — Viewing-key export (`ZcashGetViewingKey`)

- **Contents:** Orchard-only UFVK export with mandatory privacy consent; never releases
  spend authority.
- **Branch/commits:** `e5c4a6a5a9` (export Orchard viewing authority), `cb4149f7cb`
  (isolate feature-gated qstrs), `cc60d08a0d` (validate viewing key responses).
- **Review status:** Fable 5.1 **ACCEPT**, 0 must-fix (2 should-fix V1/V2). Cleanest
  path; mergeable independently once Stage 0 lands. Depends on Stage 0 only.

### Stage 4 — Streaming PCZT signing (`ZcashSignPczt` + chunk transfer)

- **Contents:** O(1)-RAM streaming signer, Sinsemilla dedup, cross-session `.buf`-rooted
  region, 16/32-action handling. Depends on Stage 0 (proto), Stage 1 (orchard/sinsemilla
  forks), and the receive/viewing primitives.
- **Branch/commits:** `81538b9aca` (streaming Ironwood signing on Safe 5), `c3244fe65a`
  (16/32-action + dedup + cross-session region), `f3f9f374ab` (apply Fable fixes to
  cross-session region).
- **Review status:**
  - Streaming *design note*: real Fable 5.1 review (1 blocker + 7 must-fixes,
    dispositioned).
  - Cross-session region fix: Fable **ACCEPT-WITH-MUST-FIX** (0 blockers for measurement;
    1 production must-fix — freed-block residue persists for the boot; 4 should-fix).
  - Integrated streaming firmware: **genuine Fable adversarial pass still owed.**
  - Remaining feature-freeze items: spend-key scalar zeroization + clarity reviews;
    device session confirming 2-signs-per-boot + Bitcoin heap coexistence.

### Stage 5 — Response contract (settled), then host/desktop

- **Contents:** the response contract is settled (firmware `dd6701f76b`, 2026-09-21):
  detached `ZcashSpendAuthSignatures` records (32109) are the single canonical response;
  the legacy whole-PCZT download pair was deleted and its IDs 32107/32108 are
  reserved/freed, never reused. What remains is the desktop driver.
- **Host side needs NO forks:** construct + prove use published `zcash_primitives` 0.30.1
  (`DeferredPcztBuilder`), `pczt` 0.9.3 Prover, `orchard`; node-free verify via
  `orchard::verify_bundle`. The fork dependency is firmware-only.
- **Branch/commit:** `948f3ad` (desktop-e2e offline pipeline driver, in the copenhagen
  repo). Redaction/retention wiring (host keeps unredacted PCZT to prove+extract a real
  device signature) is known-sound, not yet wired.
- **Review status:** desktop pipeline proven feasible offline; contract-settling and
  wiring in progress.

## Mapping summary

| Stage | Change | Branch / key commits | Depends on | Review status |
|-------|--------|----------------------|------------|---------------|
| 0 | Wire-ID reservation | `a341010237` | — | provisional; blocked-on-Trezor assign |
| 1 | Fork deltas (sinsemilla, orchard) | `bawolf/*` pins `6ca88ff`, `d4792911` | — | Opus-reviewed partial; 2 orchard commits + Fable + cargo-vet owed |
| 2 | Receive | `f331c1d736`…`75343a5553` (+`3769a23afc`) | 0 | Fable ACCEPT-WITH-MUST-FIX (R1) |
| 3 | Viewing export | `e5c4a6a5a9`, `cb4149f7cb`, `cc60d08a0d` | 0 | Fable ACCEPT |
| 4 | Streaming signing | `81538b9aca`, `c3244fe65a`, `f3f9f374ab` | 0,1,2,3 | design-note Fable done; integrated pass owed |
| 5 | Response contract + desktop | firmware `dd6701f76b`; `948f3ad` (copenhagen) | 4 | contract settled (records canonical, 32107/32108 reserved); desktop in progress; host needs no forks |

## How to offer this upstream (when authorized)

1. Open the Stage 0 reservation PR / issue first (parked; needs user authorization —
   `WIRE_ID_COORDINATION.md`). librustzcash `AGENTS.md` also requires a
   maintainer-acknowledged issue before any librustzcash-side PR; none exists yet.
2. Offer the Stage 1 fork deltas upstream as isolated single-purpose commits (their exit
   path), so the firmware can eventually drop the pins for plain crate versions.
3. Land Stage 3 (viewing, cleanest — 0 must-fix) and Stage 2 (receive, after R1 fix +
   evidence re-run) as small feature PRs.
4. Land Stage 4 only after the integrated-signing Fable pass and feature-freeze reviews.
5. Stage 5's response contract is settled, so the reservation ask is the eight live IDs
   with 32107/32108 reserved — nothing further to collapse before maintainers finalize.

Keep every PR to one layer. That is the specific correction to the 2472 shape, and it
matches how the Ledger app was structured and audited.
