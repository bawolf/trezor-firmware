// SPDX-License-Identifier: GPL-3.0-or-later
#include "memory_trace.h"

#include <string.h>

#include "py/mpconfig.h"
#include "py/gc.h"

_Static_assert(IRONWOOD_MEMORY_PHASE_COUNT <= 32, "phase mask capacity");

static ironwood_memory_trace_t trace;

void ironwood_memory_reset(void) { memset(&trace, 0, sizeof(trace)); }

void ironwood_memory_capture(enum ironwood_memory_phase phase) {
  if ((unsigned int)phase >= IRONWOOD_MEMORY_PHASE_COUNT) {
    return;
  }

  gc_info_t info;
  gc_info(&info);
  ironwood_memory_record_t *record = &trace.records[phase];
  record->used_bytes = info.used;
  record->free_bytes = info.free;
  record->largest_free_bytes = info.max_free * MICROPY_BYTES_PER_GC_BLOCK;
  trace.valid_phases |= UINT32_C(1) << (unsigned int)phase;
}

const ironwood_memory_trace_t *ironwood_memory_get(void) { return &trace; }
