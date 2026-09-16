// SPDX-License-Identifier: GPL-3.0-or-later
// Synthetic link-measurement adapter; requires a trusted Core caller.
#include <limits.h>
#include <stdbool.h>
#include <stdatomic.h>
#include <stdint.h>

#include "cmsis_compiler.h"
#include "py/mpconfig.h"

#if !defined(IRONWOOD_TARGET_NATIVE_COMPILE_ONLY) || \
    IRONWOOD_TARGET_NATIVE_COMPILE_ONLY != 1 || \
    !defined(TREZOR_MODEL_T3T1) || !defined(STM32U585xx) || \
    defined(TREZOR_MODEL_T3W1) || defined(STM32U5G9xx) || \
    defined(USE_SECMON_LAYOUT) || \
    !defined(__ARM_FEATURE_CMSE) || __ARM_FEATURE_CMSE != 3 || \
    !defined(__ARM_ARCH_8M_MAIN__) || !defined(__thumb__)
#error "executor requires the opt-in actual T3T1 secure Core target"
#endif
#if defined(USE_APP_LOADING) || defined(TREZOR_EMULATOR) || \
    defined(KERNEL_MODE) || defined(KERNEL) || defined(SECURE_MODE) || \
    PRODUCTION
#error "executor requires nonproduction secure Core without app loading"
#endif
#if !defined(MICROPY_PY_THREAD) || MICROPY_PY_THREAD || \
    !defined(MICROPY_ENABLE_SCHEDULER) || MICROPY_ENABLE_SCHEDULER
#error "executor requires disabled MicroPython threads and scheduler"
#endif
#if !defined(__SSP_ALL__)
#error "retain stack-protector-all on the state-access functions"
#endif
#if !defined(__has_attribute)
#error "compiler must support the explicit register-only gate attributes"
#elif !__has_attribute(no_stack_protector) || !__has_attribute(noinline)
#error "compiler must support the explicit register-only gate attributes"
#endif

_Static_assert(ATOMIC_INT_LOCK_FREE == 2, "executor atomics must always be lock-free");
_Static_assert(UINT_MAX == UINT32_MAX, "executor phase requires 32-bit unsigned int");

// ARMv8-M CONTROL: nPRIV = bit 0; SPSEL = bit 1. FPCA is not identity.
#define EXECUTOR_CONTROL_MASK ((1u << 0) | (1u << 1))

// Ordinary Core BSS, never TLS. Only fresh Core initialization clears this phase.
static atomic_uint executor_bound = ATOMIC_VAR_INIT(0);

// Keep state access and its stack canary out of the register-only entry gates.
__attribute__((noinline)) static void bind_core_lifetime(void) {
  unsigned int expected = 0;
  if (!atomic_compare_exchange_strong_explicit(
          &executor_bound, &expected, 1, memory_order_release,
          memory_order_relaxed)) {
    __ASM volatile("udf #0" ::: "memory");
    __builtin_unreachable();
  }
}

__attribute__((noinline)) static bool core_lifetime_is_bound(void) {
  return atomic_load_explicit(&executor_bound, memory_order_acquire) == 1;
}

// A normal -fstack-protector-all prologue would read Core BSS before the gate.
// Only these new register-only entries omit it; both state helpers retain it.
// Keep this boundary out of line, including with LTO; inspect final disassembly.
__attribute__((noinline, no_stack_protector)) void ironwood_bind_executor(void) {
  if (__get_IPSR() != 0 ||
      (__get_CONTROL() & EXECUTOR_CONTROL_MASK) != EXECUTOR_CONTROL_MASK) {
    __ASM volatile("udf #0" ::: "memory");
    __builtin_unreachable();
  }
  __COMPILER_BARRIER();
  bind_core_lifetime();
}

__attribute__((noinline, no_stack_protector)) bool ironwood_is_executor(void) {
  if (__get_IPSR() != 0 ||
      (__get_CONTROL() & EXECUTOR_CONTROL_MASK) != EXECUTOR_CONTROL_MASK) {
    return false;
  }
  __COMPILER_BARRIER();
  return core_lifetime_is_bound();
}
