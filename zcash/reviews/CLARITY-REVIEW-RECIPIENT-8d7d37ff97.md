# Clarity review: shielded payment shows `user_address` (3223765a80..8d7d37ff97)

- Reviewer model: `claude-opus-5-5[1m]` (Opus 5.5, 1M context). This is **not** a Fable run.
- Scope: readability/clarity only, read-only. `git diff 3223765a80 8d7d37ff97` in
  `trezor-firmware/upstream-series` (`zcash/ironwood-upstream-v2`), plus the head commit message.
  Nothing was built or run.
- Precedent compared against: `core/src/apps/zcash/signer.py:125-146` `output_derive_script`.

Overall: the change is small and follows the upstream precedent (decode the UA, pick the
receiver, show the whole string). Naming is mostly fine. The main problems are prose: one stale
doc statement, a doc sentence that points at a precedent the same document contradicts, a
commit message that says nothing about the new rule, and one factual overstatement.

## Should-fix (ranked)

1. **Stale §6 statement** — `docs/common/zcash-ironwood-signing.md` §6 (around lines 210-216,
   unchanged by this diff): "The handler receives only what is shown: a 43-byte receiver, a value,
   and either the text of a memo ... or its 32-byte digest." The handler now also receives
   `user_address`. Rewrite:
   > The handler receives only what it may show: a 43-byte receiver, the wallet's
   > `user_address` for it (untrusted, checked per §7), a value, and either the text of a memo
   > within the display budget or its 32-byte digest.

2. **The §7 precedent sentence conflicts with §8** — `zcash-ironwood-signing.md:232-234`: "This
   is the rule transparent Zcash payments already follow". A reader of this document takes
   "transparent Zcash payments" to mean the PCZT's transparent outputs, and §8:317 (changed in
   this same diff) says their `user_address` is **ignored**. The precedent is actually the
   `SignTx` path. Rewrite:
   > This follows `apps/zcash/signer.py` (`output_derive_script`), which decodes a unified address
   > entered for a transparent `SignTx` payment, pays its transparent receiver and shows the whole
   > address. A PCZT's transparent outputs do not use it (§8).
   A maintainer will then ask why PCZT transparent outputs don't use the same rule. Either add
   one clause to the §8 row, e.g. "... **ignored**: the device shows the Base58Check address it
   solves from `scriptPubKey`, which is what a transparent recipient enters", or mark it as an
   open question. Right now the asymmetry is unexplained.

3. **Commit message does not describe the change** — `git log -1 8d7d37ff97`: the body says
   "each shielded payment as it arrives (with its memo, ...)" and "Python receives only what is
   shown", but never says which address is shown. Since the fixup was folded into this commit, the
   message should describe it. Add after the first paragraph's payment clause:
   > ... each shielded payment as it arrives, under the wallet's unified address once its Orchard
   > receiver matches the verified one (the Orchard-only address when the wallet sends none), with
   > its memo ...

4. **§7 asserts something the device doesn't check** — `zcash-ironwood-signing.md:229-230`: "its
   other receivers belong to the same recipient and are not paid." The device can't verify that
   the other receivers belong to anyone. Rewrite:
   > its other receivers are neither checked nor paid.

5. **Docstring omits the refusal; coin lookup is done twice** — `core/src/apps/zcash/sign_pczt.py:104-122`.
   The docstring doesn't say the function raises, and `_confirm_output` (:138) already resolves
   `coin` from `coin_name` before `_payment_address` looks it up again. The precedent takes the
   coin object (`self.coin`). Rewrite:
   ```python
   def _payment_address(receiver: bytes, user_address: str | None, coin: CoinInfo) -> str:
       """The address to show for a verified payment receiver (§7): the wallet's
       unified address if its Orchard receiver is `receiver`, the Orchard-only
       address if the wallet sent none. Raises DataError otherwise."""
       from trezor import wire

       from .unified_addresses import Typecode, decode, encode

       if user_address is None:
           return encode({Typecode.ORCHARD: receiver}, coin)
       if decode(user_address, coin).get(Typecode.ORCHARD) != receiver:
           raise wire.DataError("Unified address does not match the Orchard receiver.")
       return user_address
   ```
   Call it as `_payment_address(receiver, user_address, coin)` at :139. The tests pass
   `coininfo.by_name("Zcash")`. The error text follows `signer.py:143` ("Unified address does not
   include a transparent receiver."). The current text, "Zcash recipient does not match its
   address", is vague about which side is wrong.

## Nits

6. **Three Rust comments restate one Python rule, each worded differently.**
   - `stream.rs:196-197`: "only after proving it contains `recipient`"
   - `session.rs:64-65`: "only if it contains `output.receiver`"
   - `signing.rs:100-101`: "checks it against `receiver`"

   "Contains" is imprecise: the handler requires the *Orchard* receiver to equal it, and a UA
   whose Sapling receiver matched would fail. Keep the policy statement in one place, the
   handler boundary `signing.rs:100`:
   > The wallet's untrusted recipient string; the handler shows it only if its Orchard receiver is
   > `receiver` (§7).

   Shorten the other two to `/// The wallet's recipient string, untrusted (§7).` Also,
   neighbouring comments cite "design §7" (`lib.rs:92,100,1138`) while the new ones say "§7".
   Pick one form.

7. **Binding tuple construction** — `core/embed/rust/src/micropython/zcash.rs:288-291`: the inline
   `match` inside `Tuple::alloc(&[...])` breaks from the `memo` pattern just above (:275), which
   binds first and then lists the name. Bind first to match:
   ```rust
   let user_address = match &output.user_address {
       Some(address) => Obj::try_from(address.as_str())?,
       None => Obj::const_none(),
   };
   ```
   Then list `user_address` in the tuple. The payload order is consistent across the Rust docstring
   (`zcash.rs:464`), `trezorzcash.pyi:63`, the unpack at `sign_pczt.py:524` and the tests.

8. **Test names say "wallet string" but the code says `user_address`**, in
   `core/tests/test_apps.zcash.sign_pczt.py:379-415`. Using the field name lets a reviewer grep
   from the doc to the test. Suggested names:
   - `test_no_user_address_shows_the_orchard_address`
   - `test_user_address_with_this_orchard_receiver_is_shown`
   - `test_user_address_for_another_receiver_is_refused`
   - `test_user_address_without_orchard_is_refused`
   - `test_user_address_for_another_network_is_refused`
   - `test_user_address_that_does_not_decode_is_refused`

   The class docstring "What the payment screen shows for a verified receiver (§7)." is fine.

9. **Second stream driver in the Rust test** — `core/embed/ironwood/tests/session_equivalence.rs:1083-1098`
   `confirmed_user_addresses` re-implements the begin/feed loop that `stream_into` (:1060) already
   runs, and `stream_into` now discards `user_address` with `{ output, .. }`. Push `user_address`
   into `Run` (for example `run.confirmed.push((output, user_address))` or a parallel
   `run.user_addresses`) and assert on `run(...)`'s result. That removes the helper. The test name
   `a_payment_confirmation_carries_the_wallet_string_for_the_handler_to_check` could be
   `a_payment_confirmation_carries_its_user_address`, since the check is not tested here.

10. **The chunkify comment could be simpler.** `sign_pczt.py:144-145` says "a unified address of
    106 or more characters". Since the string is now wallet-supplied, "A unified address (106+
    characters) is not comparable unbroken on a consent screen." says the same thing. The
    transparent comment at :192 ("35 characters, not 106") still reads correctly.

## Checked and fine
- The name `user_address` matches the PCZT field, which keeps it traceable to the wire. Keep it.
- The name `_payment_address` is clear enough. It mirrors `_transparent_address`.
- `stream.rs:373-378` `Reader::user_address` doc: accurate after dropping "not used".
- `stream.rs:440-444` `transparent_output` doc and the §8 table row (:317) agree with each other.
- `scratch_tier.rs` `with_longest_user_addresses` doc and the new "32 actions, longest addresses"
  case: clear. The label width change 26 to 30 is needed for the longer label.
- The `.pyi` docstring is identical to the Rust `///` source, as the generator requires.
