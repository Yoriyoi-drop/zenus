/// Virtual Memory Area tracking for mmap/munmap/mprotect.

pub const MAX_VMAS: usize = 64;

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
        (self.end + 0xFFF) & !0xFFF
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

impl VmaTable {
    pub const fn new() -> Self {
        VmaTable {
            regions: [VmaRegion::new(); MAX_VMAS],
            count: 0,
            mmap_base: 0x2000_0000_0000, // default mmap base
        }
    }

    pub fn insert(&mut self, start: u64, end: u64, prot: u64, flags: u64) -> Option<usize> {
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

    pub fn find_free(&self, size: u64, hint: u64) -> Option<u64> {
        let start = hint & !0xFFF;
        let end = start + size;
        if end > 0x7F00_0000_0000 {
            return None;
        }

        for i in 0..self.count {
            let r = &self.regions[i];
            if !r.valid {
                continue;
            }
            if start >= r.start && start < r.end {
                return self.find_free(size, r.end);
            }
            if end > r.start && end <= r.end {
                return self.find_free(size, r.end);
            }
        }
        Some(start)
    }
}
