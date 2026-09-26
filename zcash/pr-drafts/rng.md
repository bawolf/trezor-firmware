# Let extapps use rng_fill_buffer

Status: note for the owner of `bieleluk/sdk-wip` (#7516), not a PR.
`trezor_api_v1_t` and the applet syscall whitelist are on `main`, but the SDK
that uses them exists only on the draft SDK branches. Needs an owner decision
on API versioning (below).
Commit 1 alone fixes code that is identical on `main` and cherry-picks cleanly
there; it could be a small PR against `main` for cepetr/TychoVrahe. Whether an
unaligned buffer reaches it on `main` today was not checked.
Kind: bug fix (commit 1) and feature (commit 2).
Reproduction status: none; commit 1 is from code reading.

Branch: https://github.com/bawolf/trezor-firmware/tree/extapp/rng @ `ed9f103db3`
(two commits on `bieleluk/sdk-wip` @ `4cd93ff4d8`)

---

An app has no entropy source: `trezor_api_v1_t` carries no RNG and the applet
syscall whitelist does not admit one, so an app cannot draw fresh randomness,
for example for signature nonces.

- `ec7ffaaa76` fix(core): let rng_fill_buffer fill unaligned buffers
- `ed9f103db3` feat(core,sdk): allow extapps to use rng_fill_buffer

**Cause (commit 1).** The STM32 `rng_fill_buffer` stored whole words through a
`uint32_t*` cast of the destination, which is undefined for a buffer that is
not 4-byte aligned. An app can pass any byte slice through the syscall.

**Fix.**
- Commit 1: each word is copied into place with `memcpy`.
- Commit 2: `trezor_api_v1_t` gains `rng_fill_buffer`, appended so the other
  field offsets stay put. The kernel admits `SYSCALL_RNG_FILL_BUFFER` from
  applets; its verifier already checks that the buffer is the caller's
  writable memory. The SDK exposes `crypto::random_bytes(&mut [u8])`; the test
  mock fills zeros.
- Versioning: v1 is unreleased and has been changed in place before
  (`f7fe40f57b` on sdk-wip), and `app_header.c` accepts only `abi_version == 1`, so this
  appends to v1. Once v1 is frozen, this belongs in a v2 so that a new app
  fails closed on old firmware.

**Tests.** SDK checks pass; the T3T1 and T3W1 hardware builds compile the
STM32 path; an app's hedged signature nonces use it on T3W1 and T3T1
emulators. Nothing on the host exercises an unaligned STM32 write.

### Notes for QA
Apps can call `crypto::random_bytes`. Existing apps are unaffected.
