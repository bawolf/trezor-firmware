// SPDX-License-Identifier: GPL-3.0-or-later
#ifndef IRONWOOD_MEMORY_TRACE_H
#define IRONWOOD_MEMORY_TRACE_H

#include <stddef.h>
#include <stdint.h>

enum ironwood_memory_phase {
  IRONWOOD_MEMORY_INPUT_READY,
  IRONWOOD_MEMORY_REVIEW_READY,
  IRONWOOD_MEMORY_BEFORE_RESPONSE,
  IRONWOOD_MEMORY_RESPONSE_RESERVED,
  IRONWOOD_MEMORY_SIGN_RETURNED,
  IRONWOOD_MEMORY_CANCELLED,
  IRONWOOD_MEMORY_PHASE_COUNT,
};

typedef struct {
  size_t used_bytes;
  size_t free_bytes;
  size_t largest_free_bytes;
} ironwood_memory_record_t;

typedef struct {
  ironwood_memory_record_t records[IRONWOOD_MEMORY_PHASE_COUNT];
  uint32_t valid_phases;
} ironwood_memory_trace_t;

// Call only from the same executor; capture requires initialized, quiescent GC.
// Reset clears every record and validity bit. External cancellation does not.
void ironwood_memory_reset(void);

// Overwrite one phase and set its bit; ignore invalid phases without sampling.
// Used/free aggregate all areas; largest_free_bytes is one run in one area.
void ironwood_memory_capture(enum ironwood_memory_phase phase);

// Borrow a live read-only view, changed by later capture/reset calls; no copy.
const ironwood_memory_trace_t *ironwood_memory_get(void);

#endif
