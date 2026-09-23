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
| Header | magic, version/group/branch/lock_time/expiry/coin_type, `tx_modifiable`, the global proprietary count, and the transparent bundle's tag with its input and output counts | `Header`, the transparent output count |
| Transparent output `j` | value, the length-prefixed `scriptPubKey`, the absent `redeem_script`, the empty `bip32_derivation`, the bounded and ignored `user_address`, the empty proprietary map | one projection row (20-byte hash, kind, value) |
| Shielded prefix | an optional but necessarily empty Sapling bundle, the absent Orchard tag, the Ironwood bundle tag and action count | `count` |
| Action `i` | the action grammar, about 2 KB | one `ActionRecord` (§4) |
| Trailer | flags, value sum (magnitude and sign), the optional anchor, the absent proof, the optional and ignored `bsk`, end of input | nothing new |

The transparent bundle sits between the global fields and the shielded pools
in the v2 encoding, which is why the header stops at its output count and the
Ironwood action count arrives later, with the shielded prefix.

Per-section byte budgets follow from the grammar (`stream.rs`
`HEADER_BUDGET` = 101, `TRANSPARENT_OUTPUT_BUDGET` = 589, `SHIELDED_BUDGET` =
109, `ACTION_BUDGET` = 2015, `TRAILER_BUDGET` = 89); the largest is the
scanner's single buffer, and it is still the action. A section that does not
complete inside its budget is `Malformed` before any cryptography runs.

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
height, coin type, lock time zero, the expiry window. The Orchard pool tag
must be absent. A Sapling bundle may be present only if it is empty (no
spends, no outputs, zero value sum); anything else is `Policy`. The
transparent bundle may be absent, or present under the rules of §13.

**Transparent output `j`.** Each admitted output is fed to the ZIP-244 T.2c
outputs hash (§5), added to the transparent total, and shown now with its
`t1…`/`t3…` (`tm…`/`t2…`) address. Like a shielded payment confirmation this
is **not consent**. The per-transaction privacy warning precedes the first
one (§13).

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
reviewed output (shielded **or** transparent), then the accounting identity:

```
fee              = input_total − shielded_output_total − transparent_total
value_sum        = fee + transparent_total
input_total      = payment_total + change_total + transparent_total + fee
```

The transparent outputs leave the shielded pool through the Ironwood value
sum, so they are spent value and not fee. Dropping either subtraction
mis-states the fee by the whole transparent amount without failing any other
check, which is why the identity is written out here and has its own
conformance case. Then the fee cap.

**VerifyDummies.** Each dummy record's `rk.verify(sighash, signature)`. Only
then is the consent token derived (§6).

**Totals.** The amount leaving the wallet (shielded payments + transparent
outputs + fee), the fee, the digest-bound expiry height, network and account,
the count of confirmed outputs, and -- when any output is transparent -- the
public amount. The host reference height is a policy input and is not shown.
Approval consumes the token.

**Sign.** Take the pending slot before any check; require
`SpendValidatingKey::from(ask) == expected_ak`; for each real record require
`rk == ak.randomize(alpha)`, then sign. The records are released only when
every real spend has signed. Consent is consumed on every signing attempt,
including failures.

## 5. Digest

The device computes the ZIP-244/ZIP-229 V6 signature digest incrementally:
the header digest, the Sapling and Orchard-V6 nodes as their fixed empty
digests, and the Ironwood node as compact ‖ memo ‖ noncompact ‖ flags ‖ value
balance.

The transparent node is the fixed empty digest until a transparent output is
fed. With no transparent **inputs** -- which §13 makes structural -- ZIP-244
§S.2 says `transparent_sig_digest` is identical to the txid node §T.2, so the
node is

```
BLAKE2b-256("ZTxIdTranspaHash",
    BLAKE2b-256("ZTxIdPrevoutHash", "") ‖
    BLAKE2b-256("ZTxIdSequencHash", "") ‖
    BLAKE2b-256("ZTxIdOutputsHash", ‖_j LE64(value_j) ‖ CompactSize(|script_j|) ‖ script_j))
```

-- one extra running BLAKE2b state, no `hash_type` byte, no amounts/scripts
sub-hashes, no per-input pass. The fed bytes per output are exactly what
`TxOut::write` serializes. The anchor is excluded from the sighash and belongs
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
totals and fee afterwards, as Bitcoin does. Transparent outputs come first,
because the encoding puts them first, and the output numbering runs across
both halves. Those per-output confirmations are not consent (§4).

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

## 13. Transparent outputs

The device signs a **deshield**: a shielded-funded transaction that pays one
or more transparent addresses. It signs no transparent **input**, and holds no
transparent key.

**Admission.** The transparent bundle is absent, or present with:

- **zero inputs.** An input is `Policy`. This is not a parse convenience: zero
  transparent inputs is what makes ZIP-244 §S.2 collapse to §T.2 (§5), and the
  device has no BIP-44/secp256k1 keychain and no script solver with which to
  authorize a transparent spend.
- **at least one output.** The v2 encoder writes an empty bundle as an absent
  tag, so a present-and-empty bundle would be a second encoding of the same
  transaction: `Policy`.
- **at most `MAX_ACTIONS − Ironwood actions` outputs.** ZIP-317 counts a
  standard 34-byte transparent output as one logical action, in the same total
  as the shielded actions, so the device charges it against the same cap of 32
  rather than inventing a second one: `Capacity` otherwise. At least one
  Ironwood action is always required, so no more than 31 transparent outputs
  are ever admitted.

Per output:

| Field | Rule |
|---|---|
| `value` | required, at most `MAX_MONEY` (`Malformed` above it); displayed; enters the accounting |
| `script_pubkey` | exactly the 25-byte P2PKH `76 a9 14 <hash160> 88 ac` or the 23-byte P2SH `a9 14 <hash160> 87`; anything else is `Policy` |
| `redeem_script` | must be absent (`Policy`) |
| `bip32_derivation` | must be empty (`Policy`) |
| `user_address` | admitted within `USER_ADDRESS_BUDGET`, required to be UTF-8, and **ignored** -- the same rule the Ironwood output has |
| `proprietary` | must be empty (`Policy`) |

Only the two standard script shapes are admitted because an unrecognised
script is an address the device cannot render, and an address it cannot render
is one it cannot obtain consent for. P2SH is admitted, not refused: a payment
to a script is a payment a user can recognise as a `t3…`/`t2…` address, and
exchanges use them.

**Display.** Each transparent output is shown with its Base58Check address --
`t1…`/`t3…` on mainnet, `tm…`/`t2…` on testnet, from `coininfo`'s existing
7352/7357 and 7461/7354 version bytes and the base58check encoder Bitcoin
signing already uses -- chunked in fours like every other address the device
asks a user to compare. The 20-byte hash is solved out of the very
`scriptPubKey` that is fed to the digest, so what is shown is what the
signature covers.

**The privacy warning.** Exactly one screen per transaction, before the first
transparent output, under its own ButtonRequest name
(`zcash_transparent_payment`, `ButtonRequestType.Warning`) so a host can
neither suppress it nor mistake it for an output confirmation. It is per
transaction and not per output because a deshield with several recipients is
still one decision about privacy. The totals screen then repeats the public
amount, because the warning came before any amount did.

**Change.** There is none: the device owns no transparent key, so no
transparent output can be its own change and every one of them is a payment,
shown in full.
