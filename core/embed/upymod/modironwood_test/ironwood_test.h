#ifndef IRONWOOD_TEST_H
#define IRONWOOD_TEST_H
#include <stddef.h>
#include <stdint.h>

#define IRONWOOD_MAX_PCZT_BYTES 65536
#define IRONWOOD_RESPONSE_CAPACITY 65536

typedef struct {
  uint64_t value;
  uint32_t action_index;
  uint8_t kind;
  uint8_t receiver[43];
} ironwood_output_t;

typedef struct {
  uint64_t total_input, payments, change, fee;
  uint32_t branch, expiry, padding_outputs, output_count;
  uint8_t token[32];
  ironwood_output_t outputs[8];
} ironwood_snapshot_t;

_Static_assert(sizeof(ironwood_output_t) == 56, "Rust Output ABI");
_Static_assert(sizeof(ironwood_snapshot_t) == 528, "Rust Snapshot ABI");

_Static_assert(offsetof(ironwood_snapshot_t, total_input) == 0, "Rust Snapshot total_input");
_Static_assert(offsetof(ironwood_snapshot_t, payments) == 8, "Rust Snapshot payments");
_Static_assert(offsetof(ironwood_snapshot_t, change) == 16, "Rust Snapshot change");
_Static_assert(offsetof(ironwood_snapshot_t, fee) == 24, "Rust Snapshot fee");
_Static_assert(offsetof(ironwood_snapshot_t, branch) == 32, "Rust Snapshot branch");
_Static_assert(offsetof(ironwood_snapshot_t, expiry) == 36, "Rust Snapshot expiry");
_Static_assert(offsetof(ironwood_snapshot_t, padding_outputs) == 40, "Rust Snapshot padding_outputs");
_Static_assert(offsetof(ironwood_snapshot_t, output_count) == 44, "Rust Snapshot output_count");
_Static_assert(offsetof(ironwood_snapshot_t, token) == 48, "Rust Snapshot token");
_Static_assert(offsetof(ironwood_snapshot_t, outputs) == 80, "Rust Snapshot outputs");
_Static_assert(offsetof(ironwood_output_t, value) == 0, "Rust Output value");
_Static_assert(offsetof(ironwood_output_t, action_index) == 8, "Rust Output action_index");
_Static_assert(offsetof(ironwood_output_t, kind) == 12, "Rust Output kind");
_Static_assert(offsetof(ironwood_output_t, receiver) == 13, "Rust Output receiver");

// Synthetic external receiver: read 11 LE index bytes, write 43 raw bytes on
// success (0). Buffers must be disjoint/outside the arena for this call only.
// Cancels pending signing before argument checks; errors leave output unchanged.
int32_t ironwood_test_receive(const uint8_t *seed, size_t seed_length,
                              const uint8_t *index, uint8_t *receiver);

// All inputs are borrowed synchronously; no pointer is retained. Seed is
// immutable32..252bytes from trusted workflow code, never from the wire.
int32_t ironwood_test_begin(const uint8_t *seed, size_t seed_length,
                            const uint8_t *pczt, size_t length,
                            ironwood_snapshot_t *snapshot);
// Zero means there is no valid pending review; this never approves it.
size_t ironwood_test_response_capacity(void);
int32_t ironwood_test_sign(const uint8_t *token, uint8_t *output, size_t capacity,
                          size_t *written);
void ironwood_test_cancel(void);
void ironwood_bridge_require_executor(void);
void ironwood_bridge_prepare(void);
void ironwood_bridge_require_idle(void);
#endif
