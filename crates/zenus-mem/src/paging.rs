use core::sync::atomic::{AtomicU64, Ordering};
use zenus_console::serial::SerialPort;
use x86_64::structures::paging::{
    FrameAllocator, Mapper, Page, PageTableFlags, PhysFrame, Size4KiB,
    OffsetPageTable, page_table::PageTable,
};
use x86_64::VirtAddr;
use x86_64::PhysAddr;

pub const PAGE_SIZE: usize = 4096;

static HHDM_OFFSET: AtomicU64 = AtomicU64::new(0);
static LEVEL4_PHYS: AtomicU64 = AtomicU64::new(0);
static KERNEL_CR3: AtomicU64 = AtomicU64::new(0);

pub fn init(hhdm_offset: u64) {
    let flags = x86_64::registers::control::Cr4::read();
    unsafe {
        x86_64::registers::control::Cr4::write(
            flags | x86_64::registers::control::Cr4Flags::PAGE_GLOBAL,
        );
    }

    HHDM_OFFSET.store(hhdm_offset, Ordering::Release);

    let cr3_raw = get_level4_addr_raw();
    let cr3_phys = cr3_raw & !0xFFF;
    LEVEL4_PHYS.store(cr3_phys, Ordering::Release);
    KERNEL_CR3.store(cr3_raw, Ordering::Release);
}

pub fn hhdm_offset() -> u64 {
    HHDM_OFFSET.load(Ordering::Acquire)
}

pub fn kernel_cr3() -> u64 {
    KERNEL_CR3.load(Ordering::Acquire)
}

pub fn get_level4_addr() -> VirtAddr {
    VirtAddr::new(get_level4_addr_raw() & !0xFFF)
}

fn get_level4_addr_raw() -> u64 {
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    cr3
}

pub fn set_cr3(cr3_value: u64) {
    unsafe {
        core::arch::asm!("mov cr3, {}", in(reg) cr3_value, options(nostack, preserves_flags));
    }
}

fn with_mapper<F, R>(f: F) -> R
where
    F: FnOnce(&mut OffsetPageTable) -> R,
{
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let offset = VirtAddr::new(hhdm);
    let level4_phys = LEVEL4_PHYS.load(Ordering::Acquire);
    let level4_virt = (level4_phys + hhdm) as *mut PageTable;
    let mut mapper = unsafe { OffsetPageTable::new(&mut *level4_virt, offset) };
    f(&mut mapper)
}

pub fn map_page<A: FrameAllocator<Size4KiB>>(
    virt: VirtAddr,
    phys: PhysAddr,
    flags: PageTableFlags,
    allocator: &mut A,
) {
    with_mapper(|mapper| {
        let page = Page::<Size4KiB>::containing_address(virt);
        let frame = PhysFrame::containing_address(phys);
        unsafe {
            if let Ok(flush) = mapper.map_to(page, frame, flags, allocator) {
                flush.flush();
            }
        }
    })
}

pub fn unmap_page(virt: VirtAddr) {
    with_mapper(|mapper| {
        let page = Page::<Size4KiB>::containing_address(virt);
        if let Ok((frame, flush)) = mapper.unmap(page) {
            flush.flush();
            let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
            allocator.free_frame(frame.start_address());
        }
    })
}

macro_rules! raw_out {
    ($byte:expr) => {
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") $byte, options(nostack, preserves_flags)) }
    };
}

fn raw_hex(val: u64) {
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") b'0', options(nostack, preserves_flags)); }
    unsafe { core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") b'x', options(nostack, preserves_flags)); }
    let mut v = val;
    let mut i = 16;
    while i > 0 {
        i -= 1;
        let nibble = ((v >> (i * 4)) & 0xF) as u8;
        let ch = b"0123456789ABCDEF"[nibble as usize];
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") ch, options(nostack, preserves_flags)); }
    }
}

fn raw_str(s: &str) {
    let p = s.as_ptr();
    let len = s.len();
    let mut i = 0;
    while i < len {
        let byte = unsafe { *p.add(i) };
        if byte == b'\n' {
            unsafe { core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") b'\r', options(nostack, preserves_flags)); }
        }
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3f8u16, in("al") byte, options(nostack, preserves_flags)); }
        i += 1;
    }
}

#[inline(never)]
#[no_mangle]
pub extern "C" fn map_user_page_raw(cr3_phys_raw: u64, virt: u64, phys: u64, writable: bool, executable: bool) -> bool {
    raw_str("[MAP] start\n");

    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    raw_str("[MAP] hhdm=");
    raw_hex(hhdm);
    raw_str("\n");

    raw_str("[MAP] cr3_raw=");
    raw_hex(cr3_phys_raw);
    raw_str(" hhdm=");
    raw_hex(hhdm);
    raw_str("\n");

    let offset = VirtAddr::new(hhdm);
    let cr3_phys = cr3_phys_raw & !0xFFF;
    raw_str("[MAP] cr3_phys=");
    raw_hex(cr3_phys);
    raw_str("\n");

    let pt_virt = (cr3_phys + hhdm) as *mut PageTable;
    raw_str("[MAP] pt_virt=");
    raw_hex(pt_virt as u64);
    raw_str("\n");

    raw_str("[MAP] a\n");
    let mut mapper = unsafe { OffsetPageTable::new(&mut *pt_virt, offset) };
    raw_str("[MAP] b\n");

    raw_str("[MAP] c\n");
    let va = match VirtAddr::try_new(virt) {
        Ok(v) => v,
        Err(_) => {
            raw_str("[MAP] ERROR: VirtAddr::try_new failed for virt=");
            raw_hex(virt);
            raw_str("\n");
            return false;
        }
    };
    raw_str("[MAP] d\n");
    let page = Page::<Size4KiB>::containing_address(va);
    raw_str("[MAP] e\n");

    raw_str("[MAP] f\n");
    raw_hex(virt);
    raw_str(" phys=");
    raw_hex(phys);
    raw_str("\n");

    raw_str("[MAP] g\n");
    let frame = PhysFrame::containing_address(PhysAddr::new(phys));
    raw_str("[MAP] h\n");

    raw_str("[MAP] i\n");
    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    if !executable {
        flags |= PageTableFlags::NO_EXECUTE;
    }

    raw_str("[MAP] j\n");
    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    raw_str("[MAP] k\n");

    raw_str("[MAP] l\n");
    let result = unsafe { mapper.map_to(page, frame, flags, &mut *allocator) };
    raw_str("[MAP] m\n");

    match result {
        Ok(flush) => {
            raw_str("[MAP] n\n");
            flush.flush();
            raw_str("[MAP] o\n");
            return true;
        }
        Err(_) => {
            raw_str("[MAP] err\n");
            false
        },
    }
}

pub fn virt_to_phys_raw(cr3_raw: u64, virt: u64) -> Option<u64> {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let cr3_phys = cr3_raw & !0xFFF;
    let levels = [(4usize, 39), (3, 30), (2, 21), (1, 12)];
    unsafe {
        let mut table_virt = (cr3_phys + hhdm) as *const u64;
        for &(level, shift) in &levels {
            let idx = (virt >> shift) & 0x1FF;
            let entry = *table_virt.add(idx as usize);
            if (entry & 1) == 0 {
                return None;
            }
            if (entry & 0x80) != 0 && level > 1 {
                let page_bits = entry & 0x000FFFFFFFFFFFFF;
                let huge_mask = !((1u64 << shift) - 1);
                return Some((page_bits & huge_mask) | (virt & !huge_mask));
            }
            let next = entry & 0x000FFFFFFFFFF000;
            if level == 1 {
                return Some(next | (virt & 0xFFF));
            }
            table_virt = (next + hhdm) as *const u64;
        }
    }
    None
}

pub fn create_address_space() -> Option<u64> {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let cr3_phys = LEVEL4_PHYS.load(Ordering::Acquire);

    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    let new_frame = allocator.alloc_frame()?;
    drop(allocator);

    let src = (cr3_phys + hhdm) as *const PageTable;
    let dst = (new_frame.as_u64() + hhdm) as *mut PageTable;

    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, 1);
        let entries = core::slice::from_raw_parts_mut(dst as *mut u64, 512);
        for entry in entries.iter_mut().take(256) {
            *entry = 0;
        }
    }

    let flags = get_level4_addr_raw() & (0b11000u64);
    Some(new_frame.as_u64() | flags)
}

/// Walk the user-space page table and free all mapped frames and page table pages.
/// Frees: all PT-level frame mappings, intermediate PDPT/PD/PT pages, and the PML4 page.
/// Only walks user space (entries 0-255 of PML4).
///
/// SAFETY:
/// - Must not be called on the currently active address space on THIS CPU (unless
///   it is the kernel's). Automatically switches to kernel CR3 if the target matches.
/// - SMP: caller must ensure no other CPU has this CR3 loaded. Since tasks are pinned
///   to CPUs and destroy_address_space is called from task_exit() on the task's own CPU,
///   this is safe. No cross-CPU TLB shootdown is performed.
pub fn destroy_address_space(cr3_raw: u64) {
    let current_cr3 = get_level4_addr_raw() & !0xFFF;
    let cr3_phys = cr3_raw & !0xFFF;

    // Never free the kernel's address space
    let kernel_cr3_phys = KERNEL_CR3.load(Ordering::Acquire) & !0xFFF;
    if cr3_phys == kernel_cr3_phys {
        return;
    }

    // cr3_phys == 0 means no address space was allocated; nothing to free
    if cr3_phys == 0 {
        return;
    }

    // If freeing the currently active address space, switch to kernel CR3 first
    if cr3_phys == current_cr3 {
        set_cr3(kernel_cr3_phys);
    }

    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();

    // PML4 (level 4)
    let pml4_virt = (cr3_phys + hhdm) as *const u64;
    for pml4_idx in 0..256 {
        let pml4_entry = unsafe { *pml4_virt.add(pml4_idx) };
        if (pml4_entry & 1) == 0 {
            continue;
        }
        let pdpt_phys = pml4_entry & 0x000FFFFFFFFFF000;
        if (pml4_entry & 0x80) != 0 {
            // 1 GiB huge page — free all 262144 constituent 4K frames
            let base = pdpt_phys;
            for off in (0..0x4000_0000u64).step_by(4096) {
                allocator.free_frame(x86_64::PhysAddr::new(base + off));
            }
            continue;
        }

        // PDPT (level 3)
        let pdpt_virt = (pdpt_phys + hhdm) as *const u64;
        for pdpt_idx in 0..512 {
            let pdpt_entry = unsafe { *pdpt_virt.add(pdpt_idx) };
            if (pdpt_entry & 1) == 0 {
                continue;
            }
            let pd_phys = pdpt_entry & 0x000FFFFFFFFFF000;
            if (pdpt_entry & 0x80) != 0 {
                // 2 MiB huge page — free all 512 constituent 4K frames
                let base = pd_phys;
                for off in (0..0x20_0000u64).step_by(4096) {
                    allocator.free_frame(x86_64::PhysAddr::new(base + off));
                }
                continue;
            }

            // PD (level 2)
            let pd_virt = (pd_phys + hhdm) as *const u64;
            for pd_idx in 0..512 {
                let pd_entry = unsafe { *pd_virt.add(pd_idx) };
                if (pd_entry & 1) == 0 {
                    continue;
                }
                let pt_phys = pd_entry & 0x000FFFFFFFFFF000;
                if (pd_entry & 0x80) != 0 {
                    // 4 KiB page (PS bit at PD level)
                    allocator.free_frame(x86_64::PhysAddr::new(pt_phys));
                    continue;
                }

                // PT (level 1)
                let pt_virt = (pt_phys + hhdm) as *const u64;
                for pt_idx in 0..512 {
                    let pt_entry = unsafe { *pt_virt.add(pt_idx) };
                    if (pt_entry & 1) == 0 {
                        continue;
                    }
                    let frame_phys = pt_entry & 0x000FFFFFFFFFF000;
                    allocator.free_frame(x86_64::PhysAddr::new(frame_phys));
                }
                // Free the PT frame itself
                allocator.free_frame(x86_64::PhysAddr::new(pt_phys));
            }
            // Free the PD frame itself
            allocator.free_frame(x86_64::PhysAddr::new(pd_phys));
        }
        // Free the PDPT frame itself
        allocator.free_frame(x86_64::PhysAddr::new(pdpt_phys));
    }
    // Free the PML4 frame itself
    allocator.free_frame(x86_64::PhysAddr::new(cr3_phys));
}

#[cfg(feature = "testing")]
pub mod tests {
    use super::PAGE_SIZE;

    pub fn test_page_size_value() -> Result<(), &'static str> {
        if PAGE_SIZE != 4096 {
            return Err("PAGE_SIZE should be 4096");
        }
        Ok(())
    }

    pub fn test_page_size_is_power_of_two() -> Result<(), &'static str> {
        if PAGE_SIZE == 0 || (PAGE_SIZE & (PAGE_SIZE - 1)) != 0 {
            return Err("PAGE_SIZE should be a power of 2");
        }
        Ok(())
    }

    pub fn test_page_size_aligned() -> Result<(), &'static str> {
        if PAGE_SIZE % 4096 != 0 {
            return Err("PAGE_SIZE should be 4K-aligned");
        }
        Ok(())
    }
}
