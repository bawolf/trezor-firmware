/*
 * This file is part of the Trezor project, https://trezor.io/
 *
 * Copyright (c) SatoshiLabs
 *
 * This program is free software: you can redistribute it and/or modify
 * it under the terms of the GNU General Public License as published by
 * the Free Software Foundation, either version 3 of the License, or
 * (at your option) any later version.
 *
 * This program is distributed in the hope that it will be useful,
 * but WITHOUT ANY WARRANTY; without even the implied warranty of
 * MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
 * GNU General Public License for more details.
 *
 * You should have received a copy of the GNU General Public License
 * along with this program.  If not, see <http://www.gnu.org/licenses/>.
 */

#pragma once

#include <trezor_types.h>

// Test devices only, not for upstream: a firmware built without
// BOOTLOADER_DEVEL embeds the released, Trezor-signed secmon
// (models/T3W1/secmon/secmon.bin), whose smcall table predates
// SMCALL_MLDSA44_VERIFY. Such a kernel verifies ML-DSA-44 itself.
#if defined(KERNEL) && defined(USE_SECMON_LAYOUT) && !defined(BOOTLOADER_DEVEL)
#define MLDSA44_IN_KERNEL
#endif

/** Number of bytes in an ML-DSA-44 signature */
#define MLDSA44_SIGNATURE_SIZE 2420

/** Number of bytes in an ML-DSA-44 public key */
#define MLDSA44_PUBLICKEY_SIZE 1312

/** ML-DSA-44 public key structure */
typedef struct {
  uint8_t bytes[MLDSA44_PUBLICKEY_SIZE];
} mldsa44_public_key_t;

/** ML-DSA-44 signature structure */
typedef struct {
  uint8_t bytes[MLDSA44_SIGNATURE_SIZE];
} mldsa44_signature_t;

/**
 * @brief Verify a signature using the ML-DSA-44 algorithm.
 *
 * @param sig Pointer to the signature bytes.
 * @param m Pointer to the message bytes.
 * @param mlen Length of the message in bytes.
 * @param pk Pointer to the public key bytes.
 * @param valid Pointer to a secbool variable that will be set to
 *           sectrue if the signature is valid, secfalse otherwise.
 *
 * @return TS_OK if the verification process completed. Returned
 *        value does not indicate whether the signature is valid or not.
 */

ts_t mldsa44_verify(const mldsa44_signature_t *sig, const void *m, size_t mlen,
                    const mldsa44_public_key_t *pk, secbool *valid);
