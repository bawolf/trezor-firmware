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
use zeroize::Zeroize;

/// BLAKE2b personalization of the hedge. 16 bytes, the maximum.
const PERSONAL: &[u8; 16] = b"TrezorIrnwdNonce";

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

        let mut written = 0;
        let mut block_index = 0u32;
        while written < destination.len() {
            let digest = Params::new()
                .hash_length(64)
                .personal(PERSONAL)
                .to_state()
                .update(&[0x01])
                .update(&self.secret)
                .update(&entropy)
                .update(&counter.to_le_bytes())
                .update(&block_index.to_le_bytes())
                .finalize();
            let block = digest.as_bytes();
            let take = core::cmp::min(block.len(), destination.len() - written);
            destination[written..written + take].copy_from_slice(&block[..take]);
            written += take;
            block_index += 1;
        }
        entropy.zeroize();
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> core::result::Result<(), RngError> {
        self.fill_bytes(destination);
        Ok(())
    }
}

/// The wrapped RNG is the device CSPRNG and the hedge only adds a secret, so
/// the result is at least as suitable for key material as the inner one.
impl<R: CryptoRng + RngCore> CryptoRng for Hedged<R> {}
