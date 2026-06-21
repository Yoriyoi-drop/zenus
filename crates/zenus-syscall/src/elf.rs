use zenus_mem::paging;
use zenus_fs::vfs;

const ELF_MAGIC: [u8; 4] = [0x7F, b'E', b'L', b'F'];

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

    let phdrs = unsafe {
        core::slice::from_raw_parts(data.as_ptr().add(phoff) as *const Elf64Phdr, phnum)
    };

    let mut max_addr: u64 = 0;

    for phdr in phdrs {
        if phdr.p_type != PT_LOAD { continue; }
        let vaddr = phdr.p_vaddr & !0xFFF;
        if phdr.p_memsz > u64::MAX - phdr.p_vaddr { return None; }
        let end = phdr.p_vaddr.checked_add(phdr.p_memsz).unwrap_or(u64::MAX);
        let end = (end + 0xFFF) & !0xFFF;
        if end > max_addr { max_addr = end; }

        let file_off = phdr.p_offset as usize;
        let file_sz = phdr.p_filesz as usize;
        let pages = ((end - vaddr) / paging::PAGE_SIZE as u64) as usize;

        for i in 0..pages {
            let page_virt = vaddr + (i as u64) * paging::PAGE_SIZE as u64;
            let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
            let frame_phys = match allocator.alloc_frame() {
                Some(p) => p,
                None => return None,
            };
            drop(allocator);

            let hhdm = paging::hhdm_offset();
            unsafe {
                core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
            }

            if !paging::map_user_page_raw(cr3, page_virt, frame_phys.as_u64(), (phdr.p_flags & PF_W) != 0) {
                return None;
            }

            let page_off = if i == 0 { (phdr.p_vaddr & 0xFFF) as usize } else { 0 };
            let file_start = file_off + i * paging::PAGE_SIZE;
            let copy_start = if i == 0 { 0 } else { file_start.saturating_sub(page_off) };
            let copy_size = if file_sz > copy_start {
                let remaining = file_sz - copy_start;
                let space = paging::PAGE_SIZE - page_off;
                core::cmp::min(remaining, space)
            } else {
                0
            };

            if copy_size > 0 {
                let src_offset = file_start.saturating_sub(page_off);
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
            None => return None,
        };
        drop(allocator);

        let hhdm = paging::hhdm_offset();
        unsafe {
            core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
        }

        if !paging::map_user_page_raw(cr3, stack_virt, frame_phys.as_u64(), true) {
            return None;
        }
    }

    Some(LoadedElf {
        entry: header.e_entry,
        cr3,
        heap_base,
        stack_top,
    })
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

    let mut max_addr: u64 = 0;

    for phdr in phdrs {
        if phdr.p_type != PT_LOAD { continue; }
        let vaddr = phdr.p_vaddr & !0xFFF;
        if phdr.p_memsz > u64::MAX - phdr.p_vaddr { return None; }
        let end = phdr.p_vaddr.checked_add(phdr.p_memsz).unwrap_or(u64::MAX);
        let end = (end + 0xFFF) & !0xFFF;
        if end > max_addr { max_addr = end; }

        let file_off = phdr.p_offset;
        let file_sz = phdr.p_filesz;
        let pages = ((end - vaddr) / paging::PAGE_SIZE as u64) as usize;

        for i in 0..pages {
            let page_virt = vaddr + (i as u64) * paging::PAGE_SIZE as u64;
            let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
            let frame_phys = match allocator.alloc_frame() {
                Some(p) => p,
                None => return None,
            };
            drop(allocator);

            let hhdm = paging::hhdm_offset();
            unsafe {
                core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
            }

            if !paging::map_user_page_raw(cr3, page_virt, frame_phys.as_u64(), (phdr.p_flags & PF_W) != 0) {
                return None;
            }

            let page_off = if i == 0 { (phdr.p_vaddr & 0xFFF) as usize } else { 0 };
            let copy_size = if i == 0 {
                let remaining = file_sz as usize;
                if remaining > paging::PAGE_SIZE - page_off {
                    paging::PAGE_SIZE - page_off
                } else {
                    remaining
                }
            } else {
                let copied_before = (i as u64) * paging::PAGE_SIZE as u64;
                if file_sz > copied_before {
                    let remaining = (file_sz - copied_before) as usize;
                    if remaining > paging::PAGE_SIZE {
                        paging::PAGE_SIZE
                    } else {
                        remaining
                    }
                } else {
                    0
                }
            };

            if copy_size > 0 {
                let read_off = file_off + (i as u64) * paging::PAGE_SIZE as u64 - page_off as u64;
                let dst = (hhdm + frame_phys.as_u64() + page_off as u64) as *mut u8;
                let mut read_buf = alloc::vec::Vec::with_capacity(copy_size);
                read_buf.resize(copy_size, 0);
                if node.fs.read(node.inode, read_off, &mut read_buf).is_none() {
                    return None;
                }
                unsafe {
                    core::ptr::copy_nonoverlapping(read_buf.as_ptr(), dst, copy_size);
                }
            }
        }
    }

    let heap_base = if max_addr < 0x6000_0000_0000 { 0x6000_0000_0000 } else { max_addr + 0x10000 };

    // ASLR: randomize heap base upward by up to 32MB (page-aligned)
    let heap_slide = zenus_arch::random::get_random_page_aligned(0, 32 * 1024 * 1024);
    let heap_base = heap_base.saturating_add(heap_slide);

    // ASLR: randomize stack top downward by up to 8GB (page-aligned)
    let stack_max = 0x0000_7FFF_FFFF_F000u64;
    let stack_min = stack_max.saturating_sub(8u64 * 1024 * 1024 * 1024);
    let stack_top = zenus_arch::random::get_random_page_aligned(stack_min, stack_max);

    let stack_pages = 16;
    for i in 0..stack_pages {
        let stack_virt = stack_top - ((stack_pages - i) as u64) * paging::PAGE_SIZE as u64;
        let mut allocator = zenus_mem::frame_allocator::FRAME_ALLOCATOR.lock();
        let frame_phys = match allocator.alloc_frame() {
            Some(p) => p,
            None => return None,
        };
        drop(allocator);

        let hhdm = paging::hhdm_offset();
        unsafe {
            core::ptr::write_bytes((hhdm + frame_phys.as_u64()) as *mut u8, 0, paging::PAGE_SIZE);
        }

        if !paging::map_user_page_raw(cr3, stack_virt, frame_phys.as_u64(), true) {
            return None;
        }
    }

    Some(LoadedElf {
        entry: header.e_entry,
        cr3,
        heap_base,
        stack_top,
    })
}
