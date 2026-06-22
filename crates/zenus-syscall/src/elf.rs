use zenus_mem::paging;
use zenus_fs::vfs;
use x86_64::PhysAddr;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];
const MAX_ELF_PAGES: usize = 65536;

#[repr(C, packed)]
struct Elf64Header {
    e_ident: [u8; 16],
    e_type: u16,
    e_machine: u16,
    e_version: u32,
    e_entry: u64,
    e_phoff: u64,
    e_shoff: u64,
    e_flags: u32,
    e_ehsize: u16,
    e_phentsize: u16,
    e_phnum: u16,
    e_shentsize: u16,
    e_shnum: u16,
    e_shstrndx: u16,
}

#[repr(C, packed)]
struct Elf64Phdr {
    p_type: u32,
    p_flags: u32,
    p_offset: u64,
    p_vaddr: u64,
    p_paddr: u64,
    p_filesz: u64,
    p_memsz: u64,
    p_align: u64,
}

const PT_LOAD: u32 = 1;
const PF_X: u32 = 1;
const PF_W: u32 = 2;

pub struct LoadedElf {
    pub entry: u64,
    pub cr3: u64,
    pub heap_base: u64,
    pub stack_top: u64,
}

/// Load ELF from a raw byte slice (for embedded/built-in binaries)
pub fn load_elf_raw(data: &[u8], cr3: u64) -> Option<LoadedElf> {
    if data.len() < core::mem::size_of::<Elf64Header>() { return None; }

    let header: &Elf64Header = unsafe { &*(data.as_ptr() as *const Elf64Header) };

    if header.e_ident[..4] != ELF_MAGIC { return None; }
    if header.e_ident[4] != 2 { return None; }
    if header.e_machine != 0x3E { return None; }
    let e_ehsize = header.e_ehsize as usize;
    if e_ehsize != 0 && e_ehsize < core::mem::size_of::<Elf64Header>() { return None; }

    let phoff = header.e_phoff as usize;
    let phentsize = header.e_phentsize as usize;
    let phnum = header.e_phnum as usize;

    if phentsize != core::mem::size_of::<Elf64Phdr>() { return None; }
    if phoff + phnum * phentsize > data.len() { return None; }

    // Validate entry is a canonical user-space virtual address
    if header.e_entry < 0x1000 || header.e_entry >= 0x0000_8000_0000_0000 {
        return None;
    }

    let phdrs = unsafe {
        core::slice::from_raw_parts(data.as_ptr().add(phoff) as *const Elf64Phdr, phnum)
    };

    let hhdm = paging::hhdm_offset();
    if hhdm == 0 {
        return None;
    }

    let mut frames: alloc::vec::Vec<u64> = alloc::vec::Vec::new();

    let mut max_addr: u64 = 0;

    for phdr in phdrs {
        if phdr.p_type != PT_LOAD { continue; }
        let vaddr = phdr.p_vaddr & !0xFFF;
        let end = phdr.p_vaddr.checked_add(phdr.p_memsz)?;
        let end = (end + 0xFFF) & !0xFFF;
        if end > 0x0000_8000_0000_0000 || vaddr > 0x0000_8000_0000_0000 {
            free_frames_raw(&frames);
            return None;
        }
        if end > max_addr { max_addr = end; }

        let file_off = phdr.p_offset as usize;
        let file_sz = phdr.p_filesz as usize;
        let pages = ((end - vaddr) / paging::PAGE_SIZE as u64) as usize;
        if pages > MAX_ELF_PAGES {
            free_frames_raw(&frames);
            return None;
        }

        for i in 0..pages {
            let page_virt = vaddr + (i as u64) * paging::PAGE_SIZE as u64;
            let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
            let frame_phys = match allocator.alloc_frame() {
                Some(p) => p,
                None => {
                    drop(allocator);
                    free_frames_raw(&frames);
                    return None;
                }
            };
            drop(allocator);
            frames.push(frame_phys.as_u64());

            unsafe {
                core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
            }

            if !paging::map_user_page_raw(cr3, page_virt, frame_phys.as_u64(), (phdr.p_flags & PF_W) != 0, (phdr.p_flags & PF_X) != 0) {
                free_frames_raw(&frames);
                return None;
            }
            frames.pop();

            let first_page_off = (phdr.p_vaddr & 0xFFF) as usize;
            let page_off = if i == 0 { first_page_off } else { 0 };
            let copied_before = if i == 0 {
                0usize
            } else {
                let before = (i as u64) * paging::PAGE_SIZE as u64 - first_page_off as u64;
                (before.min(file_sz as u64)) as usize
            };
            let remaining = (file_sz as usize).saturating_sub(copied_before);
            let space = paging::PAGE_SIZE - page_off;
            let copy_size = remaining.min(space);

            if copy_size > 0 {
                let src_offset = file_off + copied_before;
                let dst = (hhdm + frame_phys.as_u64() + page_off as u64) as *mut u8;
                if src_offset + copy_size <= data.len() {
                    unsafe {
                        core::ptr::copy_nonoverlapping(data.as_ptr().add(src_offset), dst, copy_size);
                    }
                }
            }
        }
    }

    let heap_base = if max_addr < 0x6000_0000_0000 { 0x6000_0000_0000 } else { max_addr.saturating_add(0x10000) };

    let heap_slide = zenus_arch::random::get_random_page_aligned(0, 32 * 1024 * 1024);
    let heap_base = heap_base.saturating_add(heap_slide);

    let stack_max = 0x0000_7FFF_FFFF_F000u64;
    let stack_min = stack_max.saturating_sub(8u64 * 1024 * 1024 * 1024);
    let stack_top = zenus_arch::random::get_random_page_aligned(stack_min, stack_max);

    let stack_pages = 16;
    for i in 0..stack_pages {
        let stack_virt = stack_top - ((stack_pages - i) as u64) * paging::PAGE_SIZE as u64;
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame_phys = match allocator.alloc_frame() {
            Some(p) => p,
            None => {
                drop(allocator);
                free_frames_raw(&frames);
                return None;
            }
        };
        drop(allocator);
        frames.push(frame_phys.as_u64());

        unsafe {
            core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
        }

        if !paging::map_user_page_raw(cr3, stack_virt, frame_phys.as_u64(), true, false) {
            free_frames_raw(&frames);
            return None;
        }
        frames.pop();
    }

    Some(LoadedElf {
        entry: header.e_entry,
        cr3,
        heap_base,
        stack_top,
    })
}

/// Load a flat binary (raw .bin) at a fixed virtual address.
/// Used for simple user-mode programs that don't have ELF headers.
pub fn load_flat_binary(data: &[u8], entry: u64, cr3: u64) -> Option<LoadedElf> {
    // Validate entry is a canonical user-space virtual address, not a physical address
    if entry < 0x1000 || entry >= 0x0000_8000_0000_0000 {
        return None;
    }
    let mut dbg = zenus_console::serial::SerialPort::new(0x3F8);
    dbg.write_str("[FLAT] start\n");

    let page_size = paging::PAGE_SIZE as u64;
    let vaddr = entry & !0xFFF;
    let pages_needed = ((data.len() + page_size as usize - 1) / page_size as usize).max(1);

    let hhdm = paging::hhdm_offset();
    if hhdm == 0 {
        dbg.write_str("[FLAT] FATAL: HHDM offset is 0\n");
        return None;
    }

    let mut frames: alloc::vec::Vec<u64> = alloc::vec::Vec::new();

    for i in 0..pages_needed {
        let page_virt = vaddr + (i as u64) * page_size;
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame_phys = match allocator.alloc_frame() {
            Some(p) => p,
            None => {
                dbg.write_str("[FLAT] alloc failed\n");
                drop(allocator);
                free_frames_raw(&frames);
                return None;
            }
        };
        drop(allocator);
        frames.push(frame_phys.as_u64());

        dbg.write_str("[FLAT] frame=");
        dbg.write_hex(frame_phys.as_u64());
        dbg.write_str("\n");

        unsafe {
            core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, page_size as usize);
        }

        let writable = true;
        if !paging::map_user_page_raw(cr3, page_virt, frame_phys.as_u64(), writable, true) {
            dbg.write_str("[FLAT] map failed\n");
            free_frames_raw(&frames);
            return None;
        }
        frames.pop();
        dbg.write_str("[FLAT] mapped\n");

        let copy_start = i * page_size as usize;
        let copy_end = core::cmp::min(copy_start + page_size as usize, data.len());
        if copy_start < copy_end {
            unsafe {
                core::ptr::copy_nonoverlapping(
                    data.as_ptr().add(copy_start),
                    (hhdm + frame_phys.as_u64()) as *mut u8,
                    copy_end - copy_start,
                );
            }
        }
    }
    dbg.write_str("[FLAT] code ok\n");

    let heap_base = 0x6000_0000_0000u64;
    let heap_slide = zenus_arch::random::get_random_page_aligned(0, 32 * 1024 * 1024);
    let heap_base = heap_base.saturating_add(heap_slide);

    let stack_max = 0x0000_7FFF_FFFF_F000u64;
    let stack_min = stack_max.saturating_sub(8u64 * 1024 * 1024 * 1024);
    let stack_top = zenus_arch::random::get_random_page_aligned(stack_min, stack_max);

    let stack_pages = 16;
    dbg.write_str("[FLAT] stack...\n");
    for i in 0..stack_pages {
        let stack_virt = stack_top - ((stack_pages - i) as u64) * page_size;
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame_phys = match allocator.alloc_frame() {
            Some(p) => p,
            None => {
                dbg.write_str("[FLAT] stack alloc failed\n");
                drop(allocator);
                free_frames_raw(&frames);
                return None;
            }
        };
        drop(allocator);
        frames.push(frame_phys.as_u64());

        unsafe {
            core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, page_size as usize);
        }

        if !paging::map_user_page_raw(cr3, stack_virt, frame_phys.as_u64(), true, false) {
            dbg.write_str("[FLAT] stack map failed\n");
            free_frames_raw(&frames);
            return None;
        }
        frames.pop();
    }
    dbg.write_str("[FLAT] done\n");

    Some(LoadedElf {
        entry,
        cr3,
        heap_base,
        stack_top,
    })
}

/// Free frames that were allocated but not yet mapped into the page table.
/// Does NOT destroy the address space — caller is responsible for that.
/// This prevents double-free: destroy_address_space frees mapped frames + page tables,
/// while we only free frames that were never mapped (still in the Vec).
fn free_frames_raw(frames: &[u64]) {
    let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
    for &phys in frames {
        allocator.free_frame(PhysAddr::new(phys));
    }
}

pub fn load_elf(path: &str, cr3: u64) -> Option<LoadedElf> {
    let node = vfs::open(path)?;
    let stat = node.fs.stat(node.inode);
    if stat.size < 64 { return None; }

    let mut header_buf = [0u8; 64];
    node.fs.read(node.inode, 0, &mut header_buf)?;

    if stat.size < core::mem::size_of::<Elf64Header>() as u64 { return None; }
    let header: &Elf64Header = unsafe { &*(header_buf.as_ptr() as *const Elf64Header) };

    if header.e_ident[..4] != ELF_MAGIC { return None; }
    if header.e_ident[4] != 2 { return None; }
    if header.e_machine != 0x3E { return None; }
    let e_ehsize = header.e_ehsize as usize;
    if e_ehsize != 0 && e_ehsize < core::mem::size_of::<Elf64Header>() { return None; }

    // Validate entry is a canonical user-space virtual address
    if header.e_entry < 0x1000 || header.e_entry >= 0x0000_8000_0000_0000 {
        return None;
    }

    let phoff = header.e_phoff;
    let phentsize = header.e_phentsize as usize;
    let phnum = header.e_phnum as usize;

    if phentsize != core::mem::size_of::<Elf64Phdr>() { return None; }

    let phdr_size = phnum * phentsize;
    let mut phdr_buf: alloc::vec::Vec<u8> = alloc::vec::Vec::with_capacity(phdr_size);
    phdr_buf.resize(phdr_size, 0);
    node.fs.read(node.inode, phoff, &mut phdr_buf)?;

    let phdrs = unsafe {
        core::slice::from_raw_parts(phdr_buf.as_ptr() as *const Elf64Phdr, phnum)
    };

    let hhdm = paging::hhdm_offset();
    if hhdm == 0 { return None; }

    let mut frames: alloc::vec::Vec<u64> = alloc::vec::Vec::new();

    let mut max_addr: u64 = 0;

    for phdr in phdrs {
        if phdr.p_type != PT_LOAD { continue; }
        let vaddr = phdr.p_vaddr & !0xFFF;
        let end = phdr.p_vaddr.checked_add(phdr.p_memsz)?;
        let end = (end + 0xFFF) & !0xFFF;
        if end > 0x0000_8000_0000_0000 || vaddr > 0x0000_8000_0000_0000 {
            free_frames_raw(&frames);
            return None;
        }
        if end > max_addr { max_addr = end; }

        let file_off = phdr.p_offset;
        let file_sz = phdr.p_filesz;
        let pages = ((end - vaddr) / paging::PAGE_SIZE as u64) as usize;
        if pages > MAX_ELF_PAGES {
            free_frames_raw(&frames);
            return None;
        }

        for i in 0..pages {
            let page_virt = vaddr + (i as u64) * paging::PAGE_SIZE as u64;
            let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
            let frame_phys = match allocator.alloc_frame() {
                Some(p) => p,
                None => {
                    drop(allocator);
                    free_frames_raw(&frames);
                    return None;
                }
            };
            drop(allocator);
            frames.push(frame_phys.as_u64());

            unsafe {
                core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
            }

            if !paging::map_user_page_raw(cr3, page_virt, frame_phys.as_u64(), (phdr.p_flags & PF_W) != 0, (phdr.p_flags & PF_X) != 0) {
                free_frames_raw(&frames);
                return None;
            }
            frames.pop();

            let first_page_off = (phdr.p_vaddr & 0xFFF) as usize;
            let page_off = if i == 0 { first_page_off } else { 0 };
            let copied_before = if i == 0 {
                0u64
            } else {
                let before = (i as u64) * paging::PAGE_SIZE as u64 - first_page_off as u64;
                before.min(file_sz)
            };
            let remaining = file_sz.saturating_sub(copied_before) as usize;
            let space = paging::PAGE_SIZE - page_off;
            let copy_size = remaining.min(space);

            if copy_size > 0 {
                let read_off = file_off + copied_before;
                let dst = (hhdm + frame_phys.as_u64() + page_off as u64) as *mut u8;
                let mut read_buf = alloc::vec::Vec::with_capacity(copy_size);
                read_buf.resize(copy_size, 0);
                if node.fs.read(node.inode, read_off, &mut read_buf).is_none() {
                    free_frames_raw(&frames);
                    return None;
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(read_buf.as_ptr(), dst, copy_size);
                }
            }
        }
    }

    let heap_base = if max_addr < 0x6000_0000_0000 { 0x6000_0000_0000 } else { max_addr + 0x10000 };

    let heap_slide = zenus_arch::random::get_random_page_aligned(0, 32 * 1024 * 1024);
    let heap_base = heap_base.saturating_add(heap_slide);

    let stack_max = 0x0000_7FFF_FFFF_F000u64;
    let stack_min = stack_max.saturating_sub(8u64 * 1024 * 1024 * 1024);
    let stack_top = zenus_arch::random::get_random_page_aligned(stack_min, stack_max);

    let stack_pages = 16;
    for i in 0..stack_pages {
        let stack_virt = stack_top - ((stack_pages - i) as u64) * paging::PAGE_SIZE as u64;
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame_phys = match allocator.alloc_frame() {
            Some(p) => p,
            None => {
                drop(allocator);
                free_frames_raw(&frames);
                return None;
            }
        };
        drop(allocator);
        frames.push(frame_phys.as_u64());

        unsafe {
            core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
        }

        if !paging::map_user_page_raw(cr3, stack_virt, frame_phys.as_u64(), true, false) {
            free_frames_raw(&frames);
            return None;
        }
        frames.pop();
    }

    Some(LoadedElf {
        entry: header.e_entry,
        cr3,
        heap_base,
        stack_top,
    })
}
