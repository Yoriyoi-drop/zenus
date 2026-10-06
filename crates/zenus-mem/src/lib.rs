#![no_std]
#![allow(static_mut_refs)]

// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub mod allocator;
pub mod frame_allocator;
pub mod paging;
pub mod vma;
pub use vma::VmaTable;

/// Host-side unit tests (`cargo test --target x86_64-unknown-linux-gnu`).
///
/// `VmaTable` is pure bookkeeping with no page-table or allocator access, so
/// the whole lifecycle is testable here. Hardware-backed paths (frame
/// allocator, paging) stay in the QEMU `testing` harness.
#[cfg(test)]
mod host_tests {
    use crate::vma::{
        VmaTable, MAP_ANONYMOUS, MAP_PRIVATE, MAX_MAPPING_SIZE, MAX_VMAS, PAGE_NO_EXECUTE,
        PAGE_USER, PAGE_WRITABLE, PROT_EXEC, PROT_READ, PROT_WRITE,
    };

    /// The regression that `page_table_bytes()` exists for: the page-table
    /// zero-fills used `write_bytes(ptr, 0, 512 * 8)` against a `*mut u64`,
    /// and `write_bytes` counts *elements*, so each of the three page tables
    /// `clone_user_address_space` allocated was zeroed over 32 KiB instead of
    /// 4 KiB. The overrun landed on whatever the frame allocator handed out
    /// next, which is how a `fork` could clobber heap metadata.
    ///
    /// The test cannot catch a reintroduced `512 * 8` — it is the arithmetic
    /// that is pinned. What it does guarantee is that the byte count the call
    /// sites use is a page, so the mistake cannot be made silently again.
    #[test]
    fn a_page_table_is_exactly_one_page() {
        use crate::paging::page_table_bytes;

        assert_eq!(
            page_table_bytes(),
            crate::paging::PAGE_SIZE,
            "page_table_bytes() is what the zero-fills pass to write_bytes"
        );
        // 512 entries of 8 bytes.
        assert_eq!(page_table_bytes(), 512 * 8);
        // The mistake was 8x. Make sure that is still 8x, so the pinned value
        // cannot quietly become the oversized one.
        assert_eq!(512 * 8 * 8, page_table_bytes() * 8);
    }

    /// The regression `FrameAllocator::free_frame` existed for.
    ///
    /// It refused every frame that fell inside a known region, on the theory
    /// that this was a double-free guard. Regions describe memory *available to
    /// be handed out*, so every frame the allocator ever returned matched and
    /// nothing was ever recycled: `free_stack` stayed empty, `used_memory` never
    /// came back down, and `meminfo` reported "Used: 0 frames" for a whole boot.
    ///
    /// `FrameAllocator::new` is host-safe — it only fills the region list, and
    /// nothing here touches a real page table — so the whole cycle is testable.
    #[test]
    fn a_freed_frame_is_handed_out_again() {
        use crate::frame_allocator::{FrameAllocator, MemoryRegion};

        let map = [MemoryRegion { base: 0x1000, length: 0x10_000, kind: 0 }];
        let mut fa = FrameAllocator::new(&map);

        let first = fa.alloc_frame().expect("first frame");
        let second = fa.alloc_frame().expect("second frame");
        assert_ne!(first.as_u64(), second.as_u64());

        fa.free_frame(first);
        // The recycled frame has to come back before the bump pointer moves on.
        let third = fa.alloc_frame().expect("recycled frame");
        assert_eq!(third.as_u64(), first.as_u64(), "a freed frame must be reused");
    }

    /// A frame that is freed, re-allocated and freed again is legitimately
    /// absent from the free stack the second time, so membership there is the
    /// only double-free test that can tell the two cases apart. What it must
    /// *not* do is reject a legitimate free.
    #[test]
    fn a_double_free_does_not_double_count() {
        use crate::frame_allocator::{FrameAllocator, MemoryRegion};

        let map = [MemoryRegion { base: 0x1000, length: 0x10_000, kind: 0 }];
        let mut fa = FrameAllocator::new(&map);
        // `free_frames_count` is still the recycled stack's depth (see its own
        // docs), so it is the right thing to watch here: one free must put
        // exactly one entry on it.
        assert_eq!(fa.free_frames_count(), 0);

        let a = fa.alloc_frame().expect("frame");
        assert_eq!(fa.free_frames_count(), 0);
        fa.free_frame(a);
        assert_eq!(fa.free_frames_count(), 1, "one free, one entry");
        fa.free_frame(a); // the double free
        assert_eq!(fa.free_frames_count(), 1, "one frame back, not two");

        // Re-allocate then free again: the frame was popped, so it is no longer
        // on the stack and this free must be accepted.
        let b = fa.alloc_frame().expect("frame");
        assert_eq!(b.as_u64(), a.as_u64());
        assert_eq!(fa.free_frames_count(), 0);
        fa.free_frame(b);
        assert_eq!(fa.free_frames_count(), 1);
    }

    /// `used_memory` is what `meminfo` prints, and it has to move in both
    /// directions. It used to only move up, because the decrement sat after a
    /// guard that returned first.
    #[test]
    fn used_memory_tracks_allocations_in_both_directions() {
        use crate::frame_allocator::{FrameAllocator, MemoryRegion};

        let map = [MemoryRegion { base: 0x1000, length: 0x10_000, kind: 0 }];
        let mut fa = FrameAllocator::new(&map);
        assert_eq!(fa.used_memory(), 0);

        let a = fa.alloc_frame().expect("frame");
        assert_eq!(fa.used_memory(), 0x1000);
        let b = fa.alloc_frame().expect("frame");
        assert_eq!(fa.used_memory(), 0x2000);

        fa.free_frame(a);
        assert_eq!(fa.used_memory(), 0x1000, "a free must give the count back");
        fa.free_frame(b);
        assert_eq!(fa.used_memory(), 0);

        let c = fa.alloc_frame().expect("frame");
        fa.free_frame(c);
        fa.free_frame(c);
        assert_eq!(fa.used_memory(), 0, "a double free must not decrement twice");
    }

    /// The regression `BlockHeader::size` rounding exists for.
    ///
    /// Two invariants pull against each other in the heap allocator:
    /// `dealloc` derives a block's extent as `block + HEADER_SIZE + size`, so
    /// `size` is a payload length; and every header must be naturally aligned or
    /// its `u64` fields are read misaligned. A header sits immediately before
    /// its payload, so those two only coexist if each block's *extent* is a
    /// multiple of the alignment. A 3-byte block does not give that — its
    /// successor's header would land at `block + 35`.
    ///
    /// `devfs::readdir` allocates a `String` per entry name, so 3-byte blocks
    /// are routine, and `run` allocates an 8616-byte buffer for an ELF. Both
    /// used to leave the next header misaligned, and the corruption propagated
    /// from there: the next allocation's `used_hdr` was rounded up *past* the
    /// preceding header and overwrote its `next` and `canary`, so the free list
    /// stopped describing the heap and a single `run` wedged the allocator.
    ///
    /// The test pins the arithmetic, not the call site — it cannot catch a
    /// reintroduced `size` at the wrong place, but it does pin the one fact the
    /// layout rests on.
    #[test]
    fn a_block_extent_is_a_multiple_of_the_header_alignment() {
        use crate::allocator::{HEADER_SIZE, MIN_BLOCK};
        use core::mem::align_of;

        let align = align_of::<crate::allocator::BlockHeader>();
        assert_eq!(HEADER_SIZE % align, 0, "header size must be a multiple of its alignment");

        // Round a payload length the way `alloc_mut` does, and check the block
        // that follows is still aligned for every length that matters.
        for size in [1usize, 2, 3, 4, 5, 7, 8, 15, 16, 17, 63, 64, 0x1000, 8616, 65536] {
            let extent = (size + align - 1) & !(align - 1);
            assert_eq!(extent % align, 0, "size {size} rounded to {extent:#x}");
            assert!(extent >= size, "rounding must not shrink the payload");
            assert!(extent - size < align, "rounding must waste less than `align`");

            // The successor header sits at block + HEADER + extent.
            let successor = HEADER_SIZE + extent;
            assert_eq!(
                successor % align,
                0,
                "size {size}: successor header would be at +{successor:#x}, misaligned"
            );
        }

        // And a split still has to leave a usable leftover behind.
        assert!(MIN_BLOCK >= 32);
    }

    /// `unmap_page_raw_keep_frame` walks four levels by index. The indices come
    /// from four shifts, and a wrong shift is invisible until it reads the wrong
    /// table — which on a live system means unmapping somebody else's page.
    /// This pins the split against the addresses it actually has to serve.
    #[test]
    fn page_table_indices_are_split_at_the_right_levels() {
        use crate::paging::split_indices;

        // A shared segment is mapped at 0x3000_0000_0000 by `shmat`:
        // PML4 96, and the rest zero, so the walk is PML4[96] -> PDPT[0] ->
        // PD[0] -> PT[0].
        assert_eq!(split_indices(0x3000_0000_0000), [96, 0, 0, 0]);

        // Low addresses and the top of user space.
        assert_eq!(split_indices(0x1000), [0, 0, 0, 1]);
        assert_eq!(split_indices(0), [0, 0, 0, 0]);

        // Every index is in range for a 512-entry table, including the very top
        // of the 48-bit canonical user range.
        for virt in [0u64, 0x1000, 0x7FFF_FFFF_F000, 0x0000_7FFF_FFFF_F000] {
            for idx in split_indices(virt) {
                assert!(idx < 512, "{virt:#x} produced index {idx}");
            }
        }

        // Distinct bits land in distinct levels: two pages differ only in the
        // page-table index, one 2 MiB-aligned block differs only in the
        // page-directory index.
        assert_eq!(split_indices(0x1000)[3], 1);
        assert_eq!(split_indices(0x2000)[3], 2);
        assert_eq!(split_indices(0x1000)[2], 0);
        assert_eq!(split_indices(0x20_0000)[2], 1, "2 MiB boundary is the PD index");
    }

    #[test]
    fn vma_insert_find_contains() {
        let mut table = VmaTable::new();
        let idx = table.insert(0x1000, 0x3000, PROT_READ, MAP_PRIVATE).expect("insert");
        assert_eq!(idx, 0);
        assert_eq!(table.find(0x1000), Some(0));
        assert_eq!(table.find(0x2FFF), Some(0));
        assert_eq!(table.find(0x3000), None, "end is exclusive");
        assert_eq!(table.find(0xFFF), None);
        assert_eq!(table.find_exact(0x1000, 0x3000), Some(0));
        assert_eq!(table.find_exact(0x1000, 0x2000), None);
    }

    #[test]
    fn vma_remove_compacts() {
        let mut table = VmaTable::new();
        table.insert(0x1000, 0x2000, PROT_READ, MAP_PRIVATE).unwrap();
        table.insert(0x3000, 0x4000, PROT_READ, MAP_PRIVATE).unwrap();
        assert!(table.remove(0));
        assert_eq!(table.count, 1);
        assert_eq!(table.find(0x1000), None);
        assert_eq!(table.find(0x3000), Some(0), "survivor shifts to index 0");
        assert!(!table.remove(5), "out of range fails");
    }

    #[test]
    fn vma_table_reports_full() {
        let mut table = VmaTable::new();
        for i in 0..MAX_VMAS {
            let start = 0x1000 + i as u64 * 0x2000;
            assert!(table.insert(start, start + 0x1000, PROT_READ, MAP_PRIVATE).is_some());
        }
        assert_eq!(table.insert(0xFFFF_0000, 0xFFFF_1000, PROT_READ, MAP_PRIVATE), None);
    }

    #[test]
    fn vma_prot_maps_to_page_flags() {
        let mut table = VmaTable::new();
        let rw = table.insert(0x1000, 0x2000, PROT_READ | PROT_WRITE, MAP_PRIVATE).unwrap();
        let flags = table.regions[rw].to_page_flags();
        assert_ne!(flags & PAGE_USER, 0, "user mappings always set U/S");
        assert_ne!(flags & PAGE_WRITABLE, 0);
        assert_ne!(flags & PAGE_NO_EXECUTE, 0, "no PROT_EXEC -> NX");

        let rx = table.insert(0x3000, 0x4000, PROT_READ | PROT_EXEC, MAP_PRIVATE).unwrap();
        let flags = table.regions[rx].to_page_flags();
        assert_eq!(flags & PAGE_WRITABLE, 0, "no PROT_WRITE -> read-only");
        assert_eq!(flags & PAGE_NO_EXECUTE, 0, "PROT_EXEC clears NX");
    }

    #[test]
    fn vma_find_free_skips_occupied_ranges() {
        let mut table = VmaTable::new();
        table.insert(0x5000, 0x7000, PROT_READ, MAP_ANONYMOUS).unwrap();

        // Free hint passes through untouched.
        assert_eq!(table.find_free(0x1000, 0x1000), Some(0x1000));
        // Hint inside the region slides to its end.
        assert_eq!(table.find_free(0x1000, 0x6000), Some(0x7000));
        // A range overlapping the region's start slides past it too.
        assert_eq!(table.find_free(0x2000, 0x6000), Some(0x7000));
        // Past the user-space ceiling there is no room.
        assert_eq!(table.find_free(0x1000, 0x7F00_0000_0000), None);
    }

    #[test]
    fn vma_region_alignment_helpers() {
        let mut table = VmaTable::new();
        let idx = table.insert(0x1001, 0x2FFF, PROT_READ, MAP_PRIVATE).unwrap();
        assert_eq!(table.regions[idx].page_aligned_start(), 0x1000);
        assert_eq!(table.regions[idx].page_aligned_end(), 0x3000);
        assert!(table.regions[idx].contains(0x1001));
        assert!(
            !table.regions[idx].contains(0x1000),
            "start is inclusive, so 0x1000 is below the region"
        );
    }

    /// Regression: `find_free` recursed forever on `size == 0` — `end == start`
    /// re-triggered the "end is inside the region" branch with an unchanged
    /// hint, overflowing the stack and aborting the whole test binary (on the
    /// kernel it panics with interrupts disabled).
    #[test]
    fn zero_and_oversized_requests_do_not_recurse() {
        let mut table = VmaTable::new();
        table.insert(0x5000, 0x7000, PROT_READ, MAP_ANONYMOUS).unwrap();

        assert_eq!(table.find_free(0, 0x6000), None, "size 0 is not an allocation");
        assert_eq!(table.find_free(1, 0x6000), Some(0x7000), "one byte still works");
        assert_eq!(
            table.find_free(MAX_MAPPING_SIZE, 0x1000),
            None,
            "a mapping past the user ceiling is refused"
        );
    }

    /// Regression: `let end = start + size` panicked on overflow in debug and
    /// wrapped silently in release; `(self.end + 0xFFF)` in
    /// `page_aligned_end` did the same.
    #[test]
    fn arithmetic_does_not_overflow() {
        let mut table = VmaTable::new();
        let idx = table
            .insert(MAX_MAPPING_SIZE - 0x1000, MAX_MAPPING_SIZE, PROT_READ, MAP_PRIVATE)
            .unwrap();
        assert_eq!(table.regions[idx].page_aligned_end(), MAX_MAPPING_SIZE);

        // A hint near the top plus a huge size must be refused, not wrapped.
        assert_eq!(table.find_free(u64::MAX, 0x1000), None);
        assert_eq!(table.find_free(1 << 40, MAX_MAPPING_SIZE - 0x100), None);
    }

    /// Regression: the overlap test only asked whether the start or the end of
    /// the candidate landed inside a region, so a range straddling one was
    /// reported free and `mmap` mapped on top of an existing VMA.
    #[test]
    fn find_free_never_straddles_an_existing_region() {
        let mut table = VmaTable::new();
        table.insert(0x5000, 0x7000, PROT_READ, MAP_ANONYMOUS).unwrap();

        // 0x1000..0x9000 spans the region at 0x5000..0x7000.
        let got = table.find_free(0x8000, 0x1000).expect("a free range exists");
        assert!(
            got >= 0x7000,
            "find_free returned {got:#x}, which overlaps 0x5000..0x7000"
        );

        // Same for a hint that starts below and whose end lands inside.
        let got = table.find_free(0x6000, 0x1000).expect("a free range exists");
        assert!(got >= 0x7000, "{got:#x} overlaps the region");

        // A hint inside the region moves past its end.
        assert_eq!(table.find_free(0x1000, 0x6000), Some(0x7000));
    }

    /// Regression: `insert` accepted empty and inverted ranges; a zero-length
    /// VMA is exactly what made `find_free` loop without progress.
    #[test]
    fn insert_rejects_empty_and_out_of_range_regions() {
        let mut table = VmaTable::new();
        assert!(table.insert(0x2000, 0x2000, PROT_READ, MAP_PRIVATE).is_none());
        assert!(table.insert(0x3000, 0x2000, PROT_READ, MAP_PRIVATE).is_none());
        assert!(table.insert(0, 0, PROT_READ, MAP_PRIVATE).is_none());
        assert!(table
            .insert(0x2000, MAX_MAPPING_SIZE + 0x1000, PROT_READ, MAP_PRIVATE)
            .is_none());
        assert_eq!(table.count, 0, "no region was recorded");
    }

    /// A table packed to `MAX_VMAS`: the search must terminate and still not
    /// overlap anything.
    #[test]
    fn find_free_terminates_with_many_regions() {
        let mut table = VmaTable::new();
        for i in 0..MAX_VMAS {
            let start = 0x10_0000 + i as u64 * 0x2000;
            assert!(table.insert(start, start + 0x2000, PROT_READ, MAP_PRIVATE).is_some());
        }
        let got = table.find_free(0x1000, 0x10_0000).expect("space above the stack");
        assert!(got >= 0x10_0000 + (MAX_VMAS as u64) * 0x2000);

        // A request that cannot fit above the regions is refused, not wrapped.
        assert_eq!(table.find_free(MAX_MAPPING_SIZE, 0x10_0000), None);
    }
}
