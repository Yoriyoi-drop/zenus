use x86_64::PhysAddr;
use x86_64::structures::paging::{FrameAllocator as FrameAllocatorTrait, Size4KiB, PhysFrame};
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;

use crate::paging::PAGE_SIZE;

const FREE_STACK_SIZE: usize = 4096;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct MemoryRegion {
    pub base: u64,
    pub length: u64,
    pub kind: u64,
}

pub static FRAME_ALLOCATOR: SpinLock<FrameAllocator> = SpinLock::new(FrameAllocator {
    regions: [MemRegion { base: 0, length: 0 }; MAX_REGIONS],
    region_count: 0,
    next_free: 0,
    free_stack: [0; FREE_STACK_SIZE],
    free_count: 0,
    total_memory: 0,
    used_memory: 0,
});

impl MemoryRegion {
    pub fn is_usable(&self) -> bool {
        self.kind == 0
    }
}

#[derive(Debug, Clone, Copy)]
struct MemRegion {
    base: u64,
    length: u64,
}

const MAX_REGIONS: usize = 64;

pub struct FrameAllocator {
    regions: [MemRegion; MAX_REGIONS],
    region_count: usize,
    next_free: u64,
    free_stack: [u64; FREE_STACK_SIZE],
    free_count: usize,
    total_memory: u64,
    used_memory: u64,
}

impl FrameAllocator {
    pub fn new(memory_map: &[MemoryRegion]) -> Self {
        let mut allocator = FrameAllocator {
            regions: [MemRegion { base: 0, length: 0 }; MAX_REGIONS],
            region_count: 0,
            next_free: 0,
            free_stack: [0; FREE_STACK_SIZE],
            free_count: 0,
            total_memory: 0,
            used_memory: 0,
        };

        for entry in memory_map {
            if entry.is_usable() && entry.length > 0 {
                if allocator.region_count < MAX_REGIONS {
                    allocator.regions[allocator.region_count] = MemRegion {
                        base: entry.base,
                        length: entry.length,
                    };
                    allocator.region_count += 1;
                    allocator.total_memory += entry.length;
                }
            }
        }

        allocator.next_free = if allocator.region_count > 0 {
            let base = allocator.regions[0].base;
            let end = allocator.regions[0].base + allocator.regions[0].length;
            if base < 0x100_0000 && end > 0x100_0000 {
                0x100_0000
            } else {
                base
            }
        } else {
            0
        };

        let s = SerialPort::new(0x3F8);
        s.write_str("[OK] Memory: ");
        s.write_u64(allocator.total_memory / (1024 * 1024));
        s.write_str(" MB total\n");

        allocator
    }

    pub fn alloc_frame(&mut self) -> Option<PhysAddr> {
        if self.free_count > 0 {
            self.free_count -= 1;
            let addr = PhysAddr::new(self.free_stack[self.free_count]);
            self.used_memory += PAGE_SIZE as u64;
            return Some(addr);
        }

        let frame_size = PAGE_SIZE as u64;

        for reg_idx in 0..self.region_count {
            let reg = self.regions[reg_idx];
            let start = core::cmp::max(reg.base, self.next_free);
            let start_aligned = (start + 0xFFF) & !0xFFF;
            let end = reg.base + reg.length;

            if start_aligned + frame_size <= end {
                self.next_free = start_aligned + frame_size;
                self.used_memory += frame_size;
                return Some(PhysAddr::new(start_aligned));
            }
        }

        // Fallback: scan regions from base (for frames freed back as regions)
        for reg_idx in 0..self.region_count {
            let reg = self.regions[reg_idx];
            let start_aligned = (reg.base + 0xFFF) & !0xFFF;
            let end = reg.base + reg.length;
            if start_aligned + frame_size <= end && start_aligned < self.next_free {
                // Found a frame before next_free that we skipped before
                self.regions[reg_idx].base = start_aligned + frame_size;
                self.used_memory += frame_size;
                return Some(PhysAddr::new(start_aligned));
            }
        }

        None
    }

    pub fn free_frame(&mut self, addr: PhysAddr) {
        let a = addr.as_u64();
        for i in 0..self.free_count {
            if self.free_stack[i] == a {
                return;
            }
        }
        for i in 0..self.region_count {
            let r = self.regions[i];
            if a >= r.base && a < r.base + r.length {
                return;
            }
        }
        if self.free_count < FREE_STACK_SIZE {
            self.free_stack[self.free_count] = a;
            self.free_count += 1;
        } else if self.region_count < MAX_REGIONS {
            self.regions[self.region_count] = MemRegion {
                base: a,
                length: PAGE_SIZE as u64,
            };
            self.region_count += 1;
            if self.next_free > a {
                self.next_free = a;
            }
        } else {
            let s = zenus_console::serial::SerialPort::new(0x3F8);
            s.write_str("[WARN] Frame free stack overflow! Frame lost.\n");
        }
        self.used_memory = self.used_memory.saturating_sub(PAGE_SIZE as u64);
    }

    pub fn used_memory(&self) -> u64 { self.used_memory }
    pub fn total_memory(&self) -> u64 { self.total_memory }
    pub fn free_frames_count(&self) -> usize { self.free_count }

    pub fn reserve_region(&mut self, base: u64, length: u64) {
        if length == 0 { return; }
        let end = base + length;
        let mut i = 0;
        while i < self.region_count {
            let r = self.regions[i];
            let r_end = r.base + r.length;
            if end <= r.base || base >= r_end {
                i += 1;
                continue;
            }
            // Overlaps covers region completely
            if base <= r.base && end >= r_end {
                for j in i..self.region_count - 1 {
                    self.regions[j] = self.regions[j + 1];
                }
                self.region_count -= 1;
                continue;
            }
            // Overlap at start
            if base <= r.base {
                self.regions[i] = MemRegion { base: end, length: r_end - end };
                i += 1;
                continue;
            }
            // Overlap at end
            if end >= r_end {
                self.regions[i] = MemRegion { base: r.base, length: base - r.base };
                i += 1;
                continue;
            }
            // Split: kernel region in the middle
            self.regions[i] = MemRegion { base: r.base, length: base - r.base };
            if self.region_count < MAX_REGIONS {
                let mut j = self.region_count;
                while j > i + 1 {
                    self.regions[j] = self.regions[j - 1];
                    j -= 1;
                }
                self.regions[i + 1] = MemRegion { base: end, length: r_end - end };
                self.region_count += 1;
            }
            i += 1;
        }
        // Fix next_free if it landed in the reserved range
        if self.next_free >= base && self.next_free < end {
            self.next_free = end;
        }
    }
}

unsafe impl FrameAllocatorTrait<Size4KiB> for FrameAllocator {
    fn allocate_frame(&mut self) -> Option<PhysFrame<Size4KiB>> {
        self.alloc_frame().map(|addr| PhysFrame::containing_address(addr))
    }
}

pub fn global_init(memory_map: &[MemoryRegion]) {
    let mut fa = FRAME_ALLOCATOR.lock();
    for entry in memory_map {
        if entry.is_usable() && entry.length > 0 {
            let idx = fa.region_count;
            if idx < MAX_REGIONS {
                fa.regions[idx] = MemRegion {
                    base: entry.base,
                    length: entry.length,
                };
                fa.region_count = idx + 1;
                fa.total_memory += entry.length;
            }
        }
    }
    // Exclude ALL non-usable regions from the frame allocator.
    // KERNEL_AND_MODULES (6), ACPI_RECLAIMABLE (7), ACPI_NVS (10),
    // MMIO, reserved, bootloader regions, etc. must be excluded
    // to prevent the allocator from handing out frames that overlap
    // kernel image, ACPI tables, or hardware MMIO pages.
    for entry in memory_map {
        let kind = entry.kind;
        if kind != 0 && entry.length > 0 {
            fa.reserve_region(entry.base, entry.length);
        }
    }
    // Update total_memory to only count truly usable frames
    fa.total_memory = 0;
    for i in 0..fa.region_count {
        fa.total_memory += fa.regions[i].length;
    }
    fa.next_free = if fa.region_count > 0 {
        let base = fa.regions[0].base;
        let end = fa.regions[0].base + fa.regions[0].length;
        let start = if base < 0x100_0000 && end > 0x100_0000 {
            0x100_0000
        } else {
            base
        };
        // Also advance past any kernel/module pages at the low end
        let mut kernel_end = 0;
        for entry in memory_map {
            if entry.kind == 6 {
                let candidate = entry.base + entry.length;
                if candidate > kernel_end { kernel_end = candidate; }
            }
        }
        if start < kernel_end { kernel_end } else { start }
    } else {
        0
    };
}
