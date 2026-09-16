//! Explicit initialization of immutable public field constants, before any keys.
use crate::{
    arena::{ALLOCATOR, Counts},
    require,
};
use core::alloc::Layout;
use ff::Field;
use linked_list_allocator::hole::HoleList;
use pasta_curves::Fp;

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct LayoutFacts {
    pub size: usize,
    pub alignment: usize,
    pub rounded: usize,
}

#[repr(C)]
#[derive(Default)]
pub struct InitReport {
    pub before: Counts,
    pub after: Counts,
    pub layouts: [LayoutFacts; 5],
}

/// Only FP_TABLES: fp.rs sqrt -> FP_TABLES.sqrt_alt, initialized solely from
/// ROOT_OF_UNITY, ONE and public hasher parameters. No key/PCZT/request warmup.
pub fn initialize() -> InitReport {
    let before = ALLOCATOR.snapshot();
    let previous = ALLOCATOR.public_baseline();
    if let Some(baseline) = previous {
        ALLOCATOR.require_baseline(baseline);
    } else {
        ALLOCATOR.require_empty();
        ALLOCATOR.start_public_init();
    }

    let root = Fp::ONE.sqrt().unwrap();
    require(root.square() == Fp::ONE);
    let after = ALLOCATOR.snapshot();
    let expected = [
        Layout::array::<u8>(1098).unwrap(),
        Layout::array::<Fp>(256).unwrap(),
        Layout::array::<Fp>(256).unwrap(),
        Layout::array::<Fp>(256).unwrap(),
        Layout::array::<Fp>(129).unwrap(),
    ];
    let layouts = expected.map(|layout| LayoutFacts {
        size: layout.size(),
        alignment: layout.align(),
        rounded: HoleList::align_layout(layout).unwrap().size(),
    });
    let requested: usize = layouts.iter().map(|layout| layout.size).sum();
    let used: usize = layouts.iter().map(|layout| layout.rounded).sum();
    require(after.live_blocks == 5 && after.live_requested == requested && after.used == used);
    require(after.failures == before.failures);
    if previous.is_some() {
        require(before == after); // Repeated public initialization allocates/frees nothing.
    } else {
        require(
            after.allocations - before.allocations
                == after.deallocations - before.deallocations + 5,
        );
        ALLOCATOR.seal_public_baseline(after, expected);
    }
    InitReport {
        before,
        after,
        layouts,
    }
}
