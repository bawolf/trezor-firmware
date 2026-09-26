//! FF1-AES256 for Orchard diversifiers (radix 2, 88 bits, no tweak), in place
//! of the `fpe` crate that `address_at` would link.

use aes::Aes256;
use aes::cipher::{Block, BlockEncrypt, KeyInit};

const HALF_BITS: usize = 44;
const HALF_MASK: u64 = (1u64 << HALF_BITS) - 1;
// SP 800-38G FF1 parameter block for radix 2, 88 input bits, and no tweak.
const PARAMETER_BLOCK: [u8; 16] = [1, 2, 1, 0, 0, 2, 10, 44, 0, 0, 0, 88, 0, 0, 0, 0];

fn read_half(input: &[u8; 11], start: usize) -> u64 {
    let mut value = 0;
    for bit_index in start..start + HALF_BITS {
        let bit = (input[bit_index / 8] >> (bit_index % 8)) & 1;
        value = (value << 1) | u64::from(bit);
    }
    value
}

fn write_half(output: &mut [u8; 11], start: usize, value: u64) {
    for offset in 0..HALF_BITS {
        let bit = ((value >> (HALF_BITS - 1 - offset)) & 1) as u8;
        let bit_index = start + offset;
        output[bit_index / 8] |= bit << (bit_index % 8);
    }
}

fn encrypt_block(cipher: &Aes256, block: &mut [u8; 16]) {
    cipher.encrypt_block(Block::<Aes256>::from_mut_slice(block));
}

pub(crate) fn encrypt_diversifier_index(key: &[u8; 32], index: [u8; 11]) -> [u8; 11] {
    let cipher = Aes256::new(key.into());
    let mut a = read_half(&index, 0);
    let mut b = read_half(&index, HALF_BITS);
    for round in 0..10u8 {
        let mut r = PARAMETER_BLOCK;
        encrypt_block(&cipher, &mut r);

        let mut q = [0u8; 16];
        q[9] = round;
        q[10..].copy_from_slice(&b.to_be_bytes()[2..]);
        for (state, input) in r.iter_mut().zip(q) {
            *state ^= input;
        }
        encrypt_block(&cipher, &mut r);

        let mut low = [0u8; 8];
        low.copy_from_slice(&r[4..12]);
        let c = a.wrapping_add(u64::from_be_bytes(low)) & HALF_MASK;
        a = b;
        b = c;
    }

    let mut output = [0u8; 11];
    write_half(&mut output, 0, a);
    write_half(&mut output, HALF_BITS, b);
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: [u8; 32] = [
        0x2b, 0x7e, 0x15, 0x16, 0x28, 0xae, 0xd2, 0xa6, 0xab, 0xf7, 0x15, 0x88, 0x09, 0xcf, 0x4f,
        0x3c, 0xef, 0x43, 0x59, 0xd8, 0xd5, 0x80, 0xaa, 0x4f, 0x7f, 0x03, 0x6d, 0x6f, 0x04, 0xfc,
        0x6a, 0x94,
    ];

    // The radix-2 vectors of zcash-test-vectors' `ff1.py`, as in the `fpe`
    // crate's tests.
    #[test]
    fn test_orchard_ff1_vectors() {
        for (plaintext, ciphertext) in [
            (
                [0; 11],
                [
                    0x90, 0xac, 0xee, 0x3f, 0x83, 0xcd, 0xe7, 0xae, 0x56, 0x22, 0xf3,
                ],
            ),
            (
                [
                    0x90, 0xac, 0xee, 0x3f, 0x83, 0xcd, 0xe7, 0xae, 0x56, 0x22, 0xf3,
                ],
                [
                    0x5b, 0x8b, 0xf1, 0x20, 0xf3, 0x9b, 0xab, 0x85, 0x27, 0xea, 0x1b,
                ],
            ),
            (
                [0xaa; 11],
                [
                    0xf0, 0x82, 0xb7, 0xee, 0x8f, 0x29, 0xc0, 0x76, 0x91, 0xce, 0x64,
                ],
            ),
        ] {
            assert_eq!(encrypt_diversifier_index(&KEY, plaintext), ciphertext);
        }
    }
}
