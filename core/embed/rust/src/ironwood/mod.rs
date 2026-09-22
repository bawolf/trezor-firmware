//! Zcash Ironwood device integration: the signing region's global allocator
//! and the native state of one streamed signing request.
//!
//! The MicroPython bindings live in `micropython/ironwood.rs`, keeping the
//! logic/FFI split `thp/mod.rs` and `thp/micropython.rs` use.

#[cfg(target_arch = "arm")]
pub(crate) mod allocator;
#[cfg(not(target_arch = "arm"))]
#[path = "allocator_unix.rs"]
pub(crate) mod allocator;
pub(crate) mod signing;
