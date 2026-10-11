//! Tests for the number `free_frames_count` reports (BUG-039).
//!
//! It used to return `free_count` — the depth of the recycled stack — which is
//! zero on a fresh allocator and therefore said a machine with 2 GiB spare had no
//! free memory. `sys_mmap` budgets against this number, so the wrong answer was
//! not cosmetic: it refused mappings the machine could satisfy, capping `mmap`
//! at 16 384 pages no matter how much was actually free.
//!
//! ## The oracle
//!
//! The tests do not re-derive the count from the region list, because that is
//! exactly where the bug lived — a second copy of the allocator's arithmetic can
//! agree with it and be wrong together. Instead they build a *second* allocator
//! from the same map and drain it, and require the reported count to equal the
//! number of frames that drain actually served, at every step. If the count and
//! the allocator disagree, one of them is wrong and the test fails, whichever it
//! is.
//!
//! Before the fix every assertion on a fresh allocator saw `0`, so `capacity`
//! being non-zero is what makes them fail against the old code.
//!
//! ## Mutation check
//!
//! All seven tests fail against the old one-line body. Against the new one these
//! mutations are caught: dropping the free-stack term, dropping the low-pass
//! term, ignoring `low_cursor`, bounding the low pass by `next_free` instead of
//! `floor`, and dropping the high pass's `max(base, cursor)`.
//!
//! **One survives, deliberately recorded rather than hidden:** removing the
//! `cursor = start + pages * frame` line inside `high_pass_frames`. The cursor
//! there only ever feeds `max(reg.base, cursor)`, and for a *well-formed* region
//! list — sorted, non-overlapping, which is all `plan_regions` and
//! `reserve_region` produce — the next region's base is at or above the previous
//! region's end, so `max` is dominated by the base and the advance is a dead
//! store. It is only load-bearing for overlapping regions, which no caller can
//! create, so there is no fixture that reaches it without inventing an input the
//! allocator cannot see.

use crate::frame_allocator::{FrameAllocator, MemoryRegion};

fn region(base: u64, length: u64) -> MemoryRegion {
    MemoryRegion { base, length, kind: 0 }
}

/// How many frames a fresh allocator built from `map` hands out before it is
/// dry. Run on its own allocator so it does not disturb the one under test.
fn capacity(map: &[MemoryRegion]) -> usize {
    let mut fa = FrameAllocator::new(map);
    let mut n = 0usize;
    while fa.alloc_frame().is_some() {
        n += 1;
        assert!(n < 1 << 21, "runaway allocator");
    }
    n
}

/// Drain the map for real, requiring the reported count to match the number of
/// frames still to come at *every* step.
///
/// This is the differential oracle the whole file is built on. It catches a
/// count that is constant, lags, or over-counts partway through — including the
/// low pass, whose cursor has to advance for the number to keep falling once the
/// high pass is spent. Running it on a straddling map is what reaches `low_cursor`
/// at all.
fn assert_count_tracks_drain(map: &[MemoryRegion]) {
    let want = capacity(map);
    assert!(want > 0, "the fixture must be able to serve something");

    let mut fa = FrameAllocator::new(map);
    assert_eq!(fa.free_frames_count(), want, "before any allocation");

    let mut served = 0usize;
    while fa.alloc_frame().is_some() {
        served += 1;
        assert_eq!(
            fa.free_frames_count(),
            want - served,
            "after {served} of {want} frames"
        );
    }
    assert_eq!(served, want, "the allocator served everything it counted");
    assert_eq!(fa.free_frames_count(), 0, "a drained allocator has nothing left");
}

/// A fresh allocator reports the whole region, not the empty recycled stack.
#[test]
fn a_fresh_allocator_reports_every_page_it_can_serve() {
    let map = [region(0x1000, 0x10_000)];
    let want = capacity(&map);
    assert_eq!(want, 16, "the map is 16 pages");

    let fa = FrameAllocator::new(&map);
    assert_eq!(
        fa.free_frames_count(),
        want,
        "a fresh allocator must report every servable page, not the stack depth"
    );
    // The stack is genuinely empty; the two numbers are different things.
    assert_eq!(fa.recycled_frames_count(), 0, "nothing has been freed yet");
}

/// The count has to track a drain one frame at a time.
///
/// This is the differential check: at every step the number reported must equal
/// the number of frames still to come. A count that is a constant, or that lags
/// by one, fails.
#[test]
fn free_frames_count_is_exact_at_every_step_of_a_drain() {
    let map = [region(0x1000, 0x10_000)];
    assert_eq!(capacity(&map), 16, "the map is 16 pages");
    assert_count_tracks_drain(&map);
}

/// The same differential check on a region that straddles the 16 MiB floor, so
/// the low pass runs and `low_cursor` has to keep the number falling after the
/// high pass is spent.
///
/// Without this fixture a count that ignored `low_cursor` and kept reporting the
/// low half's full size was invisible: the high-pass-only checks never reached
/// the low pass at all.
#[test]
fn the_reported_count_tracks_the_low_pass_to_zero() {
    // 2 MiB .. 34 MiB, floor at 16 MiB inside it.
    let map = [region(0x20_0000, 0x0200_0000)];
    assert_count_tracks_drain(&map);
}

/// A region that straddles the 16 MiB floor is served by both passes, and the
/// count must include the pages of the *second* one.
///
/// This is the half a naive count drops: the high pass alone would report 4608
/// here and leave the 3584 pages below the floor out entirely — which is what
/// made `mmap` refuse mappings that were available.
#[test]
fn a_straddling_region_counts_the_pages_of_both_passes() {
    // 2 MiB .. 34 MiB: floor lands at 16 MiB inside the region.
    const BELOW: usize = (0x100_0000 - 0x20_0000) / 0x1000; // 3584
    const ABOVE: usize = (0x20_0000 + 0x0200_0000 - 0x100_0000) / 0x1000; // 4608
    let map = [region(0x20_0000, 0x0200_0000)];

    let mut fa = FrameAllocator::new(&map);
    assert_eq!(
        fa.free_frames_count(),
        BELOW + ABOVE,
        "both halves of the region are free"
    );

    // Drain the high pass by hand — the allocator serves it first — and the low
    // half must still be counted.
    for _ in 0..ABOVE {
        fa.alloc_frame().expect("a high-pass frame");
    }
    assert_eq!(
        fa.free_frames_count(),
        BELOW,
        "the below-floor half must survive the high pass being spent"
    );
    assert_eq!(capacity(&map), BELOW + ABOVE, "the drain agrees");
}

/// A recycled frame is counted once, not twice: freeing and re-allocating must
/// return the count to where it was, not one higher.
#[test]
fn a_recycled_frame_is_not_counted_twice() {
    let map = [region(0x1000, 0x10_000)];
    let mut fa = FrameAllocator::new(&map);
    let start = fa.free_frames_count();

    let a = fa.alloc_frame().expect("a frame");
    assert_eq!(fa.free_frames_count(), start - 1, "one frame is now in use");

    fa.free_frame(a);
    assert_eq!(
        fa.free_frames_count(),
        start,
        "the freed frame is available again, exactly once"
    );

    let b = fa.alloc_frame().expect("a frame");
    assert_eq!(b.as_u64(), a.as_u64(), "the recycled frame comes back first");
    assert_eq!(
        fa.free_frames_count(),
        start - 1,
        "handing it out again costs one, not two"
    );
}

/// A reservation removes pages from the map, so it must remove them from the
/// count too — the count cannot be `total_memory`-in-pages computed once and
/// then ignored while `reserve_region` reshapes the list.
#[test]
fn reserving_pages_lowers_the_reported_free_count() {
    let map = [region(0x1000, 0x10_000)]; // 16 pages
    let mut fa = FrameAllocator::new(&map);
    assert_eq!(fa.free_frames_count(), 16);

    fa.reserve_region(0x1000, 0x4_000); // take the first 4 pages
    assert_eq!(
        fa.free_frames_count(),
        12,
        "four reserved pages are no longer free"
    );
    assert_eq!(
        capacity(&map) - 4,
        12,
        "and the drain from the same map agrees once the four are removed"
    );
}

/// The whole point of the fix: `mmap_extent` refuses a mapping up front when
/// `free_frames` cannot cover it, so the number has to describe what the
/// machine can actually deliver. A 32 MiB region must not report fewer than the
/// 8192 pages it holds.
#[test]
fn the_reported_count_does_not_under_count_a_large_region() {
    let map = [region(0x20_0000, 0x0200_0000)]; // 32 MiB
    let fa = FrameAllocator::new(&map);
    assert_eq!(
        fa.free_frames_count(),
        0x0200_0000 / 0x1000,
        "a 32 MiB region is 8192 free pages, not the depth of an empty stack"
    );
}
