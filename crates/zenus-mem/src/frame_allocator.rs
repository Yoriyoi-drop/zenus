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

        let mut s = SerialPort::new(0x3F8);
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
            let max_base = core::cmp::max(reg.base, self.next_free);
            let start_aligned = max_base.checked_add(0xFFF)? & !0xFFF;
            let end = reg.base + reg.length;

            if start_aligned + frame_size <= end {
                self.next_free = start_aligned + frame_size;
                self.used_memory += frame_size;
                return Some(PhysAddr::new(start_aligned));
            }
        }
        None
    }

    pub fn free_frame(&mut self, addr: PhysAddr) {
        if self.free_count < FREE_STACK_SIZE {
            self.free_stack[self.free_count] = addr.as_u64();
            self.free_count += 1;
        }
        self.used_memory = self.used_memory.saturating_sub(PAGE_SIZE as u64);
    }

    pub fn used_memory(&self) -> u64 { self.used_memory }
    pub fn total_memory(&self) -> u64 { self.total_memory }
    pub fn free_frames_count(&self) -> usize { self.free_count }
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
    fa.next_free = if fa.region_count > 0 {
        let base = fa.regions[0].base;
        let end = fa.regions[0].base + fa.regions[0].length;
        if base < 0x100_0000 && end > 0x100_0000 {
            0x100_0000
        } else {
            base
        }
    } else {
        0
    };
}
