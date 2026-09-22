# Zcash Ironwood streaming signing

The Zcash Ironwood app signs a PCZT the device never holds in full: the host
uploads it in 1,024-byte chunks, an incremental scanner consumes the v2
encoding section by section, and the device returns detached spend
authorization signatures. This document records the rules the implementation
enforces, so that the comments in `core/embed/ironwood`,
`core/embed/rust/src/ironwood_*.rs` and `core/src/apps/zcash` can cite a rule
by number instead of restating it.

Section numbers are stable and are what the source cites as "§N". Sections 1,
2, 8, 9 and 12 of the originating design note cover goals, prior art, flash
estimates, the conformance strategy and known limits; they are not
normative for the code and are not reproduced here.

Everything below is enforced on the device. The host is untrusted: every
field is validated against the declared transfer before it is used, and any
violation ends the workflow with one error class and no diagnostic string.

## 3. Wire design

The host pulls are `ZcashPcztRequest { transfer_id, offset, length }` /
`ZcashPcztAck`, advancing contiguously from offset zero to the declared
`ZcashSignPczt.pczt_length` with no gaps, duplicates, sparse writes, retry or
resume. The transfer id is checked on every chunk.

| Stage | Device consumes | Device keeps afterwards |
|---|---|---|
| Header | magic, version/group/branch/lock_time/expiry/coin_type, the absent transparent and Orchard pool tags, an optional but necessarily empty Sapling bundle, the Ironwood bundle tag and action count | `Header`, `count` |
| Action `i` | the action grammar, about 2 KB | one `ActionRecord` (§4) |
| Trailer | flags, value sum (magnitude and sign), the optional anchor, the absent proof, the optional and ignored `bsk`, end of input | nothing new |

Per-section byte budgets follow from the grammar (`stream.rs`
`HEADER_BUDGET`, `ACTION_BUDGET`, `TRAILER_BUDGET`); the largest is the
scanner's single buffer. A section that does not complete inside its budget
is `Malformed` before any cryptography runs.

The response is `ZcashSpendAuthSignatures { transfer_id, records }`, each
record `pool u8 (Ironwood = 0x03) ‖ action_index u8 ‖ signature [u8; 64]`, in
ascending index order, one per real spend, sent only after every signature
exists. The device never sends a PCZT back, in whole or in chunks; the host
applies the records to its own copy with the `pczt` crate's
`apply_orchard_spend_auth_signature`, which re-verifies each signature
against the indexed action's `rk` and the host-computed sighash.

## 4. Device state machine

`Init → Header → Action(i) → Trailer → VerifyDummies → Totals → Approved →
Sign → Finished`, with `Rejected`/`Cancelled` terminal. Any error resets the
machine, zeroizes the records and returns one error class. The approval
counter increments on `ZcashSignPczt` receipt, before any parsing; a second
`ZcashSignPczt` or any mid-stream error clears the pending consent slot.

**Init.** Validate network, account, `host_reference_height` and
`pczt_length`; derive the session FVK and `expected_ak` from the seed, which
is borrowed for that call only.

**Header.** Version and version group, the branch derived from network and
height, coin type, lock time zero, the expiry window. The transparent and
Orchard pool tags must be absent. A Sapling bundle may be present only if it
is empty (no spends, no outputs, zero value sum); anything else is `Policy`.

**Action `i`, in order:**

1. Scan the action's fields into the scanner's fixed buffer.
2. Field validity exactly as the `orchard` PCZT parser performs it: canonical
   points and scalars, `note_version == V3` for the output, values within
   `MAX_MONEY`.
3. Real spend (`value > 0`): the wire `fvk` must equal the session FVK
   (constant-time), and the spend-auth signature must be absent. Record
   `alpha` and `rk`.
4. Dummy spend (`value == 0`): the signature must be present. Record `rk` and
   the signature for `VerifyDummies`.
5. A `zip32_derivation` on the spend or the output is admitted only when it
   names the device's own seed fingerprint (§11) *and* the consented account
   path `m/32'/coin_type'/account'`; an absent claim is admitted, anything
   else is `Policy`, and a non-hardened index is `Malformed`. The path is
   derived inside the signing core from the consented request, so the host
   supplies only bytes to compare.
6. `verify_cv_net`, nullifier ownership against the session FVK, `verify_rk`,
   `verify_note_commitment`, and the duplicate-nullifier check against the
   retained records.
7. Update the running Ironwood hashers and the checked value accounting
   (§5).
8. Classify the output's receiver against the session FVK's own external and
   internal addresses, then `verify_encryption` with the memo and OVK policy
   of §11.
9. Value-bearing, non-change output: show address, value and any memo now.
   **This confirmation is not consent**: no token exists yet and a later
   rejection discards everything. Change is counted and hidden; zero-value
   padding is counted and hidden.

**Trailer.** The Ironwood-V3 default flags (unexpected bits rejected), the
value sum, the optional anchor when present, `bsk` ignored, end of input.
Then the bundle-level checks: at least one real spend and at least one
reviewed output, fee = inputs − outputs, value balance equals the fee, the
fee cap, conservation.

**VerifyDummies.** Each dummy record's `rk.verify(sighash, signature)`. Only
then is the consent token derived (§6).

**Totals.** The amount leaving the wallet (payments + fee), the fee, the
digest-bound expiry height, network and account, and the count of payment
outputs confirmed. The host reference height is a policy input and is not
shown. Approval consumes the token.

**Sign.** Take the pending slot before any check; require
`SpendValidatingKey::from(ask) == expected_ak`; for each real record require
`rk == ak.randomize(alpha)`, then sign. The records are released only when
every real spend has signed. Consent is consumed on every signing attempt,
including failures.

## 5. Digest

The device computes the ZIP-244/ZIP-229 V6 signature digest incrementally:
the header digest, the transparent, Sapling and Orchard-V6 nodes as their
fixed empty digests, and the Ironwood node as compact ‖ memo ‖ noncompact ‖
flags ‖ value balance. The anchor is excluded from the sighash and belongs
only to the authorizing-data digest. Every sub-digest is pinned
byte-for-byte against the whole-transaction implementation by the
differential tests in `core/embed/ironwood/tests`.

## 6. Consent token

The token commits to the session, the approval counter, the policy, the FVK,
the sighash, the declared length and every byte the device consumed. The byte
commitment is a running BLAKE2b-256 under its own personalization
(`IWStreamBytesV1`) with the declared length and the action count folded in,
which is what makes it a commitment to the *bytes reviewed* and not merely to
the sighash: a different anchor or lock-time encoding that produces the same
sighash is a different token and needs new consent.

## 7. Resource model

The signing core is `no_std`. Everything it allocates comes from one
boot-lifetime region owned by the handler, never from the MicroPython GC
heap, because the collector does not scan Rust statics. Retained across a
session: the header, one record per action, the live BLAKE2b states, the
value accumulators, the token and pending slot, the session FVK and
`expected_ak`. Peak transient: the scanner's single section buffer, one
parsed action, one note plaintext, and hash-to-curve scratch. The Pasta
square-root table is resident for the whole boot and is warmed at prewarm so
it cannot fragment the region mid-action.

## 10. Handler and UI

`core/src/apps/zcash/sign_pczt.py` drives the native session on the Bitcoin
signer's generator pattern and renders every screen through the
model-agnostic `trezor.ui.layouts` facade, so caesar, delizia and eckhart
serve the same flow with no app-level branching. The handler receives only
what is shown: a 43-byte receiver, a value, a change flag, and either the
text of a memo within the display budget or its 32-byte digest. The full
memo bytes never cross into Python.

## 11. Decided display and admission policy

**Per-output review.** Each payment output is confirmed as it arrives, with
totals and fee afterwards, as Bitcoin does. Those per-output confirmations
are not consent (§4).

**Memos.** Nonempty memos are displayed within a byte budget.

- A memo of a hidden output must be empty: `Policy` otherwise. "Empty" is
  the canonical `0xF6` marker for change; zero-value padding outputs also
  admit an all-zero memo (the empty text memo).
- On a payment, the canonical `0xF6` marker or an all-zero memo shows no
  memo screen.
- A ZIP-302 text memo -- leading byte `<= 0xF4`, valid UTF-8, no NUL, at most
  `MEMO_TEXT_BUDGET` = 256 bytes after NUL trimming -- is shown verbatim,
  provided every character is one the device draws as itself. Characters
  outside the BMP are excluded because the glyph lookup truncates the code
  point to `u16` and would draw a different character; so are controls other
  than `\n`, bidi and zero-width format characters, and NBSP, each of which
  would make the screen disagree with the signed bytes.
- Everything else -- arbitrary `0xFF` data, a reserved leading byte, invalid
  UTF-8, an interior NUL, over the budget, or a character from the list above
  -- is shown as the unkeyed BLAKE2b-256 of all 512 memo bytes, rendered as
  hex. A wallet can recompute that value from the memo it built.

The memo shown is the memo signed by construction: it is recovered from the
same `enc_ciphertext` the sighash commits to, and that ciphertext is hashed
into the digest before the memo is classified.

**Outgoing viewing keys.** A payment output must be recoverable under the
external OVK. A change output -- one whose receiver the device's own
classifier places among this FVK's internal addresses -- may carry no OVK,
which is what the standard SDK's default `OvkPolicy::Sender` produces; a
change output that the *external* OVK recovers is the wrong scope and is
refused. Nothing about what the signature commits to depends on the OVK: the
note is bound by its commitment and by the `pk_d`/`esk` recovery from the
digest-committed ciphertext.

**Seed fingerprint.** The ZIP-32 seed fingerprint,
`BLAKE2b-256("Zcash_HD_Seed_FP", I2LEOSP_8(len) ‖ seed)`, is the value a
`zip32_derivation` carries and the value §4 step 5 compares against. It is a
public identifier of the *seed*: the same for every account index and for
both networks. It is therefore exported only when
`ZcashGetViewingKey.include_seed_fingerprint` is set, behind a second
warning that says what it links, and a host that does not need it should not
ask -- the SDK accepts an account with no derivation at all, and the device
admits a PCZT with no claim.

For a wallet restored from a SLIP-39 backup the device derives from a
16-byte secret, below ZIP 32's 32-byte minimum. ZIP 32 defines no
fingerprint that short and the `zip32` crate refuses one, so the device
applies the identical construction with length byte 16. This is a
Trezor-only extension: for such a wallet the exported value is
authoritative and a host must store it rather than recompute it. For every
seed length ZIP 32 admits (32..252 bytes) the value is byte-for-byte the
canonical one.
