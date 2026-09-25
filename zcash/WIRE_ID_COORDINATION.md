# Wire-ID coordination plan — Zcash / Ironwood Trezor message types

Date: 2026-09-19; updated 2026-09-21 after the response-shape collapse, 2026-09-23 for
the move to the 2300 block, and **2026-09-24 after the whole-series review**. Docs-only.
Contains a **draft** PR description that is **NOT to be opened** — parked pending explicit
user authorization. No upstream contact.

> **Current state (2026-09-24, series head in `UPSTREAM_SERIES.md`).** The block is
> **2300–2307, eight contiguous identifiers** in `common/protob/messages.proto`, placed
> after Tron's 2200 block. `ZcashSpendAuthSignatures` moved from 2309 to 2307 and the
> `reserved 2307, 2308;` line is gone: nothing public ever used those two numbers, so
> there is nothing to reserve. The capability is `Capability_Zcash_Shielded = 30`. The
> proto files, `trezorlib.zcash` and the host tests no longer carry a PROVISIONAL banner or
> any "lowest unused block" wording; the request to the maintainers is the PR cover text
> (`UPSTREAM_SERIES.md` §8), which is where the review said it belongs (M3). Sections 2–4
> and the draft below still show the earlier 32100 numbering and the reserved gap; read
> them as history and rebase by the single base offset (§3) before any of it is used.
>
> | ID | Message | Direction |
> |----|---------|-----------|
> | 2300 | `ZcashGetAddress` | in |
> | 2301 | `ZcashAddress` | out |
> | 2302 | `ZcashGetViewingKey` | in |
> | 2303 | `ZcashViewingKey` | out |
> | 2304 | `ZcashSignPczt` | in |
> | 2305 | `ZcashPcztRequest` | out |
> | 2306 | `ZcashPcztAck` | in |
> | 2307 | `ZcashSpendAuthSignatures` | out |
>
> Neither the block nor the value 30 is assigned or reserved upstream; a maintainer may
> move either, and both are a single-base edit on our side followed by regeneration.

## 1. How Trezor message-type IDs get assigned

There is **no external ZIP or registry for Trezor *wire* IDs.** ZIPs govern the Zcash
consensus protocol; they say nothing about Trezor transport enums. Trezor message-type
IDs are assigned by **SatoshiLabs / Trezor maintainers** directly in the
`trezor/trezor-firmware` repository, in `common/protob`:

- `common/protob/messages.proto` holds the single global `MessageType` enum. Every
  message the device can send or receive has one integer here, tagged
  `[(wire_in) = true]` (host→device) and/or `[(wire_out) = true]` (device→host).
- Feature-specific message *shapes* live in a per-feature file
  (`common/protob/messages-zcash.proto`), but the **number** that goes on the wire is
  the `MessageType` entry. That enum is the authority.

Assignment is therefore a maintainer decision on a shared integer namespace, coordinated
by PR review in `common/protob`. It is not something we can self-assign and call final.

## 2. Our current (provisional) block: 32100–32106 plus 32109 (eight live IDs)

`messages-zcash.proto` and `core/src/trezor/enums/MessageType.py` currently carry an
isolated block. `messages.proto` registers them with wire direction:

| ID | Message | Direction | Role |
|----|---------|-----------|------|
| 32100 | `ZcashGetAddress` | wire_in | request UA |
| 32101 | `ZcashAddress` | wire_out | UA response |
| 32102 | `ZcashGetViewingKey` | wire_in | request UFVK |
| 32103 | `ZcashViewingKey` | wire_out | UFVK response |
| 32104 | `ZcashSignPczt` | wire_in | start signing |
| 32105 | `ZcashPcztRequest` | wire_out | device asks for chunk |
| 32106 | `ZcashPcztAck` | wire_in | host supplies chunk |
| 32107 | — | — | **reserved / freed** (see below) |
| 32108 | — | — | **reserved / freed** (see below) |
| 32109 | `ZcashSpendAuthSignatures` | wire_out | detached signature records — the single response |

**32107 and 32108 are not part of the ask.** They carried a legacy whole-PCZT download
pair (`ZcashSignedPczt` out / `ZcashSignedPcztAck` in) that no device handler ever
referenced. The response-shape collapse deleted both messages; the IDs are freed, never
reused, and the firmware retires them in `messages.proto` with the enum's own
protoc-enforced idiom for omitted entries (`reserved 32107, 32108;  // omitted:
whole-PCZT download pair`, matching `reserved 219;  // omitted: StellarInflationOp`).
At HEAD `1ef4f56c4d` the gap is a comment marking them freed; the `reserved` declaration
is the pending clarity-review nit. Either way the host tests pin them absent
(`test_freed_download_ids_stay_unclaimed`).

**The response shape is settled.** `ZcashSpendAuthSignatures` (32109) is the one
canonical device→host response of the signing workflow: `transfer_id` plus `records`,
n × 66-byte `pool u8 | action_index u8 | signature[64]` in ascending action order,
1 ≤ n ≤ 32. Why detached records and not a PCZT download:

- **The host keeps the PCZT.** It already holds the authoritative, unredacted PCZT
  (it built and proved it); it applies each record with the `pczt` crate's
  `apply_orchard_spend_auth_signature`, which verifies the signature against the indexed
  action's `rk` and the host-computed sighash, so a record cannot be replayed into another
  transaction, pool, or action index. The device never returns a PCZT, in whole or in
  chunks, so there is no returned-size bound and no second transfer to audit.
- **It mirrors Ledger's shipped shape.** The Ledger Ironwood app (closest shipped
  analogue) returns detached signatures for the host to insert; Zodl's Ledger consumer
  already applies exactly this shape. It is also Bitcoin parity on Trezor
  (`TxRequest.serialized.signature` per input).
- **O(1) device RAM.** The streaming signer consumes chunks and retains nothing; a
  download path would have required the device to hold or re-derive the whole PCZT.

The proto states, verbatim, that these are "PROVISIONAL local-only identifiers … Not
assigned or reserved upstream … may change once maintainers coordinate an allocation."
The message *names* and the `ZcashViewingKey.key` response field deliberately preserve
the historical Trezor Zcash API; the chunked-transfer shapes preserve the verified
Ironwood donor's behavior without reusing its private identifiers.

## 2a. The feature capability: `Capability_Zcash_Shielded = 30` (provisional)

Firmware `85f748d098` (2026-09-23, pushed on `ironwood/zcash-streaming-signing`) adds
`Capability_Zcash_Shielded = 30;` to `Features.Capability` in
`common/protob/messages-management.proto`, commented (at the time) `PROVISIONAL: 30 is the lowest unused
value, not assigned upstream yet`. `apps/base.py` reports it only when the Ironwood app is
built in (`utils.USE_IRONWOOD`), next to the Miniscript flag it mirrors. trezorlib declares
it on the three Zcash workflows (`@workflow(capability=...)`), and the Connect methods set
`requiredDeviceCapabilities = ['Capability_Zcash_Shielded']`
([CONNECT_METHODS.md](../suite-demo/CONNECT_METHODS.md)). Without it, Connect could tell a
device with the Ironwood app from one without it only by version, which the adversarial
review of the Connect series rejected (M2).

The capability enum, like `MessageType`, is a shared integer namespace in `common/protob`
that the maintainers own. So the coordination is the same:

- **Same status.** Value 30 and the name are self-assigned, the same as wire IDs 2300–2309.
  Neither is final until maintainers assign it.
- **Same path.** It goes in the same reservation PR (§4): one more line in
  `messages-management.proto`. There is no separate process.
- **Same downstream edit.** If maintainers choose another value or name, only the enum line
  changes. The firmware, trezorlib and the regenerated `@trezor/protobuf` refer to it by
  name, so they need regeneration and no code edits.
- **No `bitcoin_only` tag.** Shielded Zcash is not in Bitcoin-only builds. Connect reports
  `Device_MissingCapabilityBtcOnly` for those.

## 3. Why 32100+ is a good proposal to hand maintainers

- **Isolated from the public application ranges.** Trezor's assigned application message
  types cluster far below this; a block at 32100 does not collide with any current
  assignment and leaves the low ranges alone.
- **Compact and self-describing.** Eight IDs in one span (32100–32109) with a
  two-ID `reserved` gap, alternating wire_in/wire_out, one feature, one file. A
  maintainer can accept, or relocate the whole span by editing a single base offset
  (the `reserved` line moves with it), with no interleaving into unrelated features.
  If maintainers prefer a gap-free run, compacting 32109 down to 32107 is the same
  single-base edit; we have no attachment to the hole.
- **Non-colliding by construction.** Because the span is a single run owned entirely by
  the Zcash feature, re-basing it to a maintainer-chosen offset is a pure find/replace
  across `messages.proto`, `messages-zcash.proto`, and `MessageType.py` — no field
  renumbering, no cross-feature edits.

## 4. What a minimal message-ID-reservation PR would contain

Deliberately minimal so it is trivially reviewable and carries no feature code:

1. **`common/protob/messages.proto`** — the eight `MessageType_Zcash*` entries with
   `wire_in`/`wire_out` tags (as above) plus the `reserved 32107, 32108;` line, in one
   block, with a comment marking them Ironwood/Zcash and pointing at the tracking issue.
2. **`common/protob/messages-zcash.proto`** — the message *shapes* (or, if maintainers
   prefer to reserve numbers before shapes, a stub file with only the enum reference and
   a note that shapes land in the feature PR).
3. **`common/protob/messages-management.proto`** — `Capability_Zcash_Shielded` (§2a), so the
   feature flag hosts gate on is assigned with the wire IDs.
4. **Nothing else.** No handlers, no Rust, no Python app code, no `workflow_handlers.py`,
   no translations. The reservation PR reserves numbers and (optionally) declares shapes;
   the behavior lands in the sequenced feature PRs (see `TREZOR_CONTRIBUTION_SHAPING.md`).

The point is to let maintainers assign or relocate the block **before** any feature
review, so the numbers are stable when the feature PRs arrive.

## 5. Does the wire-ID path need anything from the user now?

**Not to keep working.** The provisional block is internally consistent and lets all
local/dev-signed work proceed. The only thing it blocks is a *final* protocol claim.

It needs the user only when we want the numbers made real, and that is one decision:

1. **Authorize maintainer contact** (open the reservation issue/PR). Currently forbidden;
   nothing here contacts upstream.

The former second precondition — settle the response contract so we do not ask
maintainers to reserve numbers for a shape that is going away — is **done**. The
detached-records shape (32109) is canonical, the download pair is deleted, and
32107/32108 are reserved rather than claimed. The draft below asks for exactly the
eight live IDs.

So: no user action required *now* for the wire-ID path itself, beyond noting that when
they choose to pursue upstream assignment they must authorize contact.

---

## Appendix — DRAFT PR description (DO NOT OPEN; parked pending authorization)

> **Title:** protob: reserve Zcash/Ironwood message-type IDs (32100–32106, 32109)
>
> **Summary**
>
> This reserves an isolated block of `MessageType` identifiers for Zcash Ironwood
> shielded support (unified-address retrieval, Orchard viewing-key export, and streaming
> PCZT signing). It assigns *numbers and wire direction only* — no handlers, no device or
> host behavior. Feature code will follow in separate, independently-reviewable PRs once
> the numbers are stable.
>
> **Why a reservation PR**
>
> Trezor message-type IDs are a shared global namespace owned by maintainers in
> `common/protob`. We are carrying these IDs provisionally in a downstream branch and
> would like maintainers to assign (or relocate) the block before the feature PRs land,
> so the on-wire numbers do not churn during feature review.
>
> **What this PR changes**
>
> - `common/protob/messages.proto`: eight `MessageType_Zcash*` entries at 32100–32106
>   and 32109, tagged `wire_in`/`wire_out`, in one block, plus
>   `reserved 32107, 32108;` for two identifiers that a since-removed download pair
>   occupied in our downstream history (never shipped; retired so they are not reused).
> - `common/protob/messages-zcash.proto`: the corresponding message definitions
>   (`proto2`, all fields `required` so an omitted scalar cannot silently select
>   network/account/height/offset/length zero).
>
> | ID | Message | Direction |
> |----|---------|-----------|
> | 32100 | ZcashGetAddress | in |
> | 32101 | ZcashAddress | out |
> | 32102 | ZcashGetViewingKey | in |
> | 32103 | ZcashViewingKey | out |
> | 32104 | ZcashSignPczt | in |
> | 32105 | ZcashPcztRequest | out |
> | 32106 | ZcashPcztAck | in |
> | 32107 | *(reserved — not claimed)* | — |
> | 32108 | *(reserved — not claimed)* | — |
> | 32109 | ZcashSpendAuthSignatures | out |
>
> The signing workflow has exactly one response shape: `ZcashSpendAuthSignatures`
> carries detached spend-authorization signature records (`pool | action_index |
> signature`, one per real spend) that the host applies to its own retained PCZT via the
> `pczt` crate's verifying `apply_orchard_spend_auth_signature`. The device never returns
> a PCZT. This mirrors the shape the Ledger Ironwood app ships and matches Trezor's own
> Bitcoin signer (per-input `TxRequest.serialized.signature`).
>
> **What this PR does NOT change**
>
> No `MessageType.py`/enum consumers, no `workflow_handlers.py`, no app handlers, no Rust,
> no translations, no build wiring. Purely namespace reservation + schema declaration.
>
> **Placement rationale**
>
> The block sits at 32100+, isolated from the assigned application ranges and
> feature-owned, so it can be relocated to any maintainer-preferred offset by editing a
> single base — no interleaving with other features.
>
> **Open coordination questions for maintainers**
>
> 1. Is 32100–32109 an acceptable range, or should the block be based elsewhere?
> 2. Do you prefer reserving numbers now with full message shapes, or numbers first and
>    shapes in the feature PR?
> 3. Do you prefer we keep the `reserved 32107, 32108;` gap (our downstream history) or
>    compact to eight consecutive IDs (32100–32107)? Either is a one-line base change for
>    us; we have no preference.
> 4. Is `Capability_Zcash_Shielded = 30` (the next value after Miniscript's 29) acceptable for the
>    feature flag hosts gate on, or do you prefer another name or value?
>
> **Provenance**
>
> Message names and the `ZcashViewingKey.key` field preserve the historical Trezor Zcash
> API. Chunked-transfer shapes preserve verified Ironwood-donor behavior without reusing
> any private identifiers. No consensus-level (ZIP) change is implied; these are transport
> enums only.
