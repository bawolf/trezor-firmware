//! The two signing arenas, as mechanics: no statics, no `GlobalAlloc`, no
//! target. `allocator.rs` owns one [`Arenas`] and hands it the device's
//! sections; the host unit tests below drive the same code over ordinary
//! buffers, which is the only place this logic is exercised at all —
//! `allocator_unix.rs` is plain `malloc`, so no emulator run reaches it.
//!
//! Each arena is a first-fit free list of blocks, every one a multiple of
//! [`UNIT`] bytes and prefixed by a [`HEADER`]-byte header carrying its total
//! length and a free flag. Payloads are 16-byte aligned because the base is and
//! every block length is. Allocation splits a block when the remainder can hold
//! one; `realloc` resizes in place when the block, or the free block after it,
//! has room. `free` zeroes the payload, marks the block free and sweeps the
//! whole arena once, merging every maximal run of adjacent free blocks. The
//! sweep starts at the base, so a freed block merges with a free predecessor as
//! well as a free successor — backward coalescing with no per-block footer —
//! and no two adjacent free blocks ever remain, which makes first-fit
//! order-independent.
//!
//! A malformed header (zero length, off the UNIT grid, or running past the
//! end) can only come from a wild write by another subsystem. Every walk stops
//! on one instead of looping or stepping outside the arena, so corruption
//! surfaces as a null from `alloc` — a clean fatal exit — and never spreads.

use core::ptr;

/// Bytes of block header: the block's total length, then its free flag.
pub const HEADER: usize = 16;
/// Allocation grain. Also the largest alignment an arena can serve.
pub const UNIT: usize = 16;

/// Bytes a request of `size` occupies, header included.
pub const fn block_size(size: usize) -> usize {
    let size = if size == 0 { 1 } else { size };
    (HEADER + size + UNIT - 1) / UNIT * UNIT
}

/// One arena: a formatted span plus the bytes currently handed out of it.
#[derive(Clone, Copy)]
struct Span {
    base: *mut u8,
    len: usize,
    in_use: usize,
}

impl Span {
    const EMPTY: Self = Self {
        base: ptr::null_mut(),
        len: 0,
        in_use: 0,
    };

    fn live(&self) -> bool {
        !self.base.is_null()
    }

    /// True if `block` is a header address inside this span.
    fn holds(&self, block: *mut u8) -> bool {
        // SAFETY: the comparison stays within the span's own allocation.
        self.live() && block >= self.base && block < unsafe { self.base.add(self.len) }
    }
}

unsafe fn block_len(block: *mut u8) -> usize {
    unsafe { ptr::read(block.cast::<u32>()) as usize }
}
unsafe fn block_free(block: *mut u8) -> bool {
    unsafe { ptr::read(block.add(4).cast::<u32>()) != 0 }
}
unsafe fn set_block(block: *mut u8, len: usize, free: bool) {
    unsafe {
        ptr::write(block.cast::<u32>(), len as u32);
        ptr::write(block.add(4).cast::<u32>(), free as u32);
    }
}

/// Whether a header describes a well-formed block that stays inside
/// `[block, end)`.
unsafe fn block_ok(block: *mut u8, blen: usize, end: *mut u8) -> bool {
    blen != 0 && blen % UNIT == 0 && unsafe { block.add(blen) } <= end
}

/// Merges every maximal run of adjacent free blocks into one. O(n) in blocks.
unsafe fn coalesce(base: *mut u8, len: usize) {
    unsafe {
        let end = base.add(len);
        let mut block = base;
        while block < end {
            let blen = block_len(block);
            if !block_ok(block, blen, end) {
                break;
            }
            if block_free(block) {
                let mut total = blen;
                let mut next = block.add(total);
                while next < end && block_free(next) {
                    let nlen = block_len(next);
                    if !block_ok(next, nlen, end) {
                        break;
                    }
                    total += nlen;
                    next = block.add(total);
                }
                if total != blen {
                    set_block(block, total, true);
                }
                block = block.add(total);
            } else {
                block = block.add(blen);
            }
        }
    }
}

/// Sum of every in-use block's total length, by one walk of the chain.
unsafe fn walk_in_use(base: *mut u8, len: usize) -> usize {
    unsafe {
        if base.is_null() {
            return 0;
        }
        let end = base.add(len);
        let mut block = base;
        let mut total = 0;
        while block < end {
            let blen = block_len(block);
            if !block_ok(block, blen, end) {
                break;
            }
            if !block_free(block) {
                total += blen;
            }
            block = block.add(blen);
        }
        total
    }
}

/// The rooted tier and, while a session is live, its scratch tier.
///
/// Routing is mechanical and needs nothing from the caller: `alloc` takes the
/// scratch if one is installed and the rooted tier otherwise — so everything
/// allocated outside a session, including any lazy static a key derivation
/// fills, is rooted by construction — and `free` routes by the pointer's
/// address, so no crate has to know where its memory came from. The two never
/// fall back to each other: a scratch exhaustion must fail closed rather than
/// quietly eat the persistent set.
pub struct Arenas {
    persist: Span,
    scratch: Span,
}

impl Arenas {
    pub const fn new() -> Self {
        Self {
            persist: Span::EMPTY,
            scratch: Span::EMPTY,
        }
    }

    /// Formats the rooted tier on the first call and does nothing thereafter.
    /// Re-formatting would orphan Pasta's square-root table and orchard's
    /// commitment-domain caches, which are reached through Rust statics the
    /// MicroPython collector never scans.
    ///
    /// # Safety
    ///
    /// `base` must be 16-byte aligned, `len` a multiple of [`UNIT`], and the
    /// span must be valid and unaliased for the rest of the boot.
    pub unsafe fn root(&mut self, base: *mut u8, len: usize) {
        if self.persist.live() || base.is_null() || len < HEADER + UNIT {
            return;
        }
        unsafe { set_block(base, len, true) };
        self.persist = Span {
            base,
            len,
            in_use: 0,
        };
    }

    /// Lends a span to the allocator as this session's scratch tier and routes
    /// allocation to it. Refuses a second install, and any span that does not
    /// hold `least` bytes once aligned.
    ///
    /// # Safety
    ///
    /// The span must stay valid, unmoved and unaliased until
    /// [`Arenas::release_scratch`].
    pub unsafe fn install_scratch(&mut self, base: *mut u8, len: usize, least: usize) -> bool {
        if self.scratch.live() || base.is_null() {
            return false;
        }
        let head = base.align_offset(UNIT);
        if head > len {
            return false;
        }
        // SAFETY: `head <= len`, so this stays inside the caller's span.
        let base = unsafe { base.add(head) };
        let len = (len - head) / UNIT * UNIT;
        if len < least.max(HEADER + UNIT) {
            return false;
        }
        unsafe { set_block(base, len, true) };
        self.scratch = Span {
            base,
            len,
            in_use: 0,
        };
        true
    }

    /// Takes the scratch tier back and routes allocation to the rooted tier
    /// again, zeroing the span first: those bytes go straight back to the
    /// MicroPython collector, which will hand them to arbitrary objects.
    ///
    /// `Err(bytes)` means a block was still in use — a Rust static first filled
    /// during the session, which would dangle the moment the buffer is
    /// collected. That is the inverse of the cross-session bug the rooted tier
    /// exists to prevent, and the caller must treat it as fatal. The span is
    /// wiped either way, before the verdict is returned.
    pub fn release_scratch(&mut self) -> Result<(), usize> {
        let span = self.scratch;
        if !span.live() {
            return Ok(());
        }
        // Uninstall first, so nothing the caller's failure path allocates is
        // carved from a tier that is going away.
        self.scratch = Span::EMPTY;
        // The counter and the chain must agree; either one non-zero is a
        // retained block.
        // SAFETY: the span was installed by `install_scratch` and its owner
        // has not dropped it yet.
        let retained = span.in_use.max(unsafe { walk_in_use(span.base, span.len) });
        // Wipe before reporting, not after deciding. A retained block is the
        // one thing in the span `dealloc` has not already zeroed, and it is
        // by definition live secret-class state, so leaving it for the
        // caller's failure path would make the residue depend on which fatal
        // continuation runs. Invalidating the retained pointer is sound only
        // because the caller of `Err` does not return: `allocator::
        // release_scratch` turns it into `system_exit_fatal`.
        // SAFETY: the span was installed by `install_scratch` and is ours
        // until this returns.
        unsafe {
            ptr::write_bytes(span.base, 0, span.len);
        }
        core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
        if retained != 0 {
            return Err(retained);
        }
        Ok(())
    }

    /// Whether a scratch tier is currently lent to the allocator.
    ///
    /// `session_begin` asks before it touches anything: a scratch that is
    /// still installed when a new session starts means the workflow that
    /// installed it was dropped without running its `finally`, so the
    /// `bytearray` behind the span may already have been collected and
    /// handed to something else.
    pub fn scratch_installed(&self) -> bool {
        self.scratch.live()
    }

    /// Bytes handed out of each tier, as `(persist, scratch)`. O(1).
    pub fn in_use(&self) -> (usize, usize) {
        (self.persist.in_use, self.scratch.in_use)
    }

    /// The same figures read off the block chains. O(n), and equal to
    /// [`Arenas::in_use`] unless a header has been overwritten.
    pub fn audit(&self) -> (usize, usize) {
        // SAFETY: single-threaded walk over spans this type formatted.
        unsafe {
            (
                walk_in_use(self.persist.base, self.persist.len),
                walk_in_use(self.scratch.base, self.scratch.len),
            )
        }
    }

    /// Carves `size` bytes from the live tier, or null when the tier is
    /// exhausted, absent, or too coarse for `align`.
    ///
    /// # Safety
    ///
    /// The installed spans must still be valid.
    pub unsafe fn alloc(&mut self, size: usize, align: usize) -> *mut u8 {
        if align > UNIT {
            return ptr::null_mut();
        }
        let need = block_size(size);
        let span = if self.scratch.live() {
            &mut self.scratch
        } else {
            &mut self.persist
        };
        if !span.live() {
            return ptr::null_mut();
        }
        // SAFETY: the walk stays inside the span, stopping on a malformed header.
        unsafe {
            let end = span.base.add(span.len);
            let mut block = span.base;
            while block < end {
                let blen = block_len(block);
                if !block_ok(block, blen, end) {
                    return ptr::null_mut();
                }
                if block_free(block) && blen >= need {
                    // Split only when the remainder can hold a block of its
                    // own; otherwise the request keeps the slack.
                    let taken = if blen - need >= UNIT + HEADER {
                        set_block(block.add(need), blen - need, true);
                        set_block(block, need, false);
                        need
                    } else {
                        set_block(block, blen, false);
                        blen
                    };
                    span.in_use += taken;
                    return block.add(HEADER);
                }
                block = block.add(blen);
            }
        }
        ptr::null_mut()
    }

    /// Resizes `payload` to `new_size`, in place when the block allows it and
    /// by allocate-copy-free otherwise; null when it can do neither.
    ///
    /// In place is what keeps a growing `Vec` from fragmenting the arena.
    /// Pasta builds its square-root table by collecting four 256-element
    /// `Vec<Fp>` from iterators with no size hint, so each doubles its way up
    /// to 8 KB; moving on every doubling left a hole the size of each
    /// earlier buffer in front of every table, and the rooted tier refused
    /// the last 8 KB with room to spare in total.
    ///
    /// # Safety
    ///
    /// `payload` must be a live pointer [`Arenas::alloc`] returned for
    /// `old_size` bytes, and `align` the alignment it was requested with.
    pub unsafe fn realloc(
        &mut self,
        payload: *mut u8,
        old_size: usize,
        new_size: usize,
        align: usize,
    ) -> *mut u8 {
        // SAFETY: forwarded from the caller.
        if unsafe { self.resize_in_place(payload, new_size) } {
            return payload;
        }
        // SAFETY: as above; the new block is distinct from the old one.
        unsafe {
            let moved = self.alloc(new_size, align);
            if !moved.is_null() {
                ptr::copy_nonoverlapping(payload, moved, old_size.min(new_size));
                self.dealloc(payload);
            }
            moved
        }
    }

    /// Fits `payload`'s block to `new_size` without moving it: a shrink, or a
    /// growth into the free block right after it. Only in the tier `alloc`
    /// would use now, so a session never spends the rooted tier by growing a
    /// rooted block.
    unsafe fn resize_in_place(&mut self, payload: *mut u8, new_size: usize) -> bool {
        // SAFETY: the header sits HEADER bytes before any payload we handed out.
        let block = unsafe { payload.sub(HEADER) };
        let span = if self.scratch.live() {
            &mut self.scratch
        } else {
            &mut self.persist
        };
        if !span.holds(block) {
            return false;
        }
        let need = block_size(new_size);
        // SAFETY: the block is inside the span; every header read is checked
        // before it is followed.
        unsafe {
            let end = span.base.add(span.len);
            let blen = block_len(block);
            if !block_ok(block, blen, end) || block_free(block) {
                return false;
            }
            let mut total = blen;
            if need > blen {
                let next = block.add(blen);
                if next >= end || !block_free(next) {
                    return false;
                }
                let nlen = block_len(next);
                if !block_ok(next, nlen, end) || blen + nlen < need {
                    return false;
                }
                // The absorbed header becomes payload.
                ptr::write_bytes(next, 0, HEADER);
                total = blen + nlen;
            }
            // Split the remainder off as `alloc` does. It is wiped first: on a
            // shrink it held the tail of the old payload.
            let taken = if total - need >= UNIT + HEADER {
                let tail = block.add(need);
                ptr::write_bytes(tail, 0, total - need);
                core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
                set_block(tail, total - need, true);
                set_block(block, need, false);
                need
            } else {
                set_block(block, total, false);
                total
            };
            span.in_use = span.in_use - blen + taken;
            if taken < total {
                coalesce(span.base, span.len);
            }
        }
        true
    }

    /// Returns `payload`'s block to whichever tier it came from. A pointer into
    /// neither — a block from a scratch that is already released — is dropped
    /// rather than written through.
    ///
    /// # Safety
    ///
    /// `payload` must be a pointer [`Arenas::alloc`] returned and not yet
    /// freed.
    pub unsafe fn dealloc(&mut self, payload: *mut u8) {
        // SAFETY: the header sits HEADER bytes before any payload we handed out.
        let block = unsafe { payload.sub(HEADER) };
        let span = if self.persist.holds(block) {
            &mut self.persist
        } else if self.scratch.holds(block) {
            &mut self.scratch
        } else {
            return;
        };
        // SAFETY: the block is inside the span, so its header is ours to read.
        unsafe {
            let blen = block_len(block);
            if !block_ok(block, blen, span.base.add(span.len)) || block_free(block) {
                return;
            }
            // Neither tier is recycled by GC churn, so freed secret-class
            // scratch (the sinsemilla `padded: Vec<bool>` holding ak||nk bits,
            // say) would linger until the same offsets are reused. Zero the
            // payload before the block returns to the free list; the fence
            // keeps the store from being elided as dead.
            ptr::write_bytes(payload, 0, blen - HEADER);
            core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
            set_block(block, blen, true);
            span.in_use -= blen;
            coalesce(span.base, span.len);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROOTED: usize = 4 * 1024;
    const SCRATCH: usize = 2 * 1024;

    /// A 16-byte aligned host buffer standing in for a linker section.
    struct Buffer(Vec<u128>);

    impl Buffer {
        fn new(bytes: usize) -> Self {
            Self(vec![0; bytes / 16])
        }
        fn base(&mut self) -> *mut u8 {
            self.0.as_mut_ptr().cast()
        }
        fn len(&self) -> usize {
            self.0.len() * 16
        }
    }

    /// A rooted tier with nothing lent to it yet.
    fn rooted(region: &mut Buffer) -> Arenas {
        let mut arenas = Arenas::new();
        let (base, len) = (region.base(), region.len());
        unsafe { arenas.root(base, len) };
        arenas
    }

    #[test]
    fn a_request_outside_a_session_is_rooted_and_one_inside_it_is_not() {
        let mut region = Buffer::new(ROOTED);
        let mut buffer = Buffer::new(SCRATCH);
        let mut arenas = rooted(&mut region);

        let warm = unsafe { arenas.alloc(100, 8) };
        assert!(!warm.is_null());
        assert_eq!(arenas.in_use(), (block_size(100), 0));

        let (base, len) = (buffer.base(), buffer.len());
        assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });
        let session = unsafe { arenas.alloc(200, 16) };
        assert!(!session.is_null());
        // The persistent figure did not move: the session's block is not in it.
        assert_eq!(arenas.in_use(), (block_size(100), block_size(200)));
        assert_eq!(arenas.in_use(), arenas.audit());

        // Freeing routes by address, not by which tier is live.
        unsafe { arenas.dealloc(warm) };
        assert_eq!(arenas.in_use(), (0, block_size(200)));
        unsafe { arenas.dealloc(session) };
        assert_eq!(arenas.in_use(), (0, 0));
        assert!(arenas.release_scratch().is_ok());
    }

    #[test]
    fn a_block_left_in_the_scratch_is_reported_rather_than_dangled() {
        let mut region = Buffer::new(ROOTED);
        let mut buffer = Buffer::new(SCRATCH);
        let mut arenas = rooted(&mut region);
        let (base, len) = (buffer.base(), buffer.len());
        assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });

        // A Rust static filled during the session: allocated in the scratch,
        // never freed, and reached afterwards through a pointer the collector
        // cannot see.
        let leaked = unsafe { arenas.alloc(64, 8) };
        assert!(!leaked.is_null());
        unsafe { ptr::write_bytes(leaked, 0xA5, 64) };
        assert!(arenas.scratch_installed());
        assert_eq!(arenas.release_scratch(), Err(block_size(64)));
        // Reported AND wiped. The caller is fatal, so nothing reads the block
        // again; what this rules out is the secret surviving in the buffer the
        // collector is about to hand to arbitrary Python objects, on whichever
        // continuation the fatal path takes.
        assert!(buffer.0.iter().all(|word| *word == 0));
        // The tier is uninstalled either way, so a later free cannot write
        // through a buffer that has gone back to the collector.
        assert!(!arenas.scratch_installed());
        assert_eq!(arenas.in_use(), (0, 0));
        unsafe { arenas.dealloc(leaked) };
    }

    /// Walks a span's block chain and returns every header address, failing
    /// on anything that is off the grid or runs past the end.
    fn chain(base: *mut u8, len: usize) -> Vec<*mut u8> {
        let end = unsafe { base.add(len) };
        let mut blocks = Vec::new();
        let mut block = base;
        while block < end {
            assert_eq!(
                block as usize % UNIT,
                0,
                "a block header left the {UNIT}-byte grid"
            );
            let blen = unsafe { block_len(block) };
            assert!(
                unsafe { block_ok(block, blen, end) },
                "a block of {blen} B at {block:?} runs past the span"
            );
            // The header is two u32s at `block` and `block + 4`; both are
            // inside the block because the smallest block is UNIT.
            assert!(blen >= UNIT);
            blocks.push(block);
            block = unsafe { block.add(blen) };
        }
        assert_eq!(block, end, "the chain did not land exactly on the end");
        blocks
    }

    /// Every pointer the arena hands out is aligned for what was asked, the
    /// headers stay on the grid, and the chain always tiles the span exactly.
    ///
    /// On the device an unaligned payload is not a slow read, it is a
    /// UsageFault (`UNALIGN_TRP`), and it would be the same offset on every
    /// allocation of the boot -- so it is worth asserting here, where the
    /// mechanism is, rather than discovering it as an address on a screen.
    #[test]
    fn every_payload_is_aligned_and_every_header_stays_on_the_grid() {
        // Room for the whole sweep at once, so every block below is live
        // together and the chain is walked over a genuinely fragmented span.
        let mut region = Buffer::new(16 * 1024);
        let mut arenas = rooted(&mut region);
        let (base, len) = (region.base(), region.len());

        let mut live = Vec::new();
        // Sizes that straddle the grid in both directions, against every
        // alignment the arena promises to serve.
        for size in [0usize, 1, 15, 16, 17, 31, 32, 33, 48, 100, 255, 256] {
            for align in [1usize, 2, 4, 8, 16] {
                let payload = unsafe { arenas.alloc(size, align) };
                assert!(!payload.is_null(), "{size}/{align} did not fit");
                assert_eq!(
                    payload as usize % align,
                    0,
                    "{size}/{align}: payload is not aligned for the request"
                );
                // Stronger than the request: the arena's contract is that
                // every payload is UNIT-aligned, which is what lets it refuse
                // `align > UNIT` outright instead of over-allocating.
                assert_eq!(payload as usize % UNIT, 0, "{size}/{align}");
                // And the payload is where the header says it is.
                // SAFETY: `payload` came from this span, so its header and
                // the span's end are both addresses inside it.
                unsafe {
                    let block = payload.sub(HEADER);
                    assert!(block >= base && block.add(block_len(block)) <= base.add(len));
                }
                live.push(payload);
                chain(base, len);
            }
        }
        // `realloc` hands out pointers too -- in place, or moved -- and must
        // keep the same promise. Grow every live block, then shrink it back,
        // walking the chain after each.
        let sizes = [0usize, 1, 15, 16, 17, 31, 32, 33, 48, 100, 255, 256];
        for (i, payload) in live.iter_mut().enumerate() {
            let size = sizes[i / 5];
            let align = [1usize, 2, 4, 8, 16][i % 5];
            for new_size in [size + 40, size] {
                let old_size = if new_size == size { size + 40 } else { size };
                let resized = unsafe { arenas.realloc(*payload, old_size, new_size, align) };
                assert!(!resized.is_null(), "{size}/{align} -> {new_size}");
                assert_eq!(resized as usize % UNIT, 0, "{size}/{align} -> {new_size}");
                *payload = resized;
                chain(base, len);
            }
        }
        // Alignments the 16-byte grid cannot promise are refused, not
        // mis-served by handing back a 16-aligned pointer anyway.
        for align in [32usize, 64, 4096] {
            assert!(unsafe { arenas.alloc(16, align) }.is_null(), "{align}");
        }
        for payload in live {
            unsafe { arenas.dealloc(payload) };
            chain(base, len);
        }
        assert_eq!(arenas.in_use(), (0, 0));
        // Fully coalesced back to the single block `root` formatted.
        assert_eq!(chain(base, len).len(), 1);
    }

    #[test]
    fn a_scratch_is_reported_installed_for_exactly_its_session() {
        let mut region = Buffer::new(ROOTED);
        let mut buffer = Buffer::new(SCRATCH);
        let mut arenas = rooted(&mut region);
        // `session_begin` reads this before it touches the previous session's
        // memory: true at entry means a workflow was dropped without running
        // its `finally`, and the span may no longer be the caller's to write.
        assert!(!arenas.scratch_installed());
        let (base, len) = (buffer.base(), buffer.len());
        assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });
        assert!(arenas.scratch_installed());
        assert!(arenas.release_scratch().is_ok());
        assert!(!arenas.scratch_installed());
    }

    #[test]
    fn a_second_session_starts_from_the_same_rooted_tier() {
        let mut region = Buffer::new(ROOTED);
        let mut first = Buffer::new(SCRATCH);
        let mut second = Buffer::new(SCRATCH);
        let mut arenas = rooted(&mut region);
        let warm = unsafe { arenas.alloc(1_000, 16) };
        let persistent = arenas.in_use().0;

        for buffer in [&mut first, &mut second] {
            let (base, len) = (buffer.base(), buffer.len());
            assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });
            let mut blocks = Vec::new();
            for size in [16, 300, 48, 900] {
                let block = unsafe { arenas.alloc(size, 16) };
                assert!(!block.is_null(), "{size} did not fit the scratch");
                blocks.push(block);
            }
            // Out of order, so the coalescing sweep has something to merge.
            for block in [blocks[1], blocks[3], blocks[0], blocks[2]] {
                unsafe { arenas.dealloc(block) };
            }
            assert!(arenas.release_scratch().is_ok());
            // The rooted tier is untouched by a session, twice over. This is
            // the invariant the cross-session bug broke.
            assert_eq!(arenas.in_use().0, persistent);
        }

        // And the boot-lifetime block is still readable and still its own.
        unsafe { arenas.dealloc(warm) };
        assert_eq!(arenas.in_use(), (0, 0));
    }

    #[test]
    fn a_full_scratch_fails_closed_instead_of_reaching_the_rooted_tier() {
        let mut region = Buffer::new(ROOTED);
        let mut buffer = Buffer::new(SCRATCH);
        let mut arenas = rooted(&mut region);
        let (base, len) = (buffer.base(), buffer.len());
        assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });

        let mut blocks = Vec::new();
        loop {
            let block = unsafe { arenas.alloc(128, 16) };
            if block.is_null() {
                break;
            }
            blocks.push(block);
        }
        // Exhausted, and not one byte of it came from the tier the persistent
        // set lives in.
        assert!(arenas.in_use().1 + block_size(128) > SCRATCH);
        assert_eq!(arenas.in_use().0, 0);
        // A request larger than the whole scratch is refused too, rather than
        // falling back.
        assert!(unsafe { arenas.alloc(SCRATCH, 16) }.is_null());

        for block in blocks {
            unsafe { arenas.dealloc(block) };
        }
        assert!(arenas.release_scratch().is_ok());
    }

    #[test]
    fn freed_blocks_coalesce_in_both_directions_and_are_wiped() {
        let mut region = Buffer::new(ROOTED);
        let mut arenas = rooted(&mut region);

        let mut blocks = Vec::new();
        for _ in 0..3 {
            blocks.push(unsafe { arenas.alloc(240, 16) });
        }
        let secret = [0xa5u8; 240];
        unsafe { ptr::copy_nonoverlapping(secret.as_ptr(), blocks[1], secret.len()) };

        // Free the ends first, then the middle: the middle's free must merge
        // with its predecessor as well as its successor.
        unsafe { arenas.dealloc(blocks[0]) };
        unsafe { arenas.dealloc(blocks[2]) };
        unsafe { arenas.dealloc(blocks[1]) };
        assert_eq!(arenas.in_use(), (0, 0));
        // The payload is gone, not merely unlinked.
        let wiped = unsafe { core::slice::from_raw_parts(blocks[1], secret.len()) };
        assert!(wiped.iter().all(|byte| *byte == 0));

        // Fully merged: the whole arena is available again as one block.
        let whole = unsafe { arenas.alloc(ROOTED - HEADER, 16) };
        assert!(!whole.is_null());
        unsafe { arenas.dealloc(whole) };
    }

    #[test]
    fn a_growing_block_extends_in_place_and_a_shrinking_one_wipes_its_tail() {
        let mut region = Buffer::new(ROOTED);
        let mut arenas = rooted(&mut region);

        let grown = unsafe { arenas.alloc(64, 16) };
        let secret = [0xa5u8; 64];
        unsafe { ptr::copy_nonoverlapping(secret.as_ptr(), grown, secret.len()) };
        let wider = unsafe { arenas.realloc(grown, 64, 1_024, 16) };
        assert_eq!(wider, grown, "the free space after the block was not used");
        assert_eq!(arenas.in_use(), (block_size(1_024), 0));
        assert_eq!(arenas.in_use(), arenas.audit());
        let kept = unsafe { core::slice::from_raw_parts(wider, secret.len()) };
        assert_eq!(kept, secret);

        let narrower = unsafe { arenas.realloc(wider, 1_024, 32, 16) };
        assert_eq!(narrower, wider);
        assert_eq!(arenas.in_use(), (block_size(32), 0));
        assert_eq!(arenas.in_use(), arenas.audit());
        // What the shrink gave back holds none of the old payload: past the
        // new free block's header, the rest of the secret is gone.
        let tail = unsafe { core::slice::from_raw_parts(narrower.add(32 + HEADER), 16) };
        assert!(tail.iter().all(|byte| *byte == 0));
        // And it merged with the free space after it: the rest of the arena is
        // one block again.
        let rest = unsafe { arenas.alloc(ROOTED - block_size(32) - HEADER, 16) };
        assert!(!rest.is_null());
        unsafe { arenas.dealloc(rest) };
        unsafe { arenas.dealloc(narrower) };
        assert_eq!(arenas.in_use(), (0, 0));
    }

    #[test]
    fn a_block_that_cannot_grow_in_place_moves_and_frees_the_old_one() {
        let mut region = Buffer::new(ROOTED);
        let mut arenas = rooted(&mut region);

        let first = unsafe { arenas.alloc(64, 16) };
        let wall = unsafe { arenas.alloc(64, 16) };
        let secret = [0x5au8; 64];
        unsafe { ptr::copy_nonoverlapping(secret.as_ptr(), first, secret.len()) };
        let moved = unsafe { arenas.realloc(first, 64, 512, 16) };
        assert!(!moved.is_null());
        assert_ne!(moved, first);
        assert_eq!(unsafe { core::slice::from_raw_parts(moved, 64) }, secret);
        assert_eq!(arenas.in_use(), (block_size(64) + block_size(512), 0));
        // The old block went back wiped.
        let old = unsafe { core::slice::from_raw_parts(first, 64) };
        assert!(old.iter().all(|byte| *byte == 0));

        unsafe { arenas.dealloc(wall) };
        unsafe { arenas.dealloc(moved) };
        assert_eq!(arenas.in_use(), (0, 0));
    }

    #[test]
    fn a_session_does_not_grow_a_rooted_block_into_the_rooted_tier() {
        let mut region = Buffer::new(ROOTED);
        let mut buffer = Buffer::new(SCRATCH);
        let mut arenas = rooted(&mut region);
        let rooted_block = unsafe { arenas.alloc(64, 16) };

        let (base, len) = (buffer.base(), buffer.len());
        assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });
        let grown = unsafe { arenas.realloc(rooted_block, 64, 256, 16) };
        // Moved into the scratch, as any allocation during a session is; the
        // rooted tier gave nothing.
        assert_ne!(grown, rooted_block);
        assert_eq!(arenas.in_use(), (0, block_size(256)));
        unsafe { arenas.dealloc(grown) };
        assert!(arenas.release_scratch().is_ok());
    }

    #[test]
    fn the_arenas_refuse_what_they_cannot_serve() {
        let mut region = Buffer::new(ROOTED);
        let mut buffer = Buffer::new(SCRATCH);
        let mut arenas = Arenas::new();

        // Nothing is rooted yet, so there is nowhere to allocate from.
        assert!(unsafe { arenas.alloc(16, 16) }.is_null());

        let (base, len) = (region.base(), region.len());
        unsafe { arenas.root(base, len) };
        // Rooting is once per boot: a second call must not re-format the tier
        // the persistent set lives in.
        let warm = unsafe { arenas.alloc(64, 16) };
        unsafe { arenas.root(base, len) };
        assert_eq!(arenas.in_use().0, block_size(64));
        assert_eq!(arenas.audit().0, block_size(64));

        // An alignment the 16-byte grid cannot promise.
        assert!(unsafe { arenas.alloc(16, 32) }.is_null());

        // A scratch below the required size, then a second install.
        let (base, len) = (buffer.base(), buffer.len());
        assert!(!unsafe { arenas.install_scratch(base, len, SCRATCH * 2) });
        assert!(unsafe { arenas.install_scratch(base, len, SCRATCH) });
        assert!(!unsafe { arenas.install_scratch(base, len, SCRATCH) });

        assert!(arenas.release_scratch().is_ok());
        // Releasing twice is a no-op, not a second wipe of memory that is no
        // longer ours.
        assert!(arenas.release_scratch().is_ok());
        unsafe { arenas.dealloc(warm) };
    }
}
