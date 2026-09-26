//! Resource counters for measuring an app on a device, in `debug` builds only:
//! the heap high-water mark, and the longest time the app kept Core waiting
//! for its next IPC message (Core stops an app after 1 s, see
//! [`crate::ui::Progress`]). An app can report them with a debug-only message
//! of its own.

use core::alloc::{GlobalAlloc, Layout};
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::{AtomicU16, AtomicU32, AtomicUsize};

use embedded_alloc::LlffHeap;

use crate::low_level_api;

/// The counters since the previous [`take`] (or since the app started).
#[derive(Clone, Copy, Debug)]
pub struct Diagnostics {
    /// Size of the app heap in bytes.
    pub heap_size: u32,
    /// Bytes allocated now.
    pub heap_used: u32,
    /// Most bytes allocated at once.
    pub heap_peak: u32,
    /// Longest time in ms between the app receiving an IPC message and
    /// sending its next one.
    pub max_ipc_silence_ms: u32,
    /// Service id ([`crate::service::CoreIpcService`]) of the message that
    /// ended that silence.
    pub max_ipc_silence_service: u16,
    /// IPC messages the app sent.
    pub ipc_sent: u32,
}

/// Returns the counters and starts new ones: the heap peak from the current
/// use, the silence and the message count from zero.
pub fn take() -> Diagnostics {
    let counters = snapshot();
    let heap = &crate::app_runtime::HEAP;
    heap.peak.store(counters.heap_used as usize, Relaxed);
    MAX_SILENCE_MS.store(0, Relaxed);
    MAX_SILENCE_SERVICE.store(0, Relaxed);
    IPC_SENT.store(0, Relaxed);
    counters
}

/// Returns the counters without starting new ones. An app can send this
/// with each message of a long request, so its host keeps the latest values
/// even if Core stops the app before the request ends.
pub fn snapshot() -> Diagnostics {
    let heap = &crate::app_runtime::HEAP;
    Diagnostics {
        heap_size: heap.size.load(Relaxed) as u32,
        heap_used: heap.heap.used() as u32,
        heap_peak: heap.peak.load(Relaxed) as u32,
        max_ipc_silence_ms: MAX_SILENCE_MS.load(Relaxed),
        max_ipc_silence_service: MAX_SILENCE_SERVICE.load(Relaxed),
        ipc_sent: IPC_SENT.load(Relaxed),
    }
}

/// The app's global allocator in `debug` builds: [`LlffHeap`] recording its
/// high-water mark.
pub(crate) struct PeakHeap {
    heap: LlffHeap,
    size: AtomicUsize,
    peak: AtomicUsize,
}

impl PeakHeap {
    pub(crate) const fn empty() -> Self {
        Self {
            heap: LlffHeap::empty(),
            size: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
        }
    }

    /// See [`LlffHeap::init`].
    pub(crate) unsafe fn init(&self, start: usize, size: usize) {
        unsafe { self.heap.init(start, size) };
        self.size.store(size, Relaxed);
    }
}

unsafe impl GlobalAlloc for PeakHeap {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { self.heap.alloc(layout) };
        self.peak.fetch_max(self.heap.used(), Relaxed);
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { self.heap.dealloc(ptr, layout) }
    }
}

static LAST_RECEIVED_MS: AtomicU32 = AtomicU32::new(0);
static MAX_SILENCE_MS: AtomicU32 = AtomicU32::new(0);
static MAX_SILENCE_SERVICE: AtomicU16 = AtomicU16::new(0);
static IPC_SENT: AtomicU32 = AtomicU32::new(0);

/// Records that an IPC message arrived.
pub(crate) fn ipc_received() {
    LAST_RECEIVED_MS.store(low_level_api::systick_ms(), Relaxed);
}

/// Records that the app sent a message to `service`.
pub(crate) fn ipc_sent(service: u16) {
    let silence = low_level_api::systick_ms().wrapping_sub(LAST_RECEIVED_MS.load(Relaxed));
    if silence > MAX_SILENCE_MS.load(Relaxed) {
        MAX_SILENCE_MS.store(silence, Relaxed);
        MAX_SILENCE_SERVICE.store(service, Relaxed);
    }
    IPC_SENT.fetch_add(1, Relaxed);
}
