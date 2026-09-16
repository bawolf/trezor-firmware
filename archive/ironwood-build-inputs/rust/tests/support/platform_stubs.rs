//! Synthetic platform symbols for host tests, never selected by a firmware feature.
//! The original response-sizing test defaults to seed42; RNG tests set42 or43.
//! Assertion failures across this C ABI abort the child and must fail its owner.
#[cfg(target_os = "none")]
compile_error!("synthetic entropy symbols must not be linked into firmware");

use core::ffi::c_void;
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering::SeqCst};

pub static SEED: AtomicU8 = AtomicU8::new(42);
pub static ENTROPY_CALLS: AtomicUsize = AtomicUsize::new(0);
pub static CLEAR_CALLS: AtomicUsize = AtomicUsize::new(0);
static SEED_POINTER: AtomicUsize = AtomicUsize::new(0);

#[unsafe(no_mangle)]
unsafe extern "C" fn rng_fill_buffer_strong(buffer: *mut c_void, length: usize) {
    assert!(!buffer.is_null());
    assert_eq!(length, 32);
    assert_eq!((buffer as usize) % align_of::<u32>(), 0);
    assert_eq!(ENTROPY_CALLS.fetch_add(1, SeqCst), CLEAR_CALLS.load(SeqCst));
    // Synthetic process fail-stop only; never unwind through this C ABI.
    if std::env::var("IRONWOOD_TEST_ENTROPY_STOP_AT")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        == Some(ENTROPY_CALLS.load(SeqCst))
    {
        std::process::exit(77);
    }
    SEED_POINTER.store(buffer as usize, SeqCst);
    unsafe { std::ptr::write_bytes(buffer.cast::<u8>(), SEED.load(SeqCst), length) };
}

#[unsafe(no_mangle)]
unsafe extern "C" fn memzero(buffer: *mut c_void, length: usize) {
    assert_eq!(
        CLEAR_CALLS.fetch_add(1, SeqCst) + 1,
        ENTROPY_CALLS.load(SeqCst)
    );
    assert_eq!(buffer as usize, SEED_POINTER.load(SeqCst));
    assert_eq!(length, 32);
    let scratch = unsafe { std::slice::from_raw_parts_mut(buffer.cast::<u8>(), length) };
    assert_eq!(scratch, &[SEED.load(SeqCst); 32]);
    scratch.fill(0);
    assert!(scratch.iter().all(|byte| *byte == 0));
}
