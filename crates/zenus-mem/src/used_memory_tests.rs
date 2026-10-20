//! Host tests for the frame allocator's `used_memory` accounting.
//!
//! `meminfo` reports `Used` directly from `FrameAllocator::used_memory()`, but
//! nothing sampled it while frames were actually outstanding, so the line had
//! no test behind it (ROADMAP "Known broken"). These tests pin the invariant
//! that `used_memory` rises by one page per allocation and returns to zero once
//! every allocated frame is freed.

use crate::frame_allocator::{FrameAllocator, MemoryRegion};
use crate::paging::PAGE_SIZE;
use x86_64::PhysAddr;

fn region(base: u64, length: u64) -> MemoryRegion {
    MemoryRegion { base, length, kind: 0 }
}

const MAX_LIVE: usize = 256;

#[test]
fn used_starts_at_zero_on_a_fresh_allocator() {
    let map = [region(0x1000, 0x10_000)];
    let fa = FrameAllocator::new(&map);
    assert_eq!(fa.used_memory(), 0, "nothing allocated yet");
    assert_eq!(fa.total_memory(), 16 * PAGE_SIZE as u64, "the map is 16 pages");
}

#[test]
fn used_tracks_every_allocated_frame() {
    let map = [region(0x1000, 0x10_000)]; // 16 pages
    let mut fa = FrameAllocator::new(&map);
    for n in 1..=16u64 {
        let _ = fa.alloc_frame().expect("the map can serve 16 frames");
        assert_eq!(
            fa.used_memory(),
            n * PAGE_SIZE as u64,
            "after allocating {n} frames"
        );
    }
    assert_eq!(fa.free_frames_count(), 0, "all 16 frames are now in use");
}

#[test]
fn used_returns_to_zero_when_all_frames_freed() {
    let map = [region(0x1000, 0x10_000)]; // 16 pages
    let mut fa = FrameAllocator::new(&map);
    let mut live = [PhysAddr::new(0); 16];
    let mut n = 0usize;
    while let Some(f) = fa.alloc_frame() {
        live[n] = f;
        n += 1;
    }
    assert_eq!(n, 16, "the map served every page");
    assert_eq!(fa.used_memory(), 16 * PAGE_SIZE as u64);

    for f in &live {
        fa.free_frame(*f);
    }
    assert_eq!(fa.used_memory(), 0, "every frame freed");
    assert_eq!(fa.free_frames_count(), 16, "all 16 frames available again");
}

#[test]
fn used_consistency_under_interleaved_alloc_free() {
    let map = [region(0x1000, 0x80_000)]; // 128 pages
    let mut fa = FrameAllocator::new(&map);
    let total = fa.free_frames_count();
    assert_eq!(fa.used_memory(), 0);

    let mut live = [PhysAddr::new(0); MAX_LIVE];
    let mut live_n = 0usize;
    for _ in 0..total {
        let f = fa.alloc_frame().expect("frame");
        live[live_n] = f;
        live_n += 1;
        assert_eq!(
            fa.used_memory(),
            (live_n as u64) * PAGE_SIZE as u64,
            "used rises by one page per allocation"
        );
    }

    // Free every other frame and confirm `used` drops by exactly those pages.
    let mut i = 0;
    let mut kept = 0;
    while i < live_n {
        if i % 2 == 0 {
            fa.free_frame(live[i]);
        } else {
            live[kept] = live[i];
            kept += 1;
        }
        i += 1;
    }
    live_n = kept;
    assert_eq!(
        fa.used_memory(),
        (live_n as u64) * PAGE_SIZE as u64,
        "used tracks the partial free"
    );

    for j in 0..live_n {
        fa.free_frame(live[j]);
    }
    assert_eq!(fa.used_memory(), 0, "fully released");
    assert_eq!(fa.free_frames_count(), total, "the pool is restored");
}
