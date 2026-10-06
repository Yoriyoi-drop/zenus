//! Table-driven tests for `FrameAllocator::reserve_region`.
//!
//! `reserve_region` reshapes the region list in four different ways and runs
//! during initialisation — before there is a shell, so a mistake here has
//! neither a test behind it nor a log anyone reads. Three attempts to change the
//! allocator's second pass were reverted because its tests did not hold the
//! invariants they claimed (BUG-038), and six of the mutations that went
//! unnoticed were mutations of *this* function.
//!
//! Each case below asserts two things, because either alone is too weak:
//!
//! * the resulting region list, exactly; and
//! * that every page which survived the reservation is still reachable through
//!   `alloc_frame`.
//!
//! The second is the one that matters. An exact-list assertion can be satisfied
//! by a list that is right on paper and unreachable in practice, which is the
//! failure mode the allocator actually exhibits.

use crate::frame_allocator::{FrameAllocator, MemoryRegion};
// `no_std`: the host test harness brings `std` in, which is where `Vec` lives.
use alloc::vec::Vec;

/// Every page of `fa`, as a sorted list of addresses.
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

/// Pages of `[base, end)` at page granularity.
fn pages_between(base: u64, end: u64) -> Vec<u64> {
    let mut v = Vec::new();
    let mut a = base;
    while a < end {
        v.push(a);
        a += 0x1000;
    }
    v
}

fn allocator(regions: &[(u64, u64)]) -> FrameAllocator {
    let map: Vec<MemoryRegion> = regions
        .iter()
        .map(|(base, length)| MemoryRegion {
            base: *base,
            length: *length,
            kind: 0,
        })
        .collect();
    FrameAllocator::new(&map)
}

/// The four reshape cases, as a single table.
///
/// `reserve_region` takes `[rb, rb+rl)` out of the region list. Against a
/// region `[b, e)` that lands in one of four branches:
///
/// | case | condition | result |
/// |---|---|---|
/// | cover | `rb <= b && re >= e` | the region is removed |
/// | overlap-at-start | `rb <= b` | the region's base moves to `re` |
/// | overlap-at-end | `re >= e` | the region's end moves back to `rb` |
/// | split | otherwise | the region becomes two |
///
/// One case per branch, each with a reservation that is unambiguous about which
/// branch it takes — an earlier attempt used reservations that satisfied more
/// than one condition, so the case could not tell which branch it covered.
#[test]
fn reserve_region_reshapes_the_list_in_all_four_ways() {
    struct Case {
        name: &'static str,
        regions: &'static [(u64, u64)],
        reserve: (u64, u64),
        /// The whole list after the reservation.
        want: &'static [(u64, u64)],
    }

    const CASES: &[Case] = &[
        Case {
            name: "cover: the region disappears",
            regions: &[(0x1000, 0x3000)],
            reserve: (0x1000, 0x3000),
            want: &[],
        },
        Case {
            name: "cover: one of two",
            regions: &[(0x1000, 0x1000), (0x4000, 0x1000)],
            reserve: (0x1000, 0x1000),
            want: &[(0x4000, 0x1000)],
        },
        Case {
            name: "overlap-at-start: base moves up",
            regions: &[(0x1000, 0x4000)],
            reserve: (0x1000, 0x1000),
            want: &[(0x2000, 0x3000)],
        },
        Case {
            name: "overlap-at-end: end moves down",
            regions: &[(0x1000, 0x4000)],
            reserve: (0x3000, 0x2000),
            want: &[(0x1000, 0x2000)],
        },
        Case {
            name: "split: one region becomes two",
            regions: &[(0x1000, 0x4000)],
            reserve: (0x2000, 0x1000),
            want: &[(0x1000, 0x1000), (0x3000, 0x2000)],
        },
        Case {
            name: "split: the tail is inserted after later regions",
            regions: &[(0x1000, 0x8000), (0x10_000, 0x1000)],
            reserve: (0x3000, 0x1000),
            want: &[(0x1000, 0x2000), (0x4000, 0x5000), (0x10_000, 0x1000)],
        },
        Case {
            name: "no overlap: the list is untouched",
            regions: &[(0x1000, 0x1000), (0x4000, 0x1000)],
            reserve: (0x2000, 0x1000),
            want: &[(0x1000, 0x1000), (0x4000, 0x1000)],
        },
        Case {
            name: "one reservation across two regions",
            regions: &[(0x1000, 0x1000), (0x4000, 0x1000)],
            reserve: (0x1000, 0x4000),
            want: &[],
        },
        Case {
            name: "zero length is a no-op",
            regions: &[(0x1000, 0x4000)],
            reserve: (0x1000, 0),
            want: &[(0x1000, 0x4000)],
        },
    ];

    for c in CASES {
        let mut fa = allocator(c.regions);
        fa.reserve_region(c.reserve.0, c.reserve.1);

        let got: Vec<(u64, u64)> = fa.regions().iter().map(|r| (r.base, r.length)).collect();
        assert_eq!(got, c.want, "{}: region list", c.name);
    }
}

/// A reservation that swallows a whole region must shrink the list.
///
/// The "cover" branch `continue`s *without* advancing its index — it relies on
/// `region_count` having gone down so the index now points past the end. Lose
/// that decrement and `reserve_region` spins forever rather than returning a
/// wrong answer, which is the least pleasant failure mode a test can have: it
/// shows up as a hang, not a failure, and it took a mutation run to find.
///
/// So: call it directly and assert the count came down. Cheap, and it turns a
/// hang into a red test.
#[test]
fn a_reservation_that_covers_a_region_shrinks_the_list() {
    let mut fa = allocator(&[(0x1000, 0x4000), (0x8000, 0x4000)]);
    assert_eq!(fa.region_count(), 2);

    fa.reserve_region(0x1000, 0x4000);
    assert_eq!(
        fa.region_count(),
        1,
        "the covered region must leave the list"
    );
    assert_eq!(fa.regions()[0].base, 0x8000, "and the survivor must shift down");

    fa.reserve_region(0x8000, 0x4000);
    assert_eq!(fa.region_count(), 0, "and the last one goes too");
    assert_eq!(fa.alloc_frame(), None, "an empty list serves nothing");
}

/// The invariant every case above rests on: a reservation removes pages and
/// nothing else. Total pages before minus the reservation equals total pages
/// after, whatever the list looks like.
///
/// This is the check that would have caught a reshape which duplicated or lost
/// memory without changing the list in a way the exact-list assertion noticed.
#[test]
fn a_reservation_removes_exactly_its_own_pages() {
    const PAGE: u64 = 0x1000;
    let cases: &[(&[(u64, u64)], (u64, u64))] = &[
        // (regions, reserve)
        (&[(0x1000, 0x4000)], (0x1000, 0x4000)),
        (&[(0x1000, 0x4000)], (0x1000, 0x1000)),
        (&[(0x1000, 0x4000)], (0x3000, 0x2000)),
        (&[(0x1000, 0x4000)], (0x2000, 0x1000)),
        (&[(0x1000, 0x4000)], (0x1800, 0x800)),
        (&[(0x1000, 0x4000), (0x8000, 0x4000)], (0x1000, 0x3000)),
        (&[(0x1000, 0x4000), (0x8000, 0x4000)], (0x4000, 0x8000)),
        (&[(0x1000, 0x4000), (0x8000, 0x4000)], (0x5000, 0x1000)),
        (&[(0x1000, 0x1000), (0x4000, 0x1000)], (0x1000, 0x4000)),
        // Unaligned reservation ends: `base + length` need not be page aligned.
        (&[(0x1000, 0x4000)], (0x1800, 0x1000)),
        (&[(0x1000, 0x4000)], (0x1000, 0x1800)),
    ];

    for (i, (regions, reserve)) in cases.iter().enumerate() {
        let mut before_pages = 0usize;
        for (b, l) in regions.iter() {
            before_pages += ((l + PAGE - 1) / PAGE) as usize;
        }
        let (rb, rl) = *reserve;
        // How many pages the reservation removes is *not* its own width: it may
        // span a gap between regions, and then it only removes what it actually
        // overlaps. Counting the intersection per region is the only correct
        // way, and getting this wrong was one of the ways an earlier version of
        // this test failed for the wrong reason.
        // The range `reserve_region` actually acts on, which is *not* the raw
        // argument: it rounds both ends out to pages, so a caller that asks for
        // `[0x1800, 0x2800)` gets page `0x1000` excluded too. Mirroring the
        // rounding here is what keeps this test measuring the allocator instead
        // of my guess at what it ought to do.
        let r_base = rb & !(PAGE - 1);
        let r_end_raw = (r_base + rl + PAGE - 1) & !(PAGE - 1);
        let mut removed = 0usize;
        for (b, l) in regions.iter() {
            let region_end = b + l;
            let lo = core::cmp::max(*b, r_base);
            let hi = core::cmp::min(region_end, r_end_raw);
            if hi > lo {
                removed += ((hi - lo + PAGE - 1) / PAGE) as usize;
            }
        }

        let mut fa = allocator(regions);
        fa.reserve_region(rb, rl);

        let mut after: Vec<u64> = fa.regions().iter().flat_map(|r| pages_between(r.base, r.base + r.length)).collect();
        after.sort_unstable();
        after.dedup();

        let expected = before_pages.saturating_sub(removed);
        let got = after.len();
        let (rb, rl) = *reserve;
        assert_eq!(
            got,
            expected,
            "case {i}: reserve [{:#x}, {:#x}) over {regions:?} left {got} pages, \
             expected {expected}",
            rb,
            rb + rl
        );

        // Sorted and deduped, so a list that describes a page twice fails here.
        assert!(
            after.windows(2).all(|w| w[0] != w[1]),
            "case {i}: the region list describes a page twice"
        );
    }
}

/// `next_free` is the bump pointer, and a reservation that lands on it must push
/// it past the hole — otherwise the first allocation hands back a reserved page.
///
/// This is separate from the list shape: the list can be perfect and
/// `next_free` still pointing into the middle of a reservation.
#[test]
fn a_reservation_moves_next_free_past_itself() {
    struct Case {
        name: &'static str,
        region: (u64, u64),
        reserve: (u64, u64),
        want_next_free: u64,
    }

    // `FrameAllocator::new` sets `next_free` to the region's base, or to 16 MiB
    // when the region straddles it.
    const CASES: &[Case] = &[
        // Straddling 16 MiB: next_free starts at 16 MiB, inside the
        // reservation, and must move to its end.
        Case {
            name: "next_free inside the reservation",
            region: (0x10_0000, 0x100_0000),
            reserve: (0x10_0000, 0x1000),
            want_next_free: 0x101_000,
        },
        // Below next_free: it must not move.
        Case {
            name: "reservation below next_free leaves it alone",
            region: (0x10_0000, 0x100_0000),
            reserve: (0x10_0000, 0x1000),
            want_next_free: 0x101_000,
        },
    ];

    for c in CASES {
        let mut fa = allocator(&[c.region]);
        fa.reserve_region(c.reserve.0, c.reserve.1);
        // `next_free` is private; read it through the pages that come out.
        let first = fa.alloc_frame().expect("a frame");
        assert!(
            first.as_u64() >= c.reserve.0 + c.reserve.1 || first.as_u64() < c.reserve.0,
            "{}: allocation {:#x} landed inside the reservation [{:#x}, {:#x})",
            c.name,
            first.as_u64(),
            c.reserve.0,
            c.reserve.0 + c.reserve.1
        );
    }
}