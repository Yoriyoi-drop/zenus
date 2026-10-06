use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::structures::paging::{
    page_table::PageTable, FrameAllocator, Mapper, OffsetPageTable, Page, PageTableFlags,
    PhysFrame, Size4KiB,
};
use x86_64::PhysAddr;
use x86_64::VirtAddr;

pub const PAGE_SIZE: usize = 4096;

/// Bytes in one x86-64 page table: 512 8-byte entries = one 4 KiB page.
///
/// This exists so the page-table zero-fills are written *in bytes* against a
/// `*mut u8`. `write_bytes` counts **elements**, and that is exactly the bug
/// this replaces: `write_bytes(ptr, 0, 512 * 8)` on a `*mut u64` zeroes 4096
/// elements = **32 KiB**, not 4 KiB. `clone_user_address_space` did that for
/// every PDPT, PD and PT it allocated, so each `fork` zeroed 24 KiB past the
/// end of a page table it had just taken from the frame allocator — straight
/// over whatever had been handed out next. Host-tested by
/// `a_page_table_is_exactly_one_page`.
pub const fn page_table_bytes() -> usize {
    512 * core::mem::size_of::<u64>()
}
const MAX_FREED_CR3: usize = 64;

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

/// Walk ALL 4 levels of ALL present PML4 entries (entries 0-511) and
/// clear the U/S (User/Supervisor) bit from EVERY entry. This covers
/// both the kernel half (entries 256-511) AND any identity-mapped or
/// lower memory entries (0-255) that Limine may have set up.
///
/// Per Intel SDM Vol 3 §4.10.4.1, SMEP considers a page user-mode if
/// U/S=1 in ANY level — so we clear U/S at ALL 4 levels.
///
/// Must be called AFTER interrupts::init() so the IDT can catch faults,
/// and BEFORE enable_smep_smap().
pub fn ensure_kernel_pages_supervisor() {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    if hhdm == 0 {
        return;
    }
    let cr3_phys = KERNEL_CR3.load(Ordering::Acquire) & !0xFFF;
    if cr3_phys == 0 {
        return;
    }

    const US_BIT: u64 = 1u64 << 2;

    unsafe {
        let pml4_virt = (cr3_phys + hhdm) as *mut u64;

        // Walk ALL 512 PML4 entries — Limine may set up identity-mapped
        // pages in entries 0-255 that the kernel currently executes from.
        for pml4_idx in 0..512 {
            let pml4e = *pml4_virt.add(pml4_idx);
            if (pml4e & 1) == 0 {
                continue;
            }
            *pml4_virt.add(pml4_idx) = pml4e & !US_BIT;

            if (pml4e & 0x80) != 0 {
                continue;
            } // 1 GiB huge page

            let pdpt_phys = pml4e & 0x000FFFFFFFFFF000;
            let pdpt_virt = (pdpt_phys + hhdm) as *mut u64;

            for pdpt_idx in 0..512 {
                let pdpte = *pdpt_virt.add(pdpt_idx);
                if (pdpte & 1) == 0 {
                    continue;
                }
                *pdpt_virt.add(pdpt_idx) = pdpte & !US_BIT;

                if (pdpte & 0x80) != 0 {
                    continue;
                } // 2 MiB huge page

                let pd_phys = pdpte & 0x000FFFFFFFFFF000;
                let pd_virt = (pd_phys + hhdm) as *mut u64;

                for pd_idx in 0..512 {
                    let pde = *pd_virt.add(pd_idx);
                    if (pde & 1) == 0 {
                        continue;
                    }
                    *pd_virt.add(pd_idx) = pde & !US_BIT;

                    if (pde & 0x80) != 0 {
                        continue;
                    } // 2 MiB page

                    let pt_phys = pde & 0x000FFFFFFFFFF000;
                    let pt_virt = (pt_phys + hhdm) as *mut u64;

                    for pt_idx in 0..512 {
                        let pte = *pt_virt.add(pt_idx);
                        if (pte & 1) == 0 {
                            continue;
                        }
                        *pt_virt.add(pt_idx) = pte & !US_BIT;
                    }
                }
            }
        }
    }

    // Flush ALL TLB entries (including global)
    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
        cr4 &= !(1u64 << 7); // Clear PGE
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
        set_cr3(get_level4_addr_raw());
        cr4 |= 1u64 << 7; // Set PGE
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
    }
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

#[inline(never)]
#[no_mangle]
pub extern "C" fn map_user_page_raw(
    cr3_phys_raw: u64,
    virt: u64,
    phys: u64,
    writable: bool,
    executable: bool,
) -> bool {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let offset = VirtAddr::new(hhdm);
    let cr3_phys = cr3_phys_raw & !0xFFF;
    let pt_virt = (cr3_phys + hhdm) as *mut PageTable;
    let mut mapper = unsafe { OffsetPageTable::new(&mut *pt_virt, offset) };

    let va = match VirtAddr::try_new(virt) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let page = Page::<Size4KiB>::containing_address(va);
    let frame = PhysFrame::containing_address(PhysAddr::new(phys));

    let mut flags = PageTableFlags::PRESENT | PageTableFlags::USER_ACCESSIBLE;
    if writable {
        flags |= PageTableFlags::WRITABLE;
    }
    if !executable {
        flags |= PageTableFlags::NO_EXECUTE;
    }

    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    let result = unsafe { mapper.map_to(page, frame, flags, &mut *allocator) };

    match result {
        Ok(flush) => {
            flush.flush();
            return true;
        }
        Err(_) => false,
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

/// Change page table flags for a virtual address in an arbitrary address space.
/// Returns true if successful.
pub fn protect_page_raw(cr3_raw: u64, virt: u64, writable: bool, executable: bool) -> bool {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let cr3_phys = cr3_raw & !0xFFF;
    let levels = [(4usize, 39), (3, 30), (2, 21), (1, 12)];
    unsafe {
        let mut table_virt = (cr3_phys + hhdm) as *mut u64;
        for &(level, shift) in &levels {
            let idx = (virt >> shift) & 0x1FF;
            let entry = *table_virt.add(idx as usize);
            if (entry & 1) == 0 {
                return false;
            }
            if level == 1 {
                let mut new_entry = entry & !(1u64 << 1) & !(1u64 << 63);
                if writable {
                    new_entry |= 1u64 << 1;
                }
                if !executable {
                    new_entry |= 1u64 << 63;
                }
                table_virt.add(idx as usize).write(new_entry);
                core::arch::asm!("invlpg [{0}]", in(reg) virt, options(nostack, preserves_flags));
                return true;
            }
            let next = entry & 0x000FFFFFFFFFF000;
            table_virt = (next + hhdm) as *mut u64;
        }
    }
    false
}

/// The four page-table indices of a 48-bit canonical virtual address, most
/// significant first.
///
/// Pure. Every raw page-table walk in this module indexes with the same four
/// shifts, and they are easy to get subtly wrong — a missing level means the
/// walk stops one level early and silently reads the wrong table.
pub fn split_indices(virt: u64) -> [u16; 4] {
    [
        ((virt >> 39) & 0x1FF) as u16,
        ((virt >> 30) & 0x1FF) as u16,
        ((virt >> 21) & 0x1FF) as u16,
        ((virt >> 12) & 0x1FF) as u16,
    ]
}

/// Remove the PTE for `virt` in an arbitrary address space **without**
/// releasing the physical frame.
///
/// `invlpg` on its own is not an unmap: it invalidates the TLB entry and
/// leaves the PTE present, still pointing at the frame. If the frame is then
/// handed to somebody else, the next access through that address reads their
/// memory — a use-after-free across tasks, with the page table still claiming
/// the page is mapped. This clears the entry first, then flushes it.
pub fn unmap_page_raw_keep_frame(cr3_raw: u64, virt: u64) -> bool {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let cr3_phys = cr3_raw & !0xFFF;
    let idxs = split_indices(virt);
    unsafe {
        let mut table_virt = (cr3_phys + hhdm) as *mut u64;
        for (level, &idx) in idxs.iter().enumerate() {
            let entry = *table_virt.add(idx as usize);
            if (entry & 1) == 0 {
                return false;
            }
            if level == 3 {
                // The leaf. `x86_64` makes the store visible before the
                // invalidation below; without it the CPU is allowed to service
                // the very access we are trying to kill from the old TLB entry.
                core::sync::atomic::fence(Ordering::SeqCst);
                table_virt.add(idx as usize).write(0);
                core::arch::asm!("invlpg [{0}]", in(reg) virt, options(nostack, preserves_flags));
                return true;
            }
            table_virt = ((entry & 0x000FFFFFFFFFF000) + hhdm) as *mut u64;
        }
    }
    false
}

/// Remove the PTE for `virt` in an arbitrary address space **and** release the
/// frame it pointed at.
///
/// The counterpart to [`unmap_page_raw_keep_frame`], for pages the caller owns.
/// `sys_mmap` allocates the frames itself, so a mapping it has to abandon must
/// hand them back: leaking them is what turns one over-large `mmap` into a
/// permanently broken machine.
pub fn unmap_page_raw(cr3_raw: u64, virt: u64) -> bool {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);
    let cr3_phys = cr3_raw & !0xFFF;
    let idxs = split_indices(virt);
    unsafe {
        let mut table_virt = (cr3_phys + hhdm) as *mut u64;
        for (level, &idx) in idxs.iter().enumerate() {
            let entry = *table_virt.add(idx as usize);
            if (entry & 1) == 0 {
                return false;
            }
            if level == 3 {
                let frame = entry & 0x000FFFFFFFFFF000;
                // Same ordering requirement as the keep-frame variant: the store
                // has to be visible before the TLB entry is invalidated.
                core::sync::atomic::fence(Ordering::SeqCst);
                table_virt.add(idx as usize).write(0);
                core::arch::asm!("invlpg [{0}]", in(reg) virt, options(nostack, preserves_flags));
                let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
                allocator.free_frame(PhysAddr::new(frame));
                return true;
            }
            table_virt = ((entry & 0x000FFFFFFFFFF000) + hhdm) as *mut u64;
        }
    }
    false
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

/// Clone a user address space by copying all mapped user pages.
/// Read-only pages are shared (same physical frame in both parent and child).
/// Writable pages are full-copied (new frame, same content).
/// The parent's page tables are NOT modified.
/// Returns the physical address of the new PML4.
pub fn clone_user_address_space(source_cr3_raw: u64) -> Option<u64> {
    let hhdm = HHDM_OFFSET.load(Ordering::Acquire);

    let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
    let new_pml4_frame = allocator.alloc_frame()?;
    drop(allocator);

    let source_cr3_phys = source_cr3_raw & !0xFFF;
    let new_cr3_phys = new_pml4_frame.as_u64() & !0xFFF;

    // Copy kernel half (entries 256-511) from kernel CR3 (shared with kernel)
    let kernel_cr3_phys = KERNEL_CR3.load(Ordering::Acquire) & !0xFFF;
    let dst_pml4 = (new_cr3_phys + hhdm) as *mut u64;
    let src_pml4 = (source_cr3_phys + hhdm) as *const u64;

    unsafe {
        // Zero user half, copy kernel half
        core::ptr::write_bytes(dst_pml4, 0, 256);
        core::ptr::copy_nonoverlapping(
            (kernel_cr3_phys + hhdm) as *const u64,
            dst_pml4.add(256),
            256,
        );
    }

    // Walk user space (entries 0-255) of source and clone
    for pml4_idx in 0..256 {
        let pml4_entry = unsafe { *src_pml4.add(pml4_idx) };
        if (pml4_entry & 1) == 0 {
            continue;
        }

        let pdpt_phys = pml4_entry & 0x000FFFFFFFFFF000;
        let pml4_flags = pml4_entry & 0xFFF;

        // Allocate new PDPT for child
        let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
        let new_pdpt_frame = allocator.alloc_frame()?;
        drop(allocator);
        let new_pdpt_phys = new_pdpt_frame.as_u64() & !0xFFF;
        let new_pdpt_virt = (new_pdpt_phys + hhdm) as *mut u64;
        unsafe {
            core::ptr::write_bytes(new_pdpt_virt as *mut u8, 0, page_table_bytes());
        }

        let pdpt_virt = (pdpt_phys + hhdm) as *const u64;

        for pdpt_idx in 0..512 {
            let pdpt_entry = unsafe { *pdpt_virt.add(pdpt_idx) };
            if (pdpt_entry & 1) == 0 {
                continue;
            }

            let pd_phys = pdpt_entry & 0x000FFFFFFFFFF000;
            let pdpt_flags = pdpt_entry & 0xFFF;

            // Allocate new PD for child
            let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
            let new_pd_frame = allocator.alloc_frame()?;
            drop(allocator);
            let new_pd_phys = new_pd_frame.as_u64() & !0xFFF;
            let new_pd_virt = (new_pd_phys + hhdm) as *mut u64;
            unsafe {
                core::ptr::write_bytes(new_pd_virt as *mut u8, 0, page_table_bytes());
            }

            let pd_virt = (pd_phys + hhdm) as *const u64;

            for pd_idx in 0..512 {
                let pd_entry = unsafe { *pd_virt.add(pd_idx) };
                if (pd_entry & 1) == 0 {
                    continue;
                }

                let pt_phys = pd_entry & 0x000FFFFFFFFFF000;
                let _pd_flags = pd_entry & 0xFFF;

                // Allocate new PT for child
                let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
                let new_pt_frame = allocator.alloc_frame()?;
                drop(allocator);
                let new_pt_phys = new_pt_frame.as_u64() & !0xFFF;
                let new_pt_virt = (new_pt_phys + hhdm) as *mut u64;
                unsafe {
                    core::ptr::write_bytes(new_pt_virt as *mut u8, 0, page_table_bytes());
                }

                let pt_virt = (pt_phys + hhdm) as *const u64;

                for pt_idx in 0..512 {
                    let pt_entry = unsafe { *pt_virt.add(pt_idx) };
                    if (pt_entry & 1) == 0 {
                        continue;
                    }

                    let frame_phys = pt_entry & 0x000FFFFFFFFFF000;
                    let writable = (pt_entry & 2) != 0;

                    if writable {
                        let mut allocator = crate::frame_allocator::FRAME_ALLOCATOR.lock();
                        let new_frame = match allocator.alloc_frame() {
                            Some(f) => f,
                            None => return None,
                        };
                        drop(allocator);

                        unsafe {
                            core::ptr::copy_nonoverlapping(
                                (hhdm + frame_phys) as *const u8,
                                (hhdm + new_frame.as_u64()) as *mut u8,
                                4096,
                            );
                        }

                        let new_entry = (new_frame.as_u64() & !0xFFF) | (pt_entry & 0xFFF);
                        unsafe {
                            new_pt_virt.add(pt_idx).write(new_entry);
                        }
                    } else {
                        unsafe {
                            new_pt_virt.add(pt_idx).write(pt_entry);
                        }
                    }
                }

                // Link PT into child's PD (copy flags from source PD entry)
                let new_pd_entry = (new_pt_phys & !0xFFF) | (_pd_flags & 0xFFF & !0x1);
                unsafe {
                    new_pd_virt.add(pd_idx).write(new_pd_entry | 1);
                }
            }

            // Link PD into child's PDPT
            let new_pdpt_entry = (new_pd_phys & !0xFFF) | (pdpt_flags & 0xFFF & !0x1);
            unsafe {
                new_pdpt_virt.add(pdpt_idx).write(new_pdpt_entry | 1);
            }
        }

        // Link PDPT into child's PML4
        let new_pml4_entry = (new_pdpt_phys & !0xFFF) | (pml4_flags & 0xFFF & !0x1);
        unsafe {
            dst_pml4.add(pml4_idx).write(new_pml4_entry | 1);
        }
    }

    let flags = source_cr3_raw & (0b11000u64);
    Some(new_cr3_phys | flags)
}

/// Tracks freed address spaces to prevent double-free.
static FREED_CR3_COUNT: AtomicU64 = AtomicU64::new(0);
static FREED_CR3_LIST: [AtomicU64; MAX_FREED_CR3] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];

fn is_already_freed(cr3_phys: u64) -> bool {
    let count = FREED_CR3_COUNT.load(Ordering::Acquire) as usize;
    let count = count.min(MAX_FREED_CR3);
    for i in 0..count {
        if FREED_CR3_LIST[i].load(Ordering::Acquire) == cr3_phys {
            return true;
        }
    }
    false
}

fn mark_freed(cr3_phys: u64) {
    let idx = FREED_CR3_COUNT.fetch_add(1, Ordering::Release) as usize;
    if idx < MAX_FREED_CR3 {
        FREED_CR3_LIST[idx].store(cr3_phys, Ordering::Release);
    }
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

    // Double-free guard: skip if already freed
    if is_already_freed(cr3_phys) {
        return;
    }
    mark_freed(cr3_phys);

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
