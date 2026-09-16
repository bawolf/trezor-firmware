//! One image-wide allocator. No allocation, formatting or waiting under its lock.
use crate::{exit_code, fail_stop};
use core::{
    alloc::{GlobalAlloc, Layout},
    mem::MaybeUninit,
    ptr::{self, NonNull},
};
use linked_list_allocator::Heap;
use spin::mutex::{SpinMutex, SpinMutexGuard};

pub const CAPACITY: usize = 128 * 1024;
const _: () = assert!(CAPACITY >= 3 * core::mem::size_of::<usize>());

#[repr(C, align(16))]
struct Storage {
    before: [u8; 16],
    bytes: [MaybeUninit<u8>; CAPACITY],
    after: [u8; 16],
}

// T3T1 kernel clears AUX2 before Core startup; guards are filled by init.
// The existing .buf section excludes this storage from MicroPython GC.
#[unsafe(link_section = ".buf.ironwood_arena")]
static mut STORAGE: Storage = Storage {
    before: [0; 16],
    bytes: [MaybeUninit::new(0); CAPACITY],
    after: [0; 16],
};

/// Counters shared with C: every field uses that target's size_t.
/// Used/rounded counts are Heap's accounting, not fragmentation or MCU bounds.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub struct Counts {
    pub attempts: usize,
    pub allocations: usize,
    pub deallocations: usize,
    pub failures: usize,
    pub requested_total: usize,
    pub rounded_total: usize,
    pub live_requested: usize,
    pub live_blocks: usize,
    pub peak_requested: usize,
    pub peak_used: usize,
    pub largest_request: usize,
    pub used: usize,
    pub free: usize,
}

impl Counts {
    pub fn same_live(&self, other: &Self) -> bool {
        self.live_requested == other.live_requested
            && self.live_blocks == other.live_blocks
            && self.used == other.used
            && self.free == other.free
    }
}

#[derive(Clone, Copy)]
struct PublicBlock {
    address: usize,
    layout: Layout,
}

struct State {
    heap: Heap,
    counts: Counts,
    public_baseline: Option<Counts>,
    // During FP_TABLES construction at most five owners plus realloc overlap.
    // After sealing, the surviving entries protect the immutable public constants.
    public_blocks: [Option<PublicBlock>; 6],
    tracking_public: bool,
    deny_allocations: bool, // Synthetic OOM injection only; never chooses another heap.
}

impl State {
    fn record_public_alloc(&mut self, address: usize, layout: Layout) {
        if self.tracking_public {
            let slot = self
                .public_blocks
                .iter_mut()
                .find(|slot| slot.is_none())
                .unwrap_or_else(|| fail_stop(exit_code::INVARIANT));
            *slot = Some(PublicBlock { address, layout });
        }
    }

    fn check_public_free(&mut self, address: usize, layout: Layout) {
        let slot = self
            .public_blocks
            .iter_mut()
            .find(|slot| slot.is_some_and(|block| block.address == address));
        if let Some(slot) = slot {
            if !self.tracking_public || slot.is_none_or(|block| block.layout != layout) {
                fail_stop(exit_code::INVARIANT);
            }
            *slot = None;
        } else if self.tracking_public {
            fail_stop(exit_code::INVARIANT);
        }
    }

    fn initialized(&self) {
        if self.heap.bottom().is_null() {
            fail_stop(exit_code::PREINIT)
        }
    }
}

pub struct Arena(SpinMutex<State>);

#[global_allocator]
pub static ALLOCATOR: Arena = Arena(SpinMutex::new(State {
    heap: Heap::empty(),
    counts: Counts {
        attempts: 0,
        allocations: 0,
        deallocations: 0,
        failures: 0,
        requested_total: 0,
        rounded_total: 0,
        live_requested: 0,
        live_blocks: 0,
        peak_requested: 0,
        peak_used: 0,
        largest_request: 0,
        used: 0,
        free: 0,
    },
    public_baseline: None,
    public_blocks: [None; 6],
    tracking_public: false,
    deny_allocations: false,
}));

fn add(a: usize, b: usize) -> usize {
    a.checked_add(b)
        .unwrap_or_else(|| fail_stop(exit_code::INVARIANT))
}
fn sub(a: usize, b: usize) -> usize {
    a.checked_sub(b)
        .unwrap_or_else(|| fail_stop(exit_code::INVARIANT))
}

impl Arena {
    fn lock(&self) -> SpinMutexGuard<'_, State> {
        self.0
            .try_lock()
            .unwrap_or_else(|| fail_stop(exit_code::REENTRY))
    }

    pub fn init(&self) {
        let mut state = self.lock();
        // This check MUST precede construction of the unique static mutable slice.
        if !state.heap.bottom().is_null() {
            fail_stop(exit_code::DOUBLE_INIT)
        }
        // SAFETY: the sole init caller holds the lock, and Heap::bottom proved no
        // prior initialization. No other code borrows this field, resets or extends
        // the heap. The disjoint guards never belong to Heap or its allocations.
        unsafe {
            ptr::addr_of_mut!(STORAGE.before).write([0xa5; 16]);
            ptr::addr_of_mut!(STORAGE.after).write([0x5a; 16]);
            let bytes = ptr::addr_of_mut!(STORAGE.bytes).cast::<MaybeUninit<u8>>();
            state
                .heap
                .init_from_slice(core::slice::from_raw_parts_mut(bytes, CAPACITY));
        }
    }

    pub fn snapshot(&self) -> Counts {
        let state = self.lock();
        state.initialized();
        Counts {
            used: state.heap.used(),
            free: state.heap.free(),
            ..state.counts
        }
    }

    pub fn require_empty(&self) {
        let state = self.lock();
        state.initialized();
        if state.heap.used() != 0
            || state.counts.live_requested != 0
            || state.counts.live_blocks != 0
        {
            fail_stop(exit_code::INVARIANT)
        }
        // SAFETY: these disjoint fields are initialized once, checked under the
        // same lock, and never given to Heap or C. Read copies, not arena references.
        unsafe {
            if ptr::addr_of!(STORAGE.before).read() != [0xa5; 16]
                || ptr::addr_of!(STORAGE.after).read() != [0x5a; 16]
            {
                fail_stop(exit_code::INVARIANT)
            }
        }
    }

    pub fn public_baseline(&self) -> Option<Counts> {
        let state = self.lock();
        state.initialized();
        state.public_baseline
    }

    pub fn start_public_init(&self) {
        let mut state = self.lock();
        state.initialized();
        if state.public_baseline.is_some() || state.tracking_public || state.heap.used() != 0 {
            fail_stop(exit_code::INVARIANT);
        }
        state.tracking_public = true;
    }

    pub fn seal_public_baseline(&self, baseline: Counts, layouts: [Layout; 5]) {
        let mut state = self.lock();
        state.initialized();
        if state.public_baseline.is_some() || !state.tracking_public {
            fail_stop(exit_code::INVARIANT);
        }
        let mut matched = [false; 5];
        for block in state.public_blocks.iter().flatten() {
            let index = layouts
                .iter()
                .enumerate()
                .position(|(i, layout)| !matched[i] && *layout == block.layout)
                .unwrap_or_else(|| fail_stop(exit_code::INVARIANT));
            matched[index] = true;
        }
        if !matched.iter().all(|value| *value) {
            fail_stop(exit_code::INVARIANT);
        }
        state.tracking_public = false;
        state.public_baseline = Some(baseline);
    }

    pub fn test_sealed_free_gate(&self) -> ! {
        let mut state = self.lock();
        let block = state
            .public_blocks
            .iter()
            .flatten()
            .next()
            .copied()
            .unwrap_or_else(|| fail_stop(exit_code::TEST_FAILURE));
        // Test the actual guard, without an invalid GlobalAlloc call, pointer
        // reconstruction or freeing anything owned by an upstream lazy static.
        state.check_public_free(block.address, block.layout);
        fail_stop(exit_code::TEST_FAILURE)
    }

    pub fn require_baseline(&self, baseline: Counts) {
        let current = self.snapshot();
        if !current.same_live(&baseline) {
            fail_stop(exit_code::INVARIANT);
        }
        self.check_canaries();
    }

    pub fn check_canaries(&self) {
        let state = self.lock();
        state.initialized();
        // SAFETY: disjoint guard fields, copied under the same initialization lock.
        unsafe {
            if ptr::addr_of!(STORAGE.before).read() != [0xa5; 16]
                || ptr::addr_of!(STORAGE.after).read() != [0x5a; 16]
            {
                fail_stop(exit_code::INVARIANT);
            }
        }
    }

    pub fn deny_allocations(&self) {
        let mut state = self.lock();
        state.initialized();
        state.deny_allocations = true;
    }

    pub fn test_reentry(&self) -> ! {
        let _guard = self.lock();
        let _ = self.snapshot(); // Must terminate instead of waiting on this guard.
        fail_stop(exit_code::TEST_FAILURE)
    }
}

// SAFETY: callers supply valid nonzero Layouts and exact live pointer/layout
// pairs. Heap satisfies each alignment or returns failure. All heap/counter/init
// accesses share try_lock. Only allocation pointers leave this module; callers own their lifetimes.
unsafe impl GlobalAlloc for Arena {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        crate::arena_startup::ironwood_bridge_require_executor();
        let mut state = self.lock();
        state.initialized();
        state.counts.attempts = add(state.counts.attempts, 1);
        state.counts.requested_total = add(state.counts.requested_total, layout.size());
        state.counts.largest_request = state.counts.largest_request.max(layout.size());
        let previous_used = state.heap.used();
        let result = if state.deny_allocations {
            Err(())
        } else {
            state.heap.allocate_first_fit(layout)
        };
        match result {
            Err(()) => {
                state.counts.failures = add(state.counts.failures, 1);
                ptr::null_mut()
            }
            Ok(pointer) => {
                state.counts.allocations = add(state.counts.allocations, 1);
                state.counts.rounded_total = add(
                    state.counts.rounded_total,
                    sub(state.heap.used(), previous_used),
                );
                state.counts.live_requested = add(state.counts.live_requested, layout.size());
                state.counts.live_blocks = add(state.counts.live_blocks, 1);
                state.counts.peak_requested =
                    state.counts.peak_requested.max(state.counts.live_requested);
                state.counts.peak_used = state.counts.peak_used.max(state.heap.used());
                state.record_public_alloc(pointer.as_ptr() as usize, layout);
                pointer.as_ptr()
            }
        }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        crate::arena_startup::ironwood_bridge_require_executor();
        let mut state = self.lock();
        state.initialized();
        state.check_public_free(pointer as usize, layout);
        // SAFETY: GlobalAlloc's caller supplies the live allocation and its exact
        // current Layout; this wrapper never changes either argument.
        unsafe {
            state
                .heap
                .deallocate(NonNull::new_unchecked(pointer), layout)
        };
        state.counts.deallocations = add(state.counts.deallocations, 1);
        state.counts.live_requested = sub(state.counts.live_requested, layout.size());
        state.counts.live_blocks = sub(state.counts.live_blocks, 1);
    }
    // Deliberately inherit alloc_zeroed and realloc. Their zero/copy runs after
    // alloc releases the guard. Realloc failure retains old ownership; success
    // counts old + new live overlap before deallocating the old block.
}
