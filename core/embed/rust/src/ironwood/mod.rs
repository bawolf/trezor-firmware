//! Zcash shielded device integration: the signing region's global allocator
//! and the native state of one streamed signing request.
//!
//! The MicroPython bindings live in `micropython/zcash.rs`, keeping the
//! logic/FFI split `thp/mod.rs` and `thp/micropython.rs` use.

// The arena mechanics are target-independent and are the only part of the
// device allocator a host test can reach: the emulator runs `allocator_unix`,
// which is plain `malloc`.
#[cfg(all(feature = "zcash_shielded", target_arch = "arm"))]
pub(crate) mod allocator;
#[cfg(all(feature = "zcash_shielded", not(target_arch = "arm")))]
#[path = "allocator_unix.rs"]
pub(crate) mod allocator;
pub(crate) mod arena;
#[cfg(feature = "zcash_shielded")]
pub(crate) mod signing;
