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
