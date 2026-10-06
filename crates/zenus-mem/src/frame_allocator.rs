use x86_64::structures::paging::{FrameAllocator as FrameAllocatorTrait, PhysFrame, Size4KiB};
use x86_64::PhysAddr;
use zenus_sync::spinlock::SpinLock;

use crate::paging::PAGE_SIZE;

const FREE_STACK_SIZE: usize = 16384;

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

        zenus_console::kinfo!(
            "Memory: {} MB total",
            allocator.total_memory / (1024 * 1024)
        );

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

    /// Return a frame to the allocator.
    ///
    /// The frame goes on `free_stack`, and `alloc_frame` pops from there before
    /// it bumps `next_free`. Without that, every frame ever handed out is gone
    /// for good: the bump pointer only ever moves forward.
    ///
    /// ## What used to be here, and why it was wrong
    ///
    /// This used to refuse any frame that fell inside a known region, on the
    /// theory that this was a double-free guard. It is not one. Regions describe
    /// memory *available to be handed out*, which is every frame this allocator
    /// ever returned — so the guard rejected every legitimate free, `free_stack`
    /// stayed permanently empty, and `used_memory` (decremented *after* the
    /// guard, so also never reached) kept counting. `meminfo` reported
    /// "Used: 0 frames" for a whole boot no matter how much had been mapped, and
    /// "Free stack: 0 frames" on a machine with 2 GiB spare.
    ///
    /// Membership in `free_stack` is the guard that can actually tell a double
    /// free from a legitimate re-free: a frame that was freed, re-allocated and
    /// freed again is legitimately *absent* from the stack the second time.
    pub fn free_frame(&mut self, addr: PhysAddr) {
        let a = addr.as_u64();
        // Not a frame address; handing one out would give a caller a page whose
        // neighbours are whatever happens to be there.
        if a == 0 || a & (PAGE_SIZE_U64 - 1) != 0 {
            return;
        }
        // Already on the stack: a double free. Returning here also keeps
        // `used_memory` from being decremented twice for one frame.
        if self.free_stack[..self.free_count].contains(&a) {
            return;
        }
        if self.free_count < FREE_STACK_SIZE {
            self.free_stack[self.free_count] = a;
            self.free_count += 1;
            self.used_memory = self.used_memory.saturating_sub(PAGE_SIZE_U64);
            return;
        }
        // The stack is full. Appending a one-page *region* and lowering
        // `next_free` to reach it looked like a way to keep the memory, but
        // `next_free` is the high-water mark of what has been handed out, so
        // lowering it re-offers frames that are still live. Dropping the frame is
        // the honest option; the stack is 16384 entries.
        zenus_console::kwarn!(
            "Frame free stack full, dropping frame {:#x} ({} entries)",
            a,
            FREE_STACK_SIZE
        );
    }

    pub fn used_memory(&self) -> u64 {
        self.used_memory
    }
    pub fn total_memory(&self) -> u64 {
        self.total_memory
    }
    pub fn free_frames_count(&self) -> usize {
        self.free_count
    }

    /// Clear all entries from the free stack.
    /// Used before loading user programs to prevent stale frame reuse
    /// from previous address space destruction.
    pub fn clear_free_stack(&mut self) {
        self.free_count = 0;
    }

    pub fn reserve_region(&mut self, base: u64, length: u64) {
        if length == 0 {
            return;
        }
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
                self.regions[i] = MemRegion {
                    base: end,
                    length: r_end - end,
                };
                i += 1;
                continue;
            }
            // Overlap at end
            if end >= r_end {
                self.regions[i] = MemRegion {
                    base: r.base,
                    length: base - r.base,
                };
                i += 1;
                continue;
            }
            // Split: kernel region in the middle
            self.regions[i] = MemRegion {
                base: r.base,
                length: base - r.base,
            };
            if self.region_count < MAX_REGIONS {
                let mut j = self.region_count;
                while j > i + 1 {
                    self.regions[j] = self.regions[j - 1];
                    j -= 1;
                }
                self.regions[i + 1] = MemRegion {
                    base: end,
                    length: r_end - end,
                };
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
        self.alloc_frame()
            .map(|addr| PhysFrame::containing_address(addr))
    }
}

/// Reserve the physical pages that contain the kernel's boot stack (provided by
/// the bootloader, not tracked in the memory map). This must be called after
/// `global_init()` and before any frame allocator user that might get a stack page.
///
/// The kernel's initialization path (PCI, ATA, network, VFS, namespaces, etc.)
/// uses a substantial amount of stack space. After init, the shell runs directly
/// on the boot stack with interrupts enabled. When the PIT timer ISR fires
/// during `sti; hlt; cli` in the shell's readline loop, ~1-2 KiB of additional
/// stack is consumed by the ISR and its callees (schedule_tick, flush_output,
/// pit::tick, etc.). A 512 KiB reservation ensures the ISR never overflows the
/// stack and corrupts the shell's local variables.
pub fn reserve_boot_stack(hhdm_offset: u64) {
    let rsp: u64;
    unsafe {
        core::arch::asm!("mov {}, rsp", out(reg) rsp, options(nostack, preserves_flags));
    }
    let rsp_phys = rsp.wrapping_sub(hhdm_offset);
    // Round down to page boundary, then reserve 512 KiB (128 pages) below current RSP
    let stack_page_base = (rsp_phys - 524288) & !0xFFF;
    let mut fa = FRAME_ALLOCATOR.lock();
    fa.reserve_region(stack_page_base, 524288 + PAGE_SIZE as u64);
}

/// Excluded from the frame allocator so it can never hand out a page the kernel
/// is still using. `__kernel_start`/`__kernel_end` come from `linker.ld` and
/// cover `.text`, `.rodata`, `.data` **and** `.bss`.
extern "C" {
    static __kernel_start: u8;
    static __kernel_end: u8;
}

/// Reserve the kernel's own image.
///
/// Nothing in the tree reserved it. Limine knows where it loaded the kernel and
/// leaves those pages out of the *usable* map, but that is the bootloader's
/// bookkeeping: `.bss` in particular is not in the file Limine read (it is
/// zeroed at load time), so any static living there depends entirely on
/// Limine having reserved the right range.
///
/// The failure mode is quiet and severe: a frame handed out that overlaps a
/// static makes every later write through it land somewhere else. `make test`
/// died exactly that way — a write to `0xffffffff8054a9e8`, about a kilobyte past
/// `console::error::ERR_BUF`, with the boot stack unreadable at the same time.
pub fn reserve_kernel_image() {
    let start = unsafe { core::ptr::addr_of!(__kernel_start) as u64 } & !PAGE_SIZE_U64;
    let end = unsafe { core::ptr::addr_of!(__kernel_end) as u64 };
    if end <= start {
        return;
    }
    FRAME_ALLOCATOR.lock().reserve_region(start, end - start);
}

const PAGE_SIZE_U64: u64 = 4096;

pub fn global_init(memory_map: &[MemoryRegion], hhdm_offset: u64) {
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
    // The kernel's own image too. This has to happen *here*, inside
    // `global_init`, not as a `reserve_region` call afterwards: the reservation
    // only shrinks the region list, and the free stack is filled from those
    // regions further down. A frame that was already queued when the
    // reservation lands is still handed out later, so reserving the image after
    // the fact protects nothing.
    {
        let start = unsafe { core::ptr::addr_of!(__kernel_start) as u64 } & !PAGE_SIZE_U64;
        let end = unsafe { core::ptr::addr_of!(__kernel_end) as u64 };
        if end > start {
            fa.reserve_region(start, end - start);
        }
    }
    // The boot stack, for the same reason as the image above: `reserve_boot_stack`
    // runs after this function, by which point the free stack is already full of
    // frames taken from the very region the boot stack is standing in.
    {
        let rsp_phys: u64;
        unsafe {
            core::arch::asm!("mov {}, rsp", out(reg) rsp_phys, options(nostack, preserves_flags));
        }
        let rsp_phys = rsp_phys.wrapping_sub(hhdm_offset);
        let stack_page_base = rsp_phys.saturating_sub(524288) & !PAGE_SIZE_U64;
        fa.reserve_region(stack_page_base, 524288 + PAGE_SIZE_U64);
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
                if candidate > kernel_end {
                    kernel_end = candidate;
                }
            }
        }
        if start < kernel_end {
            kernel_end
        } else {
            start
        }
    } else {
        0
    };
}
