use super::ffi;

/// Fills `buffer` from the random number generator.
pub fn fill_buffer(buffer: &mut [u8]) {
    // SAFETY: `buffer` is valid for writes of `buffer.len()` bytes, at any
    // alignment.
    unsafe { ffi::rng_fill_buffer(buffer.as_mut_ptr().cast(), buffer.len()) }
}
