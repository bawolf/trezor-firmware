#![no_std]
//! Internal borrowed-seed PCZT bridge; isolated test build, not production-ready.
//! No USB-host-supplied key, entropy, policy or approval.
use core::{ptr, slice};
use ironwood_approval::wire::MAX_PCZT_BYTES;
use ironwood_approval::{Engine, OutputKind, Policy, Review};
use orchard::keys::{FullViewingKey, Scope, SpendAuthorizingKey, SpendingKey};
use rand_chacha::rand_core::{CryptoRng, Error as RngError, RngCore, impls};
use spin::Mutex;
use zeroize::Zeroizing;

unsafe extern "C" {
    fn rng_fill_buffer_strong(buffer: *mut core::ffi::c_void, length: usize);
    fn memzero(buffer: *mut core::ffi::c_void, length: usize);
}

// The strong platform API writes u32 words and halts on source failure.
// An aligned scratch also supports unaligned caller slices without retaining RNG state.
#[repr(C, align(4))]
struct EntropyBlock([u8; 32]);

struct StrongRng;

impl RngCore for StrongRng {
    fn next_u32(&mut self) -> u32 {
        impls::next_u32_via_fill(self)
    }

    fn next_u64(&mut self) -> u64 {
        impls::next_u64_via_fill(self)
    }

    fn fill_bytes(&mut self, destination: &mut [u8]) {
        let mut block = EntropyBlock([0; 32]);
        // Empty requests never reach C: its source-usage check requires nonzero length.
        for chunk in destination.chunks_mut(block.0.len()) {
            unsafe { rng_fill_buffer_strong(block.0.as_mut_ptr().cast(), block.0.len()) };
            chunk.copy_from_slice(&block.0[..chunk.len()]);
            // Clear this scratch, not caller buffers, compiler copies or nonce temporaries.
            unsafe { memzero(block.0.as_mut_ptr().cast(), block.0.len()) };
        }
    }

    fn try_fill_bytes(&mut self, destination: &mut [u8]) -> Result<(), RngError> {
        // The void/fatal C API cannot report a recoverable entropy error.
        self.fill_bytes(destination);
        Ok(())
    }
}

impl CryptoRng for StrongRng {}

pub const RESPONSE_CAPACITY: usize = 65_536;
pub const OK: i32 = 0;
pub const REJECTED: i32 = 1;
pub const OUTPUT_CAPACITY: i32 = 2;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Output {
    pub value: u64,
    pub action_index: u32,
    pub kind: u8, // 0 payment, 1 internal change.
    pub receiver: [u8; 43],
}

impl Default for Output {
    fn default() -> Self {
        Self {
            value: 0,
            action_index: 0,
            kind: 0,
            receiver: [0; 43],
        }
    }
}

#[repr(C)]
#[derive(Default)]
pub struct Snapshot {
    pub total_input: u64,
    pub payments: u64,
    pub change: u64,
    pub fee: u64,
    pub branch: u32,
    pub expiry: u32,
    pub padding_outputs: u32,
    pub output_count: u32,
    pub token: [u8; 32],
    pub outputs: [Output; 8],
}

struct PendingRequest {
    engine: Engine<StrongRng>,
    ask: Zeroizing<SpendAuthorizingKey>,
    review: Review,
}

// C calls only from the trusted MicroPython executor. Reentry is a fatal
// integration error. Never hold this guard across a MicroPython call/exception.
struct Bridge {
    next_request: u64,
    pending: Option<PendingRequest>,
}

static BRIDGE: Mutex<Bridge> = Mutex::new(Bridge {
    next_request: 0,
    pending: None,
});

// Trusted caller lends a wallet seed. Account/network remain fixed for this test.
fn spending_key(seed: &[u8]) -> Option<SpendingKey> {
    SpendingKey::from_zip32_seed(seed, 1, 9u32.try_into().expect("test account")).ok()
}

/// Derive a raw external receiver from the trusted workflow's borrowed seed.
/// Invalidates any pending signing review, including on invalid arguments.
/// Returns neither a Unified Address nor evidence of user confirmation.
///
/// # Safety
/// C supplies an immutable readable seed of seed_length bytes, an 11-byte LE
/// index and a writable 43-byte receiver. Output is disjoint from inputs; all
/// buffers are outside the Rust arena and stay valid for this synchronous call.
/// No input pointer is retained. Null arguments reject without writing the receiver.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ironwood_test_receive(
    seed: *const u8,
    seed_length: usize,
    index: *const u8,
    receiver: *mut u8,
) -> i32 {
    let mut bridge = BRIDGE.try_lock().expect("bridge reentry");
    bridge.pending = None;
    if seed.is_null() || !(32..=252).contains(&seed_length) || index.is_null() || receiver.is_null()
    {
        return REJECTED;
    }
    let index = unsafe { index.cast::<[u8; 11]>().read() };
    let seed = unsafe { slice::from_raw_parts(seed, seed_length) };
    let Some(spending) = spending_key(seed) else {
        return REJECTED;
    };
    let fvk = FullViewingKey::from(&spending);
    let address = fvk
        .address_at(index, Scope::External)
        .to_raw_address_bytes();
    unsafe { receiver.cast::<[u8; 43]>().write(address) };
    OK
}

/// Validate caller-supplied synthetic PCZT bytes, invalidating any prior review.
///
/// # Safety
/// C supplies an immutable seed of seed_length bytes, readable PCZT length
/// bytes and writable, aligned Snapshot storage.
/// The buffers are disjoint and outside the Rust arena; input stays unchanged
/// until this call returns. No input pointer is retained.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ironwood_test_begin(
    seed: *const u8,
    seed_length: usize,
    pczt: *const u8,
    length: usize,
    snapshot: *mut Snapshot,
) -> i32 {
    let mut bridge = BRIDGE.try_lock().expect("bridge reentry");
    // Drop the whole old request before even checking replacement arguments.
    bridge.pending = None;
    if seed.is_null()
        || !(32..=252).contains(&seed_length)
        || pczt.is_null()
        || length == 0
        || length > MAX_PCZT_BYTES
        || snapshot.is_null()
    {
        return REJECTED;
    }
    let Some(next_request) = bridge.next_request.checked_add(1) else {
        return REJECTED;
    };
    let request = bridge.next_request;
    bridge.next_request = next_request;
    let seed = unsafe { slice::from_raw_parts(seed, seed_length) };
    let Some(spending) = spending_key(seed) else {
        return REJECTED;
    };
    // Separate tokens even when synthetic entropy repeats. One begin per Engine;
    // the public per-boot counter is reserved above and cannot wrap.
    let mut engine = Engine::with_rng_and_counter(
        Policy::regtest(10_000_000, 100_000).expect("fixed test policy"),
        FullViewingKey::from(&spending),
        StrongRng,
        request,
    )
    .expect("strong platform session entropy");
    let bytes = unsafe { slice::from_raw_parts(pczt, length) };
    let Ok(review) = engine.begin(bytes) else {
        return REJECTED;
    };
    let projection = review.projection();
    if projection.outputs.len() > 8 {
        return REJECTED;
    }
    let mut result = Snapshot {
        total_input: projection.total_input,
        payments: projection.payments,
        change: projection.change,
        fee: projection.fee,
        branch: projection.branch,
        expiry: projection.expiry,
        padding_outputs: projection.padding_outputs as u32,
        output_count: projection.outputs.len() as u32,
        token: *review.token().context(),
        ..Snapshot::default()
    };
    for (output, reviewed) in result.outputs.iter_mut().zip(&projection.outputs) {
        *output = Output {
            value: reviewed.value,
            action_index: reviewed.action_index as u32,
            kind: match reviewed.kind {
                OutputKind::Payment => 0,
                OutputKind::InternalChange => 1,
            },
            receiver: reviewed.receiver,
        };
    }
    bridge.pending = Some(PendingRequest {
        engine,
        ask: Zeroizing::new(SpendAuthorizingKey::from(&spending)),
        review,
    });
    unsafe { snapshot.write(result) };
    OK
}

/// Conservative signed-wire size for the validated, currently pending review.
/// This query neither approves nor consumes consent and retains no new state.
#[unsafe(no_mangle)]
pub extern "C" fn ironwood_test_response_capacity() -> usize {
    let bridge = BRIDGE.try_lock().expect("bridge reentry");
    let Some(pending) = bridge.pending.as_ref() else {
        return 0;
    };
    let projection = pending.review.projection();
    let actions = projection.outputs.len() + projection.padding_outputs;
    if !(1..=8).contains(&actions) {
        return 0;
    }
    // Pinned profile: 92 bytes of framing/globals/trailer, at most 1301 per
    // action, including its signature. Recheck when admission/serialization changes.
    92 + 1301 * actions
}

/// Called only after the trusted UI finishes its final confirmation.
/// Every attempt consumes the pending review, including wrong-token/capacity errors.
///
/// # Safety
/// C supplies a readable 32-byte token, writable output of `capacity` bytes,
/// and writable aligned `written`, all disjoint and outside the Rust arena.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ironwood_test_sign(
    token: *const u8,
    output: *mut u8,
    capacity: usize,
    written: *mut usize,
) -> i32 {
    let mut bridge = BRIDGE.try_lock().expect("bridge reentry");
    if !written.is_null() {
        unsafe { written.write(0) };
    }
    // Keep the key in its owning slot; moving the request can leave an old copy.
    let result = (|| {
        let Some(pending) = bridge.pending.as_mut() else {
            return REJECTED;
        };
        let review = &pending.review;
        if token.is_null() || output.is_null() || written.is_null() || capacity > RESPONSE_CAPACITY
        {
            return REJECTED;
        }
        let token = unsafe { &*token.cast::<[u8; 32]>() };
        if token != review.token().context() {
            return REJECTED;
        }
        if pending.engine.approve(review.token()).is_err() {
            return REJECTED;
        }
        let Ok(signed) = pending.engine.sign(review.token(), &pending.ask) else {
            return REJECTED;
        };
        let Ok(encoded) = pczt::v2::Pczt::try_from(signed.pczt) else {
            return REJECTED;
        };
        let bytes = encoded.serialize();
        if bytes.len() > capacity {
            return OUTPUT_CAPACITY;
        }
        unsafe {
            ptr::copy_nonoverlapping(bytes.as_ptr(), output, bytes.len());
            written.write(bytes.len());
        }
        OK
    })();
    // No ordinary result, including an error, retains the request or approval.
    bridge.pending = None;
    result
}

#[unsafe(no_mangle)]
pub extern "C" fn ironwood_test_cancel() {
    BRIDGE.try_lock().expect("bridge reentry").pending = None;
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
#[path = "../tests/request_disposal.rs"]
mod request_disposal;
