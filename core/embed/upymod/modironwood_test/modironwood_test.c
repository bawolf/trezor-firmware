// Callable synthetic module for link measurement; requires a trusted caller.
// sign internally approves the matching token. A separate IRONWOOD_NATIVE_CALLER
// opt-in adds the trusted route; the original compile-only configuration has none.
#include "ironwood_test.h"
#include "py/objstr.h"
#include "py/runtime.h"

#if !defined(IRONWOOD_TARGET_NATIVE_COMPILE_ONLY) || \
    !defined(TREZOR_MODEL_T3T1) || !defined(STM32U585xx) || \
    defined(USE_SECMON_LAYOUT) || \
    !defined(__ARM_FEATURE_CMSE) || __ARM_FEATURE_CMSE != 3 || \
    defined(TREZOR_EMULATOR) || defined(KERNEL_MODE) || \
    defined(SECURE_MODE) || PRODUCTION
#error "ironwood_test requires the isolated nonproduction T3T1 native compile feature"
#endif

#ifdef IRONWOOD_NATIVE_CALLER
#include "memory_trace.h"
static bool memory_active;

static void memory_capture(enum ironwood_memory_phase phase) {
  if (memory_active) {
    ironwood_memory_capture(phase);
  }
}
#else
#define memory_capture(phase) ((void)0)
#endif

// The integrating firmware checks the bound core-app executor and arena owners
// before/after calls. No Rust guard or reference survives these ABI calls.
// Only immutable VM bytes may lend the internal seed; never a wire field.
static mp_buffer_info_t read_seed(mp_obj_t seed) {
  mp_buffer_info_t buffer;
  if (!mp_obj_is_type(seed, &mp_type_bytes) ||
      !mp_get_buffer(seed, &buffer, MP_BUFFER_READ) || buffer.buf == NULL ||
      buffer.len < 32 || buffer.len > 252) {
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("invalid wallet seed"));
  }
  return buffer;
}

static mp_obj_t begin(mp_obj_t seed, mp_obj_t pczt) {
  ironwood_bridge_require_executor();
  // Buffer conversion may raise; cancel the previous review before attempting it.
  ironwood_test_cancel();
  mp_buffer_info_t seed_buffer = read_seed(seed);
  mp_buffer_info_t pczt_buffer;
  if (!mp_get_buffer(pczt, &pczt_buffer, MP_BUFFER_READ) ||
      pczt_buffer.buf == NULL || pczt_buffer.len == 0 ||
      pczt_buffer.len > IRONWOOD_MAX_PCZT_BYTES) {
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("invalid PCZT buffer"));
  }
  memory_capture(IRONWOOD_MEMORY_INPUT_READY);
  ironwood_bridge_prepare();
  ironwood_snapshot_t snapshot = {0};
  // Borrow only for this synchronous call; no MicroPython work while Rust is live.
  if (ironwood_test_begin(seed_buffer.buf, seed_buffer.len, pczt_buffer.buf, pczt_buffer.len, &snapshot) != 0) {
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("synthetic validation failed"));
  }
  if (snapshot.output_count > 8) {
    ironwood_test_cancel();
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("invalid review output count"));
  }
  for (size_t i = 0; i < snapshot.output_count; i++) {
    if (snapshot.outputs[i].kind > 1) {
      ironwood_test_cancel();
      ironwood_bridge_require_idle();
      mp_raise_ValueError(MP_ERROR_TEXT("invalid review output kind"));
    }
  }
  // Rust has returned and owns the parsed PCZT, without retaining input pointers.
  // Python must wrap begin/review/sign in finally: cancel(), including argument or
  // buffer-conversion exceptions and failures allocating/converting this review.
  mp_obj_t review = mp_obj_new_dict(12);
#define FIELD(name, value) \
  mp_obj_dict_store(review, MP_OBJ_NEW_QSTR(MP_QSTR_##name), (value))
  FIELD(token, mp_obj_new_bytes(snapshot.token, sizeof(snapshot.token)));
  FIELD(network, mp_obj_new_str("regtest (synthetic)", 19));
  FIELD(pool, MP_OBJ_NEW_QSTR(MP_QSTR_Ironwood));
  FIELD(branch, mp_obj_new_int_from_uint(snapshot.branch));
  FIELD(expiry, mp_obj_new_int_from_uint(snapshot.expiry));
  FIELD(padding_outputs, mp_obj_new_int_from_uint(snapshot.padding_outputs));
  FIELD(total_input, mp_obj_new_int_from_ull(snapshot.total_input));
  FIELD(payments, mp_obj_new_int_from_ull(snapshot.payments));
  FIELD(change, mp_obj_new_int_from_ull(snapshot.change));
  FIELD(fee, mp_obj_new_int_from_ull(snapshot.fee));
  mp_obj_t outputs[8] = {0};
  for (size_t i = 0; i < snapshot.output_count; i++) {
    const ironwood_output_t *out = &snapshot.outputs[i];
    mp_obj_t item = mp_obj_new_dict(4);
    mp_obj_dict_store(item, MP_OBJ_NEW_QSTR(MP_QSTR_action_index),
                      mp_obj_new_int_from_uint(out->action_index));
    mp_obj_dict_store(item, MP_OBJ_NEW_QSTR(MP_QSTR_value),
                      mp_obj_new_int_from_ull(out->value));
    mp_obj_dict_store(item, MP_OBJ_NEW_QSTR(MP_QSTR_receiver),
                      mp_obj_new_bytes(out->receiver, sizeof(out->receiver)));
    mp_obj_dict_store(item, MP_OBJ_NEW_QSTR(MP_QSTR_kind),
                      MP_OBJ_NEW_QSTR(out->kind == 0 ? MP_QSTR_Payment : MP_QSTR_InternalChange));
    outputs[i] = item;
  }
  FIELD(outputs, mp_obj_new_tuple(snapshot.output_count, outputs));
#undef FIELD
  memory_capture(IRONWOOD_MEMORY_REVIEW_READY);
  return review;
}
static MP_DEFINE_CONST_FUN_OBJ_2(begin_obj, begin);

// Raw receiver only: no address encoding, trusted display or signing approval.
static mp_obj_t receive(mp_obj_t seed, mp_obj_t index) {
  ironwood_bridge_require_executor();
  // Conversion may raise without entering Rust; invalidate the old review first.
  ironwood_test_cancel();
  mp_buffer_info_t seed_buffer = read_seed(seed);
  mp_buffer_info_t index_buffer;
  if (!mp_get_buffer(index, &index_buffer, MP_BUFFER_READ) ||
      index_buffer.buf == NULL || index_buffer.len != 11) {
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("invalid diversifier index buffer"));
  }
  ironwood_bridge_prepare();
  uint8_t receiver[43] = {0};
  // Fresh stack output is disjoint from the VM input; neither pointer is retained.
  // No MicroPython calls or allocations while Rust is live.
  int32_t result = ironwood_test_receive(seed_buffer.buf, seed_buffer.len, index_buffer.buf, receiver);
  ironwood_bridge_require_idle();
  if (result != 0) {
    mp_raise_ValueError(MP_ERROR_TEXT("synthetic receive failed"));
  }
  // Rust has returned idle before this allocating copy (which may raise).
  return mp_obj_new_bytes(receiver, sizeof(receiver));
}
static MP_DEFINE_CONST_FUN_OBJ_2(receive_obj, receive);

static mp_obj_t sign(mp_obj_t token) {
  ironwood_bridge_require_executor();
  mp_buffer_info_t token_buffer;
  if (!mp_get_buffer(token, &token_buffer, MP_BUFFER_READ) || token_buffer.len != 32) {
    ironwood_test_cancel();
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("invalid review token"));
  }
  // GC storage is separate from the fixed Rust arena. Python's finally cancels
  // the review if allocating/converting the response raises an exception.
  vstr_t response;
  size_t capacity = ironwood_test_response_capacity();
  if (capacity == 0 || capacity > IRONWOOD_RESPONSE_CAPACITY) {
    ironwood_test_cancel();
    ironwood_bridge_require_idle();
    mp_raise_ValueError(MP_ERROR_TEXT("invalid response capacity"));
  }
  memory_capture(IRONWOOD_MEMORY_BEFORE_RESPONSE);
  vstr_init_len(&response, capacity);
  memory_capture(IRONWOOD_MEMORY_RESPONSE_RESERVED);
  size_t written = 0;
  int32_t result = ironwood_test_sign(token_buffer.buf, (uint8_t *)response.buf,
                                    response.len, &written);
  ironwood_bridge_require_idle();
  memory_capture(IRONWOOD_MEMORY_SIGN_RETURNED);
  if (result != 0 || written > response.len) {
    vstr_clear(&response);
    mp_raise_ValueError(MP_ERROR_TEXT("synthetic signing failed"));
  }
  vstr_cut_tail_bytes(&response, response.len - written);
  return mp_obj_new_bytes_from_vstr(&response);
}
static MP_DEFINE_CONST_FUN_OBJ_1(sign_obj, sign);

static mp_obj_t cancel(void) {
  ironwood_bridge_require_executor();
  ironwood_test_cancel();
  ironwood_bridge_require_idle();
  return mp_const_none;
}
static MP_DEFINE_CONST_FUN_OBJ_0(cancel_obj, cancel);

#ifdef IRONWOOD_NATIVE_CALLER
// Only the admitted transport starts a trace; cancellation never resets it.
static mp_obj_t memory_start(void) {
  ironwood_bridge_require_executor();
  if (memory_active) {
    mp_raise_ValueError(MP_ERROR_TEXT("memory trace active"));
  }
  ironwood_bridge_require_idle();
  ironwood_memory_reset();
  memory_active = true;
  return mp_const_none;
}
static MP_DEFINE_CONST_FUN_OBJ_0(memory_start_obj, memory_start);

// Called by the outer transport finally, after its ordinary cancellation.
static mp_obj_t memory_finish(void) {
  ironwood_bridge_require_executor();
  ironwood_bridge_require_idle();
  memory_capture(IRONWOOD_MEMORY_CANCELLED);
  memory_active = false;
  return mp_const_none;
}
static MP_DEFINE_CONST_FUN_OBJ_0(memory_finish_obj, memory_finish);

static void memory_write_u32(uint8_t *out, uint32_t value) {
  for (size_t i = 0; i < sizeof(value); i++) {
    out[i] = (uint8_t)(value >> (8 * i));
  }
}

// Fixed diagnostic wire format: version, mask, then six triples, all LE u32.
static mp_obj_t memory_trace(void) {
  ironwood_bridge_require_executor();
  if (memory_active) {
    mp_raise_ValueError(MP_ERROR_TEXT("memory trace active"));
  }
  ironwood_bridge_require_idle();
  _Static_assert(sizeof(size_t) == sizeof(uint32_t), "native trace word size");
  _Static_assert(IRONWOOD_MEMORY_PHASE_COUNT == 6, "trace wire phase count");
  const ironwood_memory_trace_t *trace = ironwood_memory_get();
  uint8_t data[8 + IRONWOOD_MEMORY_PHASE_COUNT * 12];
  memory_write_u32(data, 1);
  memory_write_u32(data + 4, trace->valid_phases);
  for (size_t i = 0; i < IRONWOOD_MEMORY_PHASE_COUNT; i++) {
    const ironwood_memory_record_t *record = &trace->records[i];
    memory_write_u32(data + 8 + i * 12, record->used_bytes);
    memory_write_u32(data + 12 + i * 12, record->free_bytes);
    memory_write_u32(data + 16 + i * 12, record->largest_free_bytes);
  }
  return mp_obj_new_bytes(data, sizeof(data));
}
static MP_DEFINE_CONST_FUN_OBJ_0(memory_trace_obj, memory_trace);
#endif

static const mp_rom_map_elem_t globals_table[] = {
    {MP_ROM_QSTR(MP_QSTR___name__), MP_ROM_QSTR(MP_QSTR_ironwood_test)},
    {MP_ROM_QSTR(MP_QSTR_begin), MP_ROM_PTR(&begin_obj)},
    {MP_ROM_QSTR(MP_QSTR_receive), MP_ROM_PTR(&receive_obj)},
    {MP_ROM_QSTR(MP_QSTR_sign), MP_ROM_PTR(&sign_obj)},
    {MP_ROM_QSTR(MP_QSTR_cancel), MP_ROM_PTR(&cancel_obj)},
#ifdef IRONWOOD_NATIVE_CALLER
    {MP_ROM_QSTR(MP_QSTR_memory_start), MP_ROM_PTR(&memory_start_obj)},
    {MP_ROM_QSTR(MP_QSTR_memory_finish), MP_ROM_PTR(&memory_finish_obj)},
    {MP_ROM_QSTR(MP_QSTR_memory_trace), MP_ROM_PTR(&memory_trace_obj)},
#endif
};
static MP_DEFINE_CONST_DICT(globals, globals_table);
const mp_obj_module_t mp_module_ironwood_test = {
    .base = {&mp_type_module}, .globals = (mp_obj_dict_t *)&globals,
};
MP_REGISTER_MODULE(MP_QSTR_ironwood_test, mp_module_ironwood_test);
