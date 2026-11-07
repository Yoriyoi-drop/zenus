/// Virtual Memory Area tracking for mmap/munmap/mprotect.

pub const MAX_VMAS: usize = 64;

/// Highest user-space address a mapping may reach. `mmap` refuses anything
/// above this, so `find_free` treats it as the ceiling too.
pub const MAX_MAPPING_SIZE: u64 = 0x7F00_0000_0000;

// mmap flags (Linux x86_64)
pub const MAP_SHARED: u64 = 0x01;
pub const MAP_PRIVATE: u64 = 0x02;
pub const MAP_FIXED: u64 = 0x10;
pub const MAP_ANONYMOUS: u64 = 0x20;
pub const MAP_POPULATE: u64 = 0x008000;
pub const MAP_NORESERVE: u64 = 0x04000;

// Protection flags
pub const PROT_NONE: u64 = 0x0;
pub const PROT_READ: u64 = 0x1;
pub const PROT_WRITE: u64 = 0x2;
pub const PROT_EXEC: u64 = 0x4;

// Page flags from protection
pub const PAGE_USER: u64 = 1 << 2;
pub const PAGE_WRITABLE: u64 = 1 << 1;
pub const PAGE_NO_EXECUTE: u64 = 1u64 << 63;

#[derive(Clone, Copy)]
pub struct VmaRegion {
    pub start: u64,
    pub end: u64,
    pub prot: u64,
    pub flags: u64,
    pub file_offset: u64,
    pub inode: u64,
    pub valid: bool,
}

impl Default for VmaRegion {
    fn default() -> Self {
        Self::new()
    }
}

impl VmaRegion {
    pub const fn new() -> Self {
        VmaRegion {
            start: 0,
            end: 0,
            prot: 0,
            flags: 0,
            file_offset: 0,
            inode: 0,
            valid: false,
        }
    }

    pub fn contains(&self, addr: u64) -> bool {
        self.valid && addr >= self.start && addr < self.end
    }

    pub fn page_aligned_start(&self) -> u64 {
        self.start & !0xFFF
    }
    pub fn page_aligned_end(&self) -> u64 {
        // `saturating_add`: `(end + 0xFFF)` panicked for `end == u64::MAX` in
        // debug builds, and a debug/release behavioural split in a page-table
        // helper is exactly the kind of bug that only shows up on one machine.
        self.end.saturating_add(0xFFF) & !0xFFF
    }

    pub fn to_page_flags(&self) -> u64 {
        let mut flags = PAGE_USER;
        if self.prot & PROT_WRITE != 0 {
            flags |= PAGE_WRITABLE;
        }
        if self.prot & PROT_EXEC == 0 {
            flags |= PAGE_NO_EXECUTE;
        }
        flags
    }
}

#[derive(Clone, Copy)]
pub struct VmaTable {
    pub regions: [VmaRegion; MAX_VMAS],
    pub count: usize,
    pub mmap_base: u64,
}

impl Default for VmaTable {
    fn default() -> Self {
        Self::new()
    }
}

impl VmaTable {
    pub const fn new() -> Self {
        VmaTable {
            regions: [VmaRegion::new(); MAX_VMAS],
            count: 0,
            mmap_base: 0x2000_0000_0000, // default mmap base
        }
    }

    pub fn insert(&mut self, start: u64, end: u64, prot: u64, flags: u64) -> Option<usize> {
        // An empty or inverted range is not a mapping. Accepting `end <= start`
        // is what let a zero-length VMA in, which in turn made `find_free`
        // recurse without ever making progress.
        if end <= start {
            return None;
        }
        if end > MAX_MAPPING_SIZE {
            return None;
        }
        // Reject a range that overlaps any existing mapping. Two live VMAs for
        // the same pages make `find`/`munmap` ambiguous and leave the page
        // tables inconsistent — a silent memory-corruption hazard. `MAP_FIXED`
        // over an existing region (and any straddling `mmap`) is refused here
        // at the source rather than layered on top; the non-fixed path is
        // reached through `find_free`, which already returns a non-overlapping
        // candidate, so this does not reject ordinary mappings.
        for region in self.regions.iter().take(self.count) {
            if region.valid && start < region.end && end > region.start {
                return None;
            }
        }
        if self.count >= MAX_VMAS {
            return None;
        }
        let idx = self.count;
        self.regions[idx] = VmaRegion {
            start,
            end,
            prot,
            flags,
            file_offset: 0,
            inode: 0,
            valid: true,
        };
        self.count += 1;
        Some(idx)
    }

    pub fn find(&self, addr: u64) -> Option<usize> {
        for i in 0..self.count {
            if self.regions[i].contains(addr) {
                return Some(i);
            }
        }
        None
    }

    pub fn find_exact(&self, start: u64, end: u64) -> Option<usize> {
        for i in 0..self.count {
            if self.regions[i].valid && self.regions[i].start == start && self.regions[i].end == end
            {
                return Some(i);
            }
        }
        None
    }

    pub fn remove(&mut self, idx: usize) -> bool {
        if idx >= self.count {
            return false;
        }
        self.regions[idx].valid = false;
        // Compact
        let mut write = 0;
        for read in 0..self.count {
            if self.regions[read].valid {
                if write != read {
                    self.regions[write] = self.regions[read];
                }
                write += 1;
            }
        }
        self.count = write;
        true
    }

    /// Remove every VMA that overlaps `[start, end)` (half-open). `munmap` unmaps
    /// the whole page span unconditionally, so leaving a partially-overlapped
    /// VMA in the table would claim pages that are already gone — a
    /// use-after-free, and a later `munmap` of that stale VMA re-frees the same
    /// frames (a double-free). `remove` compacts, so we re-scan after each
    /// deletion and bound the passes by the entry count (BUG-074).
    pub fn remove_overlapping(&mut self, start: u64, end: u64) -> usize {
        let mut removed = 0;
        let mut guard = 0;
        loop {
            if guard > MAX_VMAS {
                break;
            }
            guard += 1;
            let mut hit: Option<usize> = None;
            for i in 0..self.count {
                let r = &self.regions[i];
                if r.valid && r.start < end && r.end > start {
                    hit = Some(i);
                    break;
                }
            }
            match hit {
                Some(idx) => {
                    self.remove(idx);
                    removed += 1;
                }
                None => break,
            }
        }
        removed
    }

    /// Find a free page-aligned range of `size` bytes at or after `hint`.
    ///
    /// Three bugs lived here; all three are now pinned by tests:
    ///
    /// * `size == 0` recursed forever (`end == start` re-triggered the "end is
    ///   inside the region" branch with an unchanged hint), overflowing the
    ///   stack and aborting the whole process.
    /// * `start + size` and `(end + 0xFFF)` panicked on overflow in debug
    ///   builds and wrapped silently in release, handing out addresses far
    ///   outside the user range.
    /// * The overlap test only asked whether the *start* or the *end* landed
    ///   inside a region, so a range straddling one (`0x1000..0x9000` over a
    ///   `0x5000..0x7000` mapping) was reported free — `mmap` then mapped on
    ///   top of an existing VMA.
    pub fn find_free(&self, size: u64, hint: u64) -> Option<u64> {
        if size == 0 || size > MAX_MAPPING_SIZE {
            return None;
        }
        let mut candidate = hint & !0xFFF;
        // Bounded by MAX_VMAS + 1: every iteration must move the candidate past
        // one region, so this cannot spin.
        for _ in 0..=MAX_VMAS {
            match candidate.checked_add(size) {
                Some(end) if end <= MAX_MAPPING_SIZE => {}
                _ => return None,
            }

            let mut bump: Option<u64> = None;
            for region in self.regions.iter().take(self.count) {
                if !region.valid {
                    continue;
                }
                // Proper half-open overlap test.
                if candidate < region.end && candidate.saturating_add(size) > region.start {
                    bump = Some(region.end);
                    break;
                }
            }

            match bump {
                None => return Some(candidate),
                Some(next) => {
                    let aligned = next & !0xFFF;
                    if aligned <= candidate {
                        // No forward progress (unaligned or pathological
                        // region): refuse instead of looping.
                        return None;
                    }
                    candidate = aligned;
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod host_tests {
    use super::*;

    /// BUG-073: `insert` must refuse a range that overlaps an existing VMA.
    /// Without this check a `MAP_FIXED` (or any straddling `mmap`) over a live
    /// mapping produced two VMAs for the same pages, which made `find`/`munmap`
    /// ambiguous and left the page tables inconsistent — a silent memory
    /// corruption hazard.
    #[test]
    fn insert_refuses_overlapping_range() {
        let mut t = VmaTable::new();

        // First mapping succeeds.
        let a = t.insert(0x1000, 0x3000, PROT_READ | PROT_WRITE, 0);
        assert!(a.is_some(), "first mapping should be accepted");

        // Overlapping from the start side: 0x2000..0x4000 sits on top of a.
        let b = t.insert(0x2000, 0x4000, PROT_READ | PROT_WRITE, 0);
        assert!(b.is_none(), "range overlapping an existing VMA must be refused");

        // Overlapping from the end side: 0x0000..0x2000 straddles a's start.
        let c = t.insert(0x0000, 0x2000, PROT_READ | PROT_WRITE, 0);
        assert!(c.is_none(), "range overlapping an existing VMA must be refused");

        // Fully contained overlap: 0x1500..0x2500 lives entirely inside a.
        let d = t.insert(0x1500, 0x2500, PROT_READ | PROT_WRITE, 0);
        assert!(d.is_none(), "range contained in an existing VMA must be refused");

        // Adjacent (end-exclusive) ranges do NOT overlap and must be accepted.
        let e = t.insert(0x3000, 0x5000, PROT_READ | PROT_WRITE, 0);
        assert!(e.is_some(), "range that only touches the boundary must be accepted");

        assert_eq!(t.count, 2, "exactly two VMAs should have been recorded");
    }

    /// A subsequent non-overlapping mapping after one exists must still go in.
    #[test]
    fn insert_accepts_disjoint_range() {
        let mut t = VmaTable::new();
        assert!(t.insert(0x1000, 0x2000, PROT_READ, 0).is_some());
        assert!(t.insert(0x5000, 0x6000, PROT_READ, 0).is_some());
        assert_eq!(t.count, 2);
    }

    /// BUG-074: `munmap` unmaps the whole page span, so `remove_overlapping`
    /// must drop *every* VMA the range touches, not just the first. Leaving a
    /// partially-overlapped VMA behind would claim freed frames (use-after-free
    /// / double-free on a later munmap).
    #[test]
    fn remove_overlapping_drops_every_touched_vma() {
        let mut t = VmaTable::new();
        assert!(t.insert(0x1000, 0x3000, PROT_READ, 0).is_some()); // A
        assert!(t.insert(0x3000, 0x5000, PROT_READ, 0).is_some()); // B (adjacent)
        assert!(t.insert(0x6000, 0x8000, PROT_READ, 0).is_some()); // C

        // Range 0x1000..0x5000 spans A and B exactly; C must survive.
        let n = t.remove_overlapping(0x1000, 0x5000);
        assert_eq!(n, 2, "should have removed both A and B");
        assert_eq!(t.count, 1, "only C should remain");
        assert!(t.find(0x1000).is_none());
        assert!(t.find(0x3000).is_none());
        assert!(t.find(0x6000).is_some());

        // Partial overlap: 0x7000..0x9000 reaches into C from the end side.
        let m = t.remove_overlapping(0x7000, 0x9000);
        assert_eq!(m, 1);
        assert_eq!(t.count, 0);

        // A range touching nothing removes nothing.
        let k = t.remove_overlapping(0x9000, 0xA000);
        assert_eq!(k, 0);
        assert_eq!(t.count, 0);
    }
}
