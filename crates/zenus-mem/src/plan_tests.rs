//! Tests for `plan_regions` — the seeding of the frame allocator.
//!
//! This is the half of the allocator that `reserve_tests` could not reach.
//! `global_init` reads `rsp` and the linker symbols, so no host test can call
//! it; `plan_regions` is the same decision with everything it consults passed in
//! as values, and it is where the cursors (`next_free`, `low_cursor`) are set.
//! Until it existed, `FrameAllocator::new` was a second copy of the same seeding
//! that only agreed with `global_init` by inspection — so a host test could be
//! green while the boot path was wrong (BUG-038).
//!
//! The assertions are deliberately of two kinds:
//!
//! * the exact resulting region list, hand-computed from the input; and
//! * the set of pages actually handed out, checked against the input's
//!   reservations rather than against the region list.
//!
//! The second kind is the one that cannot be fooled by a list that is right on
//! paper and unreachable in practice.

use crate::frame_allocator::{
    plan_regions, AllocPlanInput, FrameAllocator, MemoryRegion, BOOT_STACK_RESERVE,
};
use alloc::vec::Vec;

fn align_up(a: u64) -> u64 {
    (a + 0xfff) & !0xfff
}

/// The pages `alloc_frame` can serve out of `[base, base + length)`: aligned
/// starts `p` with `p + PAGE <= end`, which is the rule both passes apply.
fn servable(base: u64, length: u64) -> Vec<u64> {
    let end = base + length;
    let mut v = Vec::new();
    let mut p = align_up(base);
    while p + 0x1000 <= end {
        v.push(p);
        p += 0x1000;
    }
    v
}

fn drain(fa: &mut FrameAllocator, cap: usize) -> Vec<u64> {
    let mut out = Vec::new();
    while let Some(f) = fa.alloc_frame() {
        assert!(out.len() < cap, "allocator kept serving past {cap} pages");
        out.push(f.as_u64());
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// Every page the region list says is servable, sorted.
fn reachable(fa: &FrameAllocator) -> Vec<u64> {
    let mut v: Vec<u64> = fa
        .regions()
        .iter()
        .flat_map(|r| servable(r.base, r.length))
        .collect();
    v.sort_unstable();
    v
}

fn region(base: u64, length: u64, kind: u64) -> MemoryRegion {
    MemoryRegion { base, length, kind }
}

/// An allocator planned exactly as `global_init` would plan one.
///
/// `kernel_image` is `(base, end)` — the two values `AllocPlanInput` wants — not
/// `(base, length)`. Getting that wrong silently disables the reservation: `end`
/// lands below `base`, `plan_regions` skips it because there is no image to
/// reserve, and the test then fails on the *reservation*, not on the mistake.
fn planned(
    map: &[MemoryRegion],
    kernel_image: (u64, u64),
    boot_stack_phys: u64,
) -> FrameAllocator {
    let mut fa = FrameAllocator::new(&[]);
    let input = AllocPlanInput {
        memory_map: map,
        kernel_image_base: kernel_image.0,
        kernel_image_end: kernel_image.1,
        boot_stack_phys,
    };
    plan_regions(&mut fa, &input);
    fa
}

/// The whole point of the function: after planning, every page that is handed
/// out is a page that exists and is not reserved, and every such page is handed
/// out. A `Vec` of the exact region list would not catch a reservation that
/// removed the wrong pages; this does.
#[test]
fn plan_keeps_exactly_the_usable_unreserved_pages() {
    // 1 MiB .. 1 GiB of usable memory with a 1 MiB MMIO window inside it, the
    // kernel image at the bottom of it, and the boot stack in the middle.
    const BIG_BASE: u64 = 0x0010_0000;
    const BIG_END: u64 = 0x4000_0000;
    const MMIO: (u64, u64) = (0x0800_0000, 0x0810_0000); // 128 MiB .. 129 MiB
    const KERNEL: (u64, u64) = (0x0010_0000, 0x0030_0000); // 1 MiB .. 3 MiB
    const STACK_PHYS: u64 = 0x2000_0000; // 512 MiB
    const LOW: (u64, u64) = (0x0000_0000, 0x0009_FC00);

    let map = [
        region(LOW.0, LOW.1, 0),
        region(BIG_BASE, BIG_END - BIG_BASE, 0),
        region(MMIO.0, MMIO.1 - MMIO.0, 2), // not usable: MMIO
    ];

    let mut fa = planned(&map, KERNEL, STACK_PHYS);

    // The reservations are applied in `plan_regions`' order: non-usable map
    // entries, then the kernel image, then the boot stack. Each one reshapes the
    // list, so the exact result is worth pinning.
    let stack_base = align_up(STACK_PHYS - BOOT_STACK_RESERVE);
    let stack_end = align_up(stack_base + BOOT_STACK_RESERVE + 0x1000);
    let want = [
        (LOW.0, LOW.1),
        (KERNEL.1, MMIO.0 - KERNEL.1),
        (MMIO.1, stack_base - MMIO.1),
        (stack_end, BIG_END - stack_end),
    ];
    let got: Vec<(u64, u64)> = fa.regions().iter().map(|r| (r.base, r.length)).collect();
    assert_eq!(got, want, "region list after planning");

    // `total_memory` counts what survived, not what the map offered.
    let raw_usable = LOW.1 + (BIG_END - BIG_BASE);
    assert!(
        fa.total_memory() < raw_usable,
        "reservations must reduce total_memory below the raw usable size"
    );
    assert_eq!(
        fa.total_memory(),
        fa.regions().iter().map(|r| r.length).sum::<u64>(),
        "total_memory is the region list's own total"
    );

    // And the pages: nothing reserved, nothing invented.
    let cap = reachable(&fa).len();
    let served = drain(&mut fa, cap);

    for (lo, hi) in [KERNEL, MMIO, (stack_base, stack_end)] {
        assert!(
            !served.iter().any(|&p| p >= lo && p < hi),
            "a page from the reserved range [{lo:#x}, {hi:#x}) was handed out"
        );
    }
    for &p in &served {
        let in_usable = (p >= LOW.0 && p < LOW.0 + LOW.1) || (BIG_BASE..BIG_END).contains(&p);
        assert!(in_usable, "page {p:#x} was served from outside the usable map");
    }

    let want_pages = reachable(&fa);
    assert_eq!(served, want_pages, "the pages served are the region list's");
    assert_eq!(
        served.len(),
        cap,
        "every reachable page must be served (none lost)"
    );
    assert_eq!(
        fa.used_memory(),
        cap as u64 * 0x1000,
        "used_memory counts what was actually served"
    );
}

/// `global_init` raises the floor to 16 MiB when the first usable region
/// straddles it, so the pages below are only reachable through the second pass.
/// That pass is the one BUG-038 was about, and this is the input that reaches it.
#[test]
fn a_first_region_straddling_16_mib_is_split_across_both_passes() {
    // 2 MiB .. 34 MiB: one region, floor lands at 16 MiB inside it.
    const FLOOR: u64 = 0x100_0000;
    const LO: u64 = 0x20_0000;
    const LEN: u64 = 0x0200_0000;
    let map = [region(LO, LEN, 0)];

    let mut fa = planned(&map, (0, 0), 0);
    assert_eq!(
        fa.regions(),
        &[crate::frame_allocator::MemRegion { base: LO, length: LEN }],
        "no reservation applies, so the region is untouched"
    );

    // The first pass starts at the floor, so the very first frame comes from
    // above it — that is what makes the second pass necessary at all.
    let first = fa.alloc_frame().expect("a frame").as_u64();
    assert_eq!(first, FLOOR, "the bump pointer starts at the 16 MiB floor");

    let total = (LEN / 0x1000) as usize;
    let mut served = Vec::new();
    served.push(first);
    served.extend(drain(&mut fa, total - 1));
    served.sort_unstable();
    served.dedup();

    let want = servable(LO, LEN);
    assert_eq!(served.len(), total, "every page of the region, once each");
    assert_eq!(served, want, "the frame set is the whole region");

    // Split by which pass served them, and the split has to be at the floor:
    // stopping the second pass from advancing would lose all of these.
    let below = served.iter().filter(|&&p| p < FLOOR).count();
    assert_eq!(
        below,
        ((FLOOR - LO) / 0x1000) as usize,
        "every page below the floor must come from the second pass"
    );
}

/// Without a straddle the floor is the region's base, and the second pass has
/// nothing to do. Pinned so the 16 MiB rule cannot silently start applying.
#[test]
fn a_first_region_below_16_mib_keeps_its_base_as_the_floor() {
    let map = [region(0x1000, 0x10_000, 0)];
    let mut fa = planned(&map, (0, 0), 0);
    assert_eq!(
        fa.alloc_frame().expect("a frame").as_u64(),
        0x1000,
        "no straddle, so no 16 MiB floor"
    );
}

/// `plan_regions` has to clear the allocator it is given. It is called on the
/// global allocator, which a previous plan — or, on a re-init path, a previous
/// boot's worth of allocations — may have filled with regions, a free stack and
/// cursors. A plan that only appends would inherit all of it, and the damage is
/// silent: an allocator holding a stale cursor serves *fewer* pages, and fewer
/// pages is indistinguishable from a smaller machine.
#[test]
fn planning_clears_whatever_the_allocator_already_held() {
    let mut fa = planned(&[region(0x1000, 0x4_0000, 0)], (0, 0), 0);
    for _ in 0..8 {
        fa.alloc_frame().expect("a frame");
    }
    let freed = fa.alloc_frame().expect("a frame");
    fa.free_frame(freed);
    assert!(fa.used_memory() > 0);
    assert_eq!(fa.free_frames_count(), 1, "the free stack is not empty");
    assert!(fa.alloc_frame().is_some(), "the low cursor has moved");

    // Re-plan with a different map, through the same `&mut`.
    let next = [region(0x2000_0000, 0x10_000, 0)];
    let input = AllocPlanInput {
        memory_map: &next,
        kernel_image_base: 0,
        kernel_image_end: 0,
        boot_stack_phys: 0,
    };
    plan_regions(&mut fa, &input);

    assert_eq!(
        fa.regions(),
        &[crate::frame_allocator::MemRegion { base: 0x2000_0000, length: 0x10_000 }],
        "the old regions must be gone, not appended to"
    );
    assert_eq!(fa.used_memory(), 0, "the counters reset");
    assert_eq!(fa.free_frames_count(), 0, "the free stack reset");
    assert_eq!(
        fa.total_memory(),
        0x10_000,
        "total_memory describes the new plan"
    );
    assert_eq!(
        fa.alloc_frame().expect("a frame").as_u64(),
        0x2000_0000,
        "and the cursors point into the new plan, not the old one"
    );
}

/// The invariant BUG-038 broke: handing out a frame must not change what the
/// region list says exists.
///
/// The old second pass advanced `regions[i].base` in place, so a region's
/// claimed end — `base + length` — grew by a page per allocation below the
/// floor. After enough of them the region described memory that was never in the
/// memory map, and the first pass, which trusts `base + length`, would hand out
/// pages that do not exist. `total_memory` drifted up with it.
///
/// Growth is bounded by the below-floor span, so the old code needed thousands
/// of allocations before it served a phantom page; the list growing at all is
/// the tell, and it is visible after one.
#[test]
fn serving_a_frame_never_changes_the_region_list() {
    let map = [region(0x20_0000, 0x0200_0000, 0)];
    let mut fa = planned(&map, (0, 0), 0);

    let before: Vec<(u64, u64)> = fa.regions().iter().map(|r| (r.base, r.length)).collect();
    let total_before = fa.total_memory();

    // Enough to drain both passes completely: 32 MiB of pages, one each.
    let mut n = 0usize;
    while let Some(_f) = fa.alloc_frame() {
        n += 1;
        assert!(n < 20_000, "runaway allocator");
        let now: Vec<(u64, u64)> = fa.regions().iter().map(|r| (r.base, r.length)).collect();
        assert_eq!(now, before, "the region list changed after {n} allocations");
        assert_eq!(
            fa.total_memory(),
            total_before,
            "total_memory changed after {n} allocations"
        );
    }
    assert_eq!(n, 0x0200_0000 / 0x1000, "and it still served every page once");
}

/// Pop the next `n` frames into `out`. Panics if the allocator runs dry.
fn take(fa: &mut FrameAllocator, n: usize, out: &mut Vec<u64>) {
    for _ in 0..n {
        out.push(fa.alloc_frame().expect("a frame").as_u64());
    }
}

/// Re-planning the same machine has to serve the same pages.
///
/// This is the one that holds `reset`'s clearing of `low_cursor`. After the
/// first drain every cursor is non-zero — the straddler's low half was served up
/// to the floor — so a `reset` that left them behind would bound the low pass at
/// a watermark from a plan that no longer exists, and the second drain would
/// quietly come up short instead of failing.
#[test]
fn planning_again_serves_the_same_pages_it_did_the_first_time() {
    let map = [region(0x20_0000, 0x0200_0000, 0)]; // 2 MiB .. 34 MiB
    let total = (0x0200_0000 / 0x1000) as usize;

    let mut fa = planned(&map, (0, 0), 0);
    let first = drain(&mut fa, total);
    assert_eq!(first.len(), total, "the first plan serves the whole region");
    assert_eq!(first, reachable(&fa));

    let input = AllocPlanInput {
        memory_map: &map,
        kernel_image_base: 0,
        kernel_image_end: 0,
        boot_stack_phys: 0,
    };
    plan_regions(&mut fa, &input);

    let second = drain(&mut fa, total);
    assert_eq!(
        second, first,
        "the same plan must serve the same pages, not fewer"
    );
}

/// A region's low-pass watermark is part of the region, so it has to move down
/// with it when `reserve_region` removes an entry ahead of it.
///
/// The removal branch shifts every later entry down by one and relies on
/// `region_count` having gone down. If the cursor array is not shifted with the
/// region array, the surviving region inherits the *removed* region's cursor —
/// zero, if the removed one was never low-served — and the low pass starts over
/// at the survivor's base, serving pages it already handed out.
///
/// Reaching this needs the low pass to have served something in a *later*
/// region before the removal, which is why it is its own test rather than a case
/// in the table.
#[test]
fn a_reservation_keeps_the_low_cursor_with_its_region() {
    // r0 straddles the 16 MiB floor, which is what makes the low pass run at
    // all; r1 sits entirely below the floor, so the low pass serves it too.
    let map = [
        region(0x20_0000, 0x0200_0000, 0), // 2 MiB .. 34 MiB
        region(0x0, 0x10_0000, 0),         // 0 .. 1 MiB
    ];
    let mut fa = planned(&map, (0, 0), 0);

    let mut served: Vec<u64> = Vec::new();
    // The high pass over r0: [16 MiB, 34 MiB).
    take(&mut fa, 4608, &mut served);
    // The low pass over r0: [2 MiB, 16 MiB).
    take(&mut fa, 3584, &mut served);
    // And its first step into r1, which is what leaves r1's cursor non-zero.
    take(&mut fa, 1, &mut served);
    assert_eq!(*served.last().expect("a frame"), 0x0, "the low pass reached r1");

    // Remove r0 outright. r1 shifts from index 1 to index 0 and must take its
    // cursor with it.
    fa.reserve_region(0x20_0000, 0x0200_0000);
    assert_eq!(fa.region_count(), 1, "only r1 is left");

    while let Some(f) = fa.alloc_frame() {
        served.push(f.as_u64());
        assert!(served.len() < 9000, "runaway allocator");
    }

    let mut sorted = served.clone();
    sorted.sort_unstable();
    let mut uniq = sorted.clone();
    uniq.dedup();
    assert_eq!(
        sorted, uniq,
        "a page was served twice after the reservation moved the cursor"
    );
    assert_eq!(sorted.len(), 8448, "32 MiB + 1 MiB of pages, each once");
}

/// A split inserts a tail at index `i + 1`, which pushes every later region down
/// by one. Their cursors have to be pushed down with them.
///
/// Distinct from the removal case above in what it takes to observe: here the
/// region that moves is the *last* one and it has been low-served, so losing its
/// cursor means the low pass starts over at its base and hands out pages that
/// were already served. The region that moves is below the floor and the split
/// is above it, so neither is a special case of the other.
#[test]
fn a_split_shifts_the_cursor_of_the_regions_after_it() {
    let map = [
        region(0x20_0000, 0x0200_0000, 0), // 2 MiB .. 34 MiB, straddles the floor
        region(0x0, 0x10_0000, 0),         // 0 .. 1 MiB, entirely below it
    ];
    let mut fa = planned(&map, (0, 0), 0);

    let mut served: Vec<u64> = Vec::new();
    take(&mut fa, 4608, &mut served); // the high pass over the straddler
    take(&mut fa, 3584, &mut served); // its whole low half
    take(&mut fa, 1, &mut served); // and the first page of the low region

    // Split the straddler strictly inside itself. The tail lands at index 1 and
    // the low region moves from index 1 to index 2.
    fa.reserve_region(0x80_0000, 0x4_0000); // 8 MiB .. 8.25 MiB
    assert_eq!(fa.region_count(), 3, "the straddler must have split");
    assert_eq!(fa.regions()[1].base, 0x84_0000, "the tail at 8.25 MiB");
    assert_eq!(fa.regions()[2].base, 0x0, "the low region shifted to the end");

    while let Some(f) = fa.alloc_frame() {
        served.push(f.as_u64());
        assert!(served.len() < 9000, "runaway allocator");
    }

    let mut sorted = served.clone();
    sorted.sort_unstable();
    let mut uniq = sorted.clone();
    uniq.dedup();
    assert_eq!(
        sorted, uniq,
        "a cursor was left on the wrong region by the split"
    );
    assert_eq!(sorted.len(), 8192 + 256, "both regions, every page once");
}

/// And a split: the served part of the low half is not confined to the piece
/// that keeps the low address.
///
/// The removal test above needs the *removal* branch; this one needs `split`,
/// and the difference is where the cursor has to end up. A split hands the tail
/// `max(cursor, hole_end)`, and that `max` only matters when the cursor is
/// already past the hole — which means reserving pages the low pass has already
/// handed out. `reserve_region` cannot un-serve them, so 64 of the pages in
/// `served` end up inside the reservation; the thing this test is about is that
/// none of them comes out a second time.
#[test]
fn a_split_does_not_let_the_tail_re_serve_what_the_low_pass_took() {
    let map = [region(0x20_0000, 0x0200_0000, 0)]; // 2 MiB .. 34 MiB, straddles
    let mut fa = planned(&map, (0, 0), 0);

    let mut served: Vec<u64> = Vec::new();
    take(&mut fa, 4608, &mut served); // the high pass: 16 MiB .. 34 MiB
    take(&mut fa, 256, &mut served); // the low pass: 2 MiB .. 3 MiB

    // A hole strictly inside what the low pass has already served, so the tail's
    // base (2.75 MiB) is below the cursor (3 MiB).
    fa.reserve_region(0x28_0000, 0x4_0000); // 2.5 MiB .. 2.75 MiB
    assert_eq!(fa.region_count(), 2, "the region must have split");
    assert_eq!(fa.regions()[0].base, 0x20_0000);
    assert_eq!(fa.regions()[1].base, 0x2C_0000, "2.75 MiB");

    while let Some(f) = fa.alloc_frame() {
        served.push(f.as_u64());
        assert!(served.len() < 9000, "runaway allocator");
    }

    let mut sorted = served.clone();
    sorted.sort_unstable();
    let mut uniq = sorted.clone();
    uniq.dedup();
    assert_eq!(
        sorted, uniq,
        "the tail re-served a page the low pass had already handed out"
    );
    assert_eq!(sorted.len(), 8192, "every page of the original region, once");
}

/// A reservation inside the low half must stay reserved even though the second
/// pass, not the first, is what walks that memory.
///
/// The map is deliberately unsorted — the straddling region first, the low
/// region second — because that is the shape a per-region cursor has to handle
/// and a single monotonic one cannot: with one address watermark the low region
/// would be walked past and never come back. `reserve_region` leaves the list
/// unsorted too, since a split inserts a tail after the entry it came from.
#[test]
fn the_second_pass_does_not_serve_a_reserved_page() {
    const RESERVE: (u64, u64) = (0x4_0000, 0x1_0000); // 256 KiB .. 320 KiB
    const FLOOR: u64 = 0x100_0000;
    let map = [
        region(0x20_0000, 0x0200_0000, 0), // 2 MiB .. 34 MiB, straddles the floor
        region(0x0, 0x10_0000, 0),         // 0 .. 1 MiB, entirely below it
        region(RESERVE.0, RESERVE.1, 2),   // reserved window inside the second
    ];

    let mut fa = planned(&map, (0, 0), 0);
    // Three regions: the straddler, and the low one split by the reservation.
    assert_eq!(fa.region_count(), 3, "the low region must have been split");

    let total = (0x0200_0000 / 0x1000) as usize + (0x10_0000 / 0x1000) as usize
        - (RESERVE.1 / 0x1000) as usize;
    let served = drain(&mut fa, total);

    assert_eq!(served.len(), total, "every unreserved page, and only those");
    assert!(
        !served.iter().any(|&p| p >= RESERVE.0 && p < RESERVE.0 + RESERVE.1),
        "a page from the reserved window was served"
    );
    assert!(
        served.iter().any(|&p| p < RESERVE.0),
        "the pages below the reserved window must still be served"
    );
    assert!(
        served.iter().any(|&p| p < FLOOR),
        "the below-floor half of the straddler still has to be served"
    );
}

/// The kernel image is not in the memory map as a non-usable entry — the
/// bootloader leaves it out entirely — so nothing but the explicit reservation
/// excludes it. A frame overlapping the image is a write that lands somewhere
/// else, which is what `make test` died of.
#[test]
fn the_kernel_image_is_never_handed_out() {
    const KERNEL: (u64, u64) = (0x1000_0000, 0x1020_0000); // 256 MiB .. 258 MiB
    // A region that comfortably straddles both 16 MiB and the image.
    let map = [region(0x0010_0000, 0x3000_0000, 0)];

    let mut fa = planned(&map, KERNEL, 0);
    // The image splits the region, so the list must show the hole.
    let got: Vec<(u64, u64)> = fa.regions().iter().map(|r| (r.base, r.length)).collect();
    assert_eq!(
        got,
        [
            (0x0010_0000, KERNEL.0 - 0x0010_0000),
            (KERNEL.1, 0x3010_0000 - KERNEL.1),
        ],
        "the kernel image must leave a hole in the region list"
    );

    let served = drain(&mut fa, 0x30_000);
    assert!(
        !served.iter().any(|&p| p >= KERNEL.0 && p < KERNEL.1),
        "a page inside the kernel image was served"
    );
}
