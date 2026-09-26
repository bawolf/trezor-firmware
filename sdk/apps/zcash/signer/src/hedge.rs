//! Hedged randomness for RedPallas nonces.
//!
//! reddsa derives the nonce from random bytes, the public key and the message,
//! not from the secret key, so a predictable RNG reveals the key from one
//! signature. [`Hedged`] mixes a device secret into every draw:
//!
//! ```text
//! block(i) = BLAKE2b-512("TrezorZcashNonce", 0x01 || secret || entropy || counter || i)
//! ```
//!
//! With a failed RNG this is still a deterministic scheme keyed by the secret.

use blake2b_simd::Params;
use rand_core::{CryptoRng, Error as RngError, RngCore};
use zeroize::{Zeroize, Zeroizing};

/// BLAKE2b personalization of the hedge. 16 bytes, the maximum.
const PERSONAL: &[u8; 16] = b"TrezorZcashNonce";

/// `0x01 || secret[32] || entropy[32] || counter[8] || block_index[4]`.
const INPUT_BYTES: usize = 1 + 32 + 32 + 8 + 4;

/// An RNG that mixes a device secret into every draw.
pub struct Hedged<R> {
    inner: R,
    secret: [u8; 32],
    counter: u64,
}

impl<R: RngCore> Hedged<R> {
    /// Wraps `inner`, keyed by `BLAKE2b-256("TrezorZcashNonce", 0x00 || key)`
    /// of a device secret such as the account spending key.
    pub fn new(inner: R, key: &[u8]) -> Self {
        let digest = Params::new()
            .hash_length(32)
            .personal(PERSONAL)
            .to_state()
            .update(&[0x00])
            .update(key)
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
        let mut entropy = [0u8; 32];
        self.inner.fill_bytes(&mut entropy);

        let counter = self.counter;
        self.counter = self.counter.wrapping_add(1);

        // One wiped buffer rather than `update` calls, whose copies in the
        // hasher cannot be wiped.
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

impl<R: CryptoRng + RngCore> CryptoRng for Hedged<R> {}

#[cfg(test)]
mod tests {
    use super::*;

    /// An RNG that has failed: it returns zeros.
    struct Dead;

    impl RngCore for Dead {
        fn next_u32(&mut self) -> u32 {
            0
        }

        fn next_u64(&mut self) -> u64 {
            0
        }

        fn fill_bytes(&mut self, destination: &mut [u8]) {
            destination.fill(0);
        }

        fn try_fill_bytes(&mut self, destination: &mut [u8]) -> core::result::Result<(), RngError> {
            destination.fill(0);
            Ok(())
        }
    }

    fn draw(rng: &mut impl RngCore, len: usize) -> Vec<u8> {
        let mut bytes = vec![0; len];
        rng.fill_bytes(&mut bytes);
        bytes
    }

    #[test]
    fn test_matches_the_construction() {
        let key = [7; 32];
        let secret = Params::new()
            .hash_length(32)
            .personal(PERSONAL)
            .to_state()
            .update(&[0x00])
            .update(&key)
            .finalize();
        let block = |counter: u64, index: u32| {
            Params::new()
                .hash_length(64)
                .personal(PERSONAL)
                .to_state()
                .update(&[0x01])
                .update(secret.as_bytes())
                .update(&[0; 32])
                .update(&counter.to_le_bytes())
                .update(&index.to_le_bytes())
                .finalize()
        };
        let mut hedged = Hedged::new(Dead, &key);
        let first = draw(&mut hedged, 80);
        assert_eq!(&first[..64], block(0, 0).as_bytes());
        assert_eq!(&first[64..], &block(0, 1).as_bytes()[..16]);
        assert_eq!(draw(&mut hedged, 64), block(1, 0).as_bytes());
    }

    #[test]
    fn test_dead_rng_is_keyed_and_never_repeats() {
        let mut a = Hedged::new(Dead, &[1; 32]);
        let mut b = Hedged::new(Dead, &[2; 32]);
        let a1 = draw(&mut a, 80);
        assert_ne!(a1, draw(&mut b, 80));
        assert_ne!(a1, draw(&mut a, 80));
        assert_eq!(a1, draw(&mut Hedged::new(Dead, &[1; 32]), 80));
    }

    #[test]
    fn test_entropy_is_mixed_in() {
        use rand_chacha::ChaCha20Rng;
        use rand_core::SeedableRng;

        let mut dead = Hedged::new(Dead, &[1; 32]);
        let mut live = Hedged::new(ChaCha20Rng::seed_from_u64(1), &[1; 32]);
        assert_ne!(draw(&mut dead, 32), draw(&mut live, 32));
    }
}
