use core::sync::atomic::{AtomicU64, Ordering};
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

    HHDM_OFFSET.store(hhdm_offset, Ordering::Relaxed);

    let cr3_raw = get_level4_addr_raw();
    let cr3_phys = cr3_raw & !0xFFF;
    LEVEL4_PHYS.store(cr3_phys, Ordering::Relaxed);
    KERNEL_CR3.store(cr3_raw, Ordering::Relaxed);
}

pub fn hhdm_offset() -> u64 {
    HHDM_OFFSET.load(Ordering::Relaxed)
}

pub fn kernel_cr3() -> u64 {
    KERNEL_CR3.load(Ordering::Relaxed)
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
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let offset = VirtAddr::new(hhdm);
    let level4_phys = LEVEL4_PHYS.load(Ordering::Relaxed);
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
            mapper
                .map_to(page, frame, flags, allocator)
                .unwrap()
                .flush();
        }
    })
}

pub fn unmap_page(virt: VirtAddr) {
    with_mapper(|mapper| {
        let page = Page::<Size4KiB>::containing_address(virt);
        let (_, flush) = mapper.unmap(page).unwrap();
        flush.flush();
    })
}

pub fn map_user_page_raw(cr3_phys_raw: u64, virt: u64, phys: u64, writable: bool) -> bool {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let offset = VirtAddr::new(hhdm);
    let cr3_phys = cr3_phys_raw & !0xFFF;
    let pt_virt = (cr3_phys + hhdm) as *mut PageTable;
    let mut mapper = unsafe { OffsetPageTable::new(&mut *pt_virt, offset) };

    let page = Page::<Size4KiB>::containing_address(VirtAddr::new(virt));
    let frame = PhysFrame::containing_address(PhysAddr::new(phys));

    let mut flags =
        PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE | PageTableFlags::NO_EXECUTE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }

    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    match unsafe { mapper.map_to(page, frame, flags, &mut *allocator) } {
        Ok(flush) => {
            flush.flush();
            true
        }
        Err(_) => false,
    }
}

pub fn virt_to_phys_raw(cr3_raw: u64, virt: u64) -> Option<u64> {
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
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
    let hhdm = HHDM_OFFSET.load(Ordering::Relaxed);
    let cr3_phys = LEVEL4_PHYS.load(Ordering::Relaxed);

    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    let new_frame = allocator.alloc_frame()?;
    drop(allocator);

    let src = (cr3_phys + hhdm) as *const PageTable;
    let dst = (new_frame.as_u64() + hhdm) as *mut PageTable;

    unsafe {
        core::ptr::copy_nonoverlapping(src, dst, 1);
        let entries = core::slice::from_raw_parts_mut(dst as *mut u64, 512);
        for i in 0..256 {
            entries[i] = 0;
        }
    }

    let flags = get_level4_addr_raw() & 0xFFF;
    Some(new_frame.as_u64() | flags)
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
