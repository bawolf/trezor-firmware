//! Hedged randomness for RedPallas spend-authorization nonces.
//!
//! reddsa derives the nonce as `T = HStar(random_bytes[80] ‖ pk ‖ msg)` and
//! then `s = T + c*sk` (reddsa 0.5.1 `src/signing_key.rs`). The secret key
//! does **not** enter `T`. So a device whose TRNG is stuck, biased, or
//! glitched into a state the attacker knows hands over `T`, and from one
//! signature `rsk = (s - T)/c`; `alpha` is host-supplied, so `ask = rsk -
//! alpha`. One signature is enough. That is a strictly stronger dependence on
//! the TRNG than any Bitcoin path on this device has: `ecdsa_sign` derives its
//! nonce with RFC 6979 (`crypto/rfc6979.c`, HMAC-DRBG over the private key and
//! the digest) and Ed25519 is deterministic by construction.
//!
//! [`Hedged`] restores the same property without touching reddsa: the bytes it
//! hands to `sign` are
//!
//! ```text
//! block(i) = BLAKE2b-512(personal = "TrezorIrnwdNonce",
//!                        secret ‖ entropy ‖ counter ‖ i)
//! ```
//!
//! where `secret` is device-held and derived from the wallet seed, and
//! `entropy` is a fresh draw from the wrapped RNG. With a healthy TRNG this is
//! as random as before. With a dead one it degrades to a deterministic
//! signature scheme keyed by `secret` -- unpredictable to anyone who does not
//! hold the seed -- and reddsa's own `HStar` still binds `pk` and `msg`, so
//! two different transactions, or two actions of one transaction, never share
//! a nonce.
//!
//! `secret` is a one-way function of the seed rather than `ask` itself because
//! `ask` is not nameable outside the signing call and lives for a shorter time
//! than the session; both are secrets only this device holds, derived from the
//! same seed, so the argument above is unchanged.

use blake2b_simd::Params;
use rand_core::{CryptoRng, Error as RngError, RngCore};
use zeroize::{Zeroize, Zeroizing};

/// BLAKE2b personalization of the hedge. 16 bytes, the maximum.
const PERSONAL: &[u8; 16] = b"TrezorIrnwdNonce";

/// `0x01 ‖ secret[32] ‖ entropy[32] ‖ counter[8] ‖ block_index[4]`.
const INPUT_BYTES: usize = 1 + 32 + 32 + 8 + 4;

/// An RNG whose output no longer depends on entropy alone.
///
/// Wraps the device CSPRNG. Construct it once per signing request with
/// [`Hedged::from_seed`]; every draw mixes a fresh entropy sample with the
/// request's secret and a monotone counter.
pub struct Hedged<R> {
    inner: R,
    secret: [u8; 32],
    counter: u64,
}

impl<R: RngCore> Hedged<R> {
    /// Derives the hedge secret from the wallet seed.
    ///
    /// The seed is borrowed for this call only and is not retained; what the
    /// value keeps is `BLAKE2b-256("TrezorIrnwdNonce", 0x00 ‖ seed)`, from
    /// which the seed cannot be recovered.
    pub fn from_seed(inner: R, seed: &[u8]) -> Self {
        let digest = Params::new()
            .hash_length(32)
            .personal(PERSONAL)
            .to_state()
            .update(&[0x00])
            .update(seed)
            .finalize();
        let mut secret = [0u8; 32];
        secret.copy_from_slice(digest.as_bytes());
        // `blake2b_simd::Hash` is 64 bytes by value and implements no
        // `Zeroize`, so the half this did not take stays on the frame; the
        // caller must wipe the stack after `session_begin`, the only thing
        // that lends the seed.
        Self {
            inner,
            secret,
            counter: 0,
        }
    }
}

impl<R> Drop for Hedged<R> {
    fn drop(&mut self) {
        self.secret.zeroize();
        self.counter.zeroize();
    }
}

impl<R: RngCore> RngCore for Hedged<R> {
    fn next_u32(&mut self) -> u32 {
        let mut bytes = [0; 4];
        self.fill_bytes(&mut bytes);
        u32::from_le_bytes(bytes)
    }

    fn next_u64(&mut self) -> u64 {
        let mut bytes = [0; 8];
        self.fill_bytes(&mut bytes);
        u64::from_le_bytes(bytes)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        // A fresh entropy sample per draw. A failure to produce one is not
        // fatal here the way it is for the session identifier: the hedge is
        // designed to survive exactly that.
        let mut entropy = [0u8; 32];
        self.inner.fill_bytes(&mut entropy);

        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);

        // The hash input is assembled once, in a buffer this function owns and
        // wipes, rather than streamed through `update`: what `update` copies
        // into the hasher's internal block buffer is not reachable to wipe.
        let mut input = Zeroizing::new([0u8; INPUT_BYTES]);
        input[0] = 0x01;
        input[1..33].copy_from_slice(&self.secret);
        input[33..65].copy_from_slice(&entropy);
        input[65..73].copy_from_slice(&counter.to_le_bytes());
        entropy.zeroize();

        let mut written = 0;
        let mut block_index = 0u32;
        while written < destination.len() {
            input[73..77].copy_from_slice(&block_index.to_le_bytes());
            // The output is copied out and wiped too, so the only 64-byte
            // keystream block left after this call is the part the caller
            // asked for.
            let mut block = Zeroizing::new([0u8; 64]);
            block.copy_from_slice(
                Params::new()
                    .hash_length(64)
                    .personal(PERSONAL)
                    .hash(input.as_slice())
                    .as_bytes(),
            );
            let take = core::cmp::min(block.len(), destination.len() - written);
            destination[written..written + take].copy_from_slice(&block[..take]);
            written += take;
            block_index += 1;
        }
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> core::result::Result<(), RngError> {
        self.fill_bytes(destination);
        Ok(())
    }
}

/// The wrapped RNG is the device CSPRNG and the hedge only adds a secret, so
/// the result is at least as suitable for key material as the inner one.
impl<R: CryptoRng + RngCore> CryptoRng for Hedged<R> {}
