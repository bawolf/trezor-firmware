//! Measurement-only micro-benchmarks for the per-action verification cost.
//!
//! Per-action verification dominates the signing latency, and that phase is
//! built from two candidate costs: Sinsemilla hashing (under the *computed*
//! generators, ~100x slower than the table) and Pallas variable-base scalar
//! multiplication. These benches time each operation in isolation, using
//! the *same* `ironwood-sinsemilla` (computed-generators) and
//! `ironwood-pasta-curves` instances the signing path links, so the next
//! hardware run can attribute the cost and pick the latency fix.
//!
//! Each entry runs `iters` iterations of one operation and returns a folded
//! accumulator of every result. The caller times the call with
//! `utime.ticks_ms` on the Python side; returning (and black-boxing) the
//! accumulator keeps the optimizer from hoisting or eliding the work. The
//! bench allocates (Sinsemilla pads into a `Vec<bool>`, Pasta builds its
//! square-root table), so the caller installs a region first, exactly like the
//! signing path.
//!
//! This module changes no signing behavior and is reached only through the
//! reserved all-`0xff` diversifier index in `get_address`.

use alloc::vec::Vec;
use core::hint::black_box;

use ironwood_pasta_curves::group::ff::{Field, PrimeField};
use ironwood_pasta_curves::group::{Group, GroupEncoding};
use ironwood_pasta_curves::pallas;
use ironwood_sinsemilla::{CommitDomain, HashDomain};

// ZIP-224/225 domain personalizations, copied verbatim from
// `orchard::constants::fixed_bases` (crate-private there). `CommitDomain::new`
// derives the M hash domain as "{personalization}-M", so the hash-only bench
// below hashes over the matching "-M" domain to isolate the blinding term.
const NOTE_COMMITMENT_PERSONALIZATION: &str = "z.cash:Orchard-NoteCommit";
const NOTE_COMMITMENT_M_PERSONALIZATION: &str = "z.cash:Orchard-NoteCommit-M";
const COMMIT_IVK_PERSONALIZATION: &str = "z.cash:Orchard-CommitIvk";

// NoteCommit message length: g_d(256) + pk_d(256) + v(64) + rho(255) + psi(255)
// = 1086 bits (orchard::note::commitment::NoteCommitment::derive).
const NOTE_COMMIT_MSG_BITS: usize = 1086;
// CommitIvk message length: ak(255) + nk(255) = 510 bits
// (orchard::spec::commit_ivk).
const COMMIT_IVK_MSG_BITS: usize = 510;

/// A deterministic, seed-varying bit message so each iteration hashes distinct
/// input (defeats constant-folding) while staying allocation-representative of
/// the real Sinsemilla message a note commitment absorbs.
fn message(bits: usize, seed: u64) -> Vec<bool> {
    let mut state = seed | 1;
    let mut out = Vec::with_capacity(bits);
    for _ in 0..bits {
        // xorshift64; the low bit is the message bit.
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.push((state & 1) == 1);
    }
    out
}

/// Fold a curve point into the running accumulator so the result is observed.
fn absorb_point(acc: &mut u64, point: pallas::Point) {
    let bytes = point.to_bytes();
    for byte in bytes {
        *acc = acc.rotate_left(1).wrapping_add(byte as u64);
    }
}

/// Fold a base-field element into the running accumulator.
fn absorb_base(acc: &mut u64, value: pallas::Base) {
    let bytes = value.to_repr();
    for byte in bytes {
        *acc = acc.rotate_left(1).wrapping_add(byte as u64);
    }
}

/// Force one-time initialisation (Pasta square-root table, domain generator
/// derivation under computed generators) so it is not billed to the first timed
/// op. Runs one of every op at a single iteration.
///
/// Measurement-only: this is the selector-0 warm step of the `get_address`
/// bench hook. The production signing path uses [`crate::prewarm`] instead,
/// which roots only the Pasta square-root table and does no
/// Sinsemilla work.
pub fn warmup() {
    // Building the domains derives Q and R via `hash_to_curve`, which builds
    // Pasta's lazy square-root table inside the installed region.
    black_box(note_commitment(1));
    black_box(sinsemilla_hash(1));
    black_box(scalar_mul(1));
    black_box(commit_ivk(1));
}

/// N iterations of the Orchard note commitment as `verify_note_commitment`
/// computes it: `sinsemilla::CommitDomain::commit` over the NoteCommit domain
/// with a fresh blinding scalar. This is the exact call
/// `orchard::note::commitment::NoteCommitment::derive` makes, so it includes
/// BOTH the Sinsemilla hash (`hash_to_point` over the ~1088-bit message) AND
/// the blinding `R * r` fixed-base scalar multiplication. This is the dominant
/// per-action operation.
pub fn note_commitment(iters: u32) -> u64 {
    let domain = CommitDomain::new(NOTE_COMMITMENT_PERSONALIZATION);
    let mut acc = 0u64;
    for i in 0..iters {
        let msg = message(NOTE_COMMIT_MSG_BITS, 0xC0_FFEE ^ i as u64);
        let r = pallas::Scalar::from(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1));
        let point = Option::from(domain.commit(black_box(msg).into_iter(), black_box(&r)))
            .unwrap_or_else(pallas::Point::identity);
        absorb_point(&mut acc, point);
    }
    black_box(acc)
}

/// N iterations of *just* the Sinsemilla hash over the same ~1088-bit message
/// and the same "-M" domain the note-commitment `CommitDomain` builds
/// internally (`HashDomain::hash_to_point`, no blinding). Subtracting this from
/// `note_commitment` isolates the blinding `R * r` scalar multiplication.
pub fn sinsemilla_hash(iters: u32) -> u64 {
    let domain = HashDomain::new(NOTE_COMMITMENT_M_PERSONALIZATION);
    let mut acc = 0u64;
    for i in 0..iters {
        let msg = message(NOTE_COMMIT_MSG_BITS, 0xC0_FFEE ^ i as u64);
        let point = Option::from(domain.hash_to_point(black_box(msg).into_iter()))
            .unwrap_or_else(pallas::Point::identity);
        absorb_point(&mut acc, point);
    }
    black_box(acc)
}

/// N iterations of a Pallas variable-base scalar multiplication
/// `pallas::Point * pallas::Scalar` — the operation cv_net / rk / nullifier /
/// address-derivation use repeatedly. A fresh non-identity base (the running
/// accumulator point) and a fresh full-width scalar each iteration prevent
/// constant-folding; Pasta's `Mul` is constant-time (255 doublings) regardless
/// of the scalar value, so the timing is representative.
pub fn scalar_mul(iters: u32) -> u64 {
    let mut base = pallas::Point::generator();
    let mut scalar = pallas::Scalar::from(0x1234_5678_9ABC_DEF0u64);
    let step = pallas::Scalar::from(0x9E37_79B9_7F4A_7C15u64);
    let mut acc = 0u64;
    for _ in 0..iters {
        scalar += step;
        let product = black_box(base) * black_box(scalar);
        absorb_point(&mut acc, product);
        // Chain the next base off the product so it stays non-identity and
        // varies; the single point addition is negligible next to the mul.
        base += product;
    }
    black_box(acc)
}

/// N iterations of `commit_ivk` (the FVK-derivation cost): the
/// `sinsemilla::CommitDomain::short_commit` over the CommitIvk domain
/// (`orchard::spec::commit_ivk`), a 510-bit Sinsemilla hash plus the blinding
/// term and an x-coordinate extraction.
pub fn commit_ivk(iters: u32) -> u64 {
    let domain = CommitDomain::new(COMMIT_IVK_PERSONALIZATION);
    let mut acc = 0u64;
    for i in 0..iters {
        let msg = message(COMMIT_IVK_MSG_BITS, 0xABCD_EF ^ i as u64);
        let r = pallas::Scalar::from(0x9E37_79B9_7F4A_7C15u64.wrapping_mul(i as u64 + 1));
        let base = Option::from(domain.short_commit(black_box(msg).into_iter(), black_box(&r)))
            .unwrap_or(pallas::Base::ZERO);
        absorb_base(&mut acc, base);
    }
    black_box(acc)
}
