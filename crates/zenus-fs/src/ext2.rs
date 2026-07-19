use crate::vfs::{FileSystem, FileType, FileStat, DirEntry};
use crate::block_cache::bc_read;
use zenus_sync::spinlock::SpinLock;

pub(crate) const EXT2_MAGIC: u16 = 0xEF53;
const ROOT_INODE: u64 = 2;

const EXT2_GOOD_OLD_REV: u32 = 0;
const EXT2_DYNAMIC_REV: u32 = 1;

const EXT2_FT_DIR: u8 = 2;

const MAX_EXT2_INSTANCES: usize = 4;

static EXT2_POOL: SpinLock<[Option<Ext2Fs>; MAX_EXT2_INSTANCES]> =
    SpinLock::new([None; MAX_EXT2_INSTANCES]);

static NEXT_EXT2_ID: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct RawSuperblock {
    pub inodes_count: u32,
    pub blocks_count: u32,
    pub r_blocks_count: u32,
    pub free_blocks_count: u32,
    pub free_inodes_count: u32,
    pub first_data_block: u32,
    pub log_block_size: u32,
    pub log_frag_size: u32,
    pub blocks_per_group: u32,
    pub frags_per_group: u32,
    pub inodes_per_group: u32,
    pub mtime: u32,
    pub wtime: u32,
    pub mnt_count: u16,
    pub max_mnt_count: u16,
    pub magic: u16,
    pub state: u16,
    pub errors: u16,
    pub minor_rev_level: u16,
    pub lastcheck: u32,
    pub checkinterval: u32,
    pub creator_os: u32,
    pub rev_level: u32,
    pub def_resuid: u16,
    pub def_resgid: u16,
    pub first_ino: u32,
    pub inode_size_raw: u16,
    pub block_group_nr: u16,
    pub feature_compat: u32,
    pub feature_incompat: u32,
    pub feature_ro_compat: u32,
    pub uuid: [u8; 16],
    pub volume_name: [u8; 16],
    pub last_mounted: [u8; 64],
    pub algorithm_usage_bitmap: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct RawBlockGroupDescriptor {
    pub block_bitmap: u32,
    pub inode_bitmap: u32,
    pub inode_table: u32,
    pub free_blocks_count: u16,
    pub free_inodes_count: u16,
    pub used_dirs_count: u16,
    pub pad: u16,
    pub reserved: [u32; 3],
}

#[derive(Clone, Copy)]
#[repr(C)]
pub(crate) struct RawInode {
    pub mode: u16,
    pub uid: u16,
    pub size_low: u32,
    pub atime: u32,
    pub ctime: u32,
    pub mtime: u32,
    pub dtime: u32,
    pub gid: u16,
    pub links_count: u16,
    pub blocks_count: u32,
    pub flags: u32,
    pub osd1: u32,
    pub block: [u32; 15],
    pub generation: u32,
    pub file_acl: u32,
    pub dir_acl: u32,
    pub faddr: u32,
    pub osd2: [u32; 3],
}

#[derive(Clone, Copy)]
#[repr(C)]
struct RawDirEntry {
    inode: u32,
    rec_len: u16,
    name_len: u8,
    file_type: u8,
}

#[derive(Clone, Copy)]
pub struct Ext2Fs {
    id: u64,
    dev_id: u8,
    block_size: u64,
    blocks_per_group: u32,
    inodes_per_group: u32,
    inodes_count: u32,
    blocks_count: u32,
    bgdt_start: u64,
    inode_size: u16,
    mounted: bool,
}

fn read_unaligned_sb(buf: &[u8]) -> RawSuperblock {
    unsafe {
        let ptr = buf.as_ptr() as *const u8;
        let mut sb: RawSuperblock = core::mem::zeroed();
        core::ptr::copy_nonoverlapping(ptr, &mut sb as *mut RawSuperblock as *mut u8, core::mem::size_of::<RawSuperblock>());
        sb
    }
}

fn read_unaligned_bgdt(buf: &[u8]) -> RawBlockGroupDescriptor {
    unsafe {
        let mut bgd: RawBlockGroupDescriptor = core::mem::zeroed();
        core::ptr::copy_nonoverlapping(buf.as_ptr(), &mut bgd as *mut RawBlockGroupDescriptor as *mut u8, core::mem::size_of::<RawBlockGroupDescriptor>());
        bgd
    }
}

fn read_unaligned_inode(buf: &[u8]) -> RawInode {
    unsafe {
        let mut inode: RawInode = core::mem::zeroed();
        core::ptr::copy_nonoverlapping(buf.as_ptr(), &mut inode as *mut RawInode as *mut u8, core::mem::size_of::<RawInode>());
        inode
    }
}

fn write_unaligned_bgdt(buf: &mut [u8], bgd: &RawBlockGroupDescriptor) {
    unsafe {
        core::ptr::copy_nonoverlapping(bgd as *const RawBlockGroupDescriptor as *const u8, buf.as_mut_ptr(), core::mem::size_of::<RawBlockGroupDescriptor>());
    }
}

fn write_unaligned_inode(buf: &mut [u8], inode: &RawInode) {
    unsafe {
        core::ptr::copy_nonoverlapping(inode as *const RawInode as *const u8, buf.as_mut_ptr(), core::mem::size_of::<RawInode>());
    }
}

impl Ext2Fs {
    pub fn mount(dev_id: u8) -> Option<&'static Self> {
        let mut sb_buf = [0u8; 2048];
        for i in 0..4u64 {
            let start = (i * 512) as usize;
            let end = ((i + 1) * 512) as usize;
            if !bc_read(dev_id, i, &mut sb_buf[start..end]) {
                return None;
            }
        }

        let raw_sb = read_unaligned_sb(&sb_buf[1024..1024 + core::mem::size_of::<RawSuperblock>()]);

        if raw_sb.magic != EXT2_MAGIC {
            return None;
        }

        let rev = raw_sb.rev_level;
        let inode_size = if rev >= EXT2_DYNAMIC_REV {
            if raw_sb.inode_size_raw < 128 || raw_sb.inode_size_raw > 256 {
                return None;
            }
            raw_sb.inode_size_raw
        } else {
            128
        };

        let log_block_size = raw_sb.log_block_size;
        if log_block_size > 6 {
            return None;
        }
        let block_size = (1024u64) << log_block_size;
        let bgdt_start = if block_size == 1024 { 2u64 } else { 1u64 };

        let id = NEXT_EXT2_ID.fetch_add(1, core::sync::atomic::Ordering::SeqCst);

        let fs = Ext2Fs {
            id,
            dev_id,
            block_size,
            blocks_per_group: raw_sb.blocks_per_group,
            inodes_per_group: raw_sb.inodes_per_group,
            inodes_count: raw_sb.inodes_count,
            blocks_count: raw_sb.blocks_count,
            bgdt_start,
            inode_size,
            mounted: true,
        };

        let mut pool = EXT2_POOL.lock();
        for slot in pool.iter_mut() {
            if slot.is_none() {
                *slot = Some(fs);
                let ptr = slot.as_ref().unwrap() as *const Ext2Fs;
                return Some(unsafe { &*ptr });
            }
        }
        None
    }

    pub fn unmount(dev_id: u8) {
        let mut pool = EXT2_POOL.lock();
        for slot in pool.iter_mut() {
            if let Some(fs) = slot {
                if fs.dev_id == dev_id {
                    *slot = None;
                    return;
                }
            }
        }
    }

    fn read_bgdt(&self, group: u32) -> Option<RawBlockGroupDescriptor> {
        let entry_size = core::mem::size_of::<RawBlockGroupDescriptor>() as u64;
        let offset = group as u64 * entry_size;
        let bgdt_start = self.bgdt_start;
        let block_size = self.block_size;
        let sector = (bgdt_start * block_size / 512) + (offset / 512);
        let offset_in_sector = offset % 512;

        if offset_in_sector + entry_size > 512 {
            return None;
        }

        let mut buf = [0u8; 512];
        if !bc_read(self.dev_id, sector, &mut buf) {
            return None;
        }

        Some(read_unaligned_bgdt(&buf[offset_in_sector as usize..]))
    }

    fn read_inode_raw(&self, inode: u64) -> Option<RawInode> {
        if inode == 0 || inode > self.inodes_count as u64 { return None; }
        let group = ((inode - 1) / self.inodes_per_group as u64) as u32;
        let local_idx = ((inode - 1) % self.inodes_per_group as u64) as u32;
        let bgd = self.read_bgdt(group)?;

        let inode_size = self.inode_size as u64;
        let inode_offset = local_idx as u64 * inode_size;
        let sector = (bgd.inode_table as u64 * self.block_size / 512) + (inode_offset / 512);
        let offset_in_sector = (inode_offset % 512) as usize;

        let raw_size = core::mem::size_of::<RawInode>();
        let needed_sectors = (offset_in_sector + raw_size + 511) / 512;
        let mut buf = [0u8; 2048];
        for i in 0..needed_sectors as u64 {
            let s = i as usize;
            if !bc_read(self.dev_id, sector + i, &mut buf[s * 512..(s + 1) * 512]) {
                return None;
            }
        }

        Some(read_unaligned_inode(&buf[offset_in_sector..]))
    }

    fn read_block_data(&self, block: u32, buf: &mut [u8]) -> bool {
        let sector = block as u64 * self.block_size / 512;
        let sectors = (self.block_size as usize + 511) / 512;
        for i in 0..sectors {
            let off = i * 512;
            if off >= buf.len() { break; }
            let end = core::cmp::min(off + 512, buf.len());
            if !bc_read(self.dev_id, sector + i as u64, &mut buf[off..end]) {
                return false;
            }
        }
        true
    }

    fn inode_read_block(&self, raw: &RawInode, block_idx: u32) -> Option<u32> {
        if (block_idx as usize) < 12 {
            if raw.block[block_idx as usize] == 0 {
                return None;
            }
            return Some(raw.block[block_idx as usize]);
        }

        let indirect_idx = block_idx - 12;
        let ptrs_per_block = (self.block_size / 4) as u32;

        if indirect_idx < ptrs_per_block && raw.block[12] != 0 {
            let bsz = self.block_size as usize;
            let mut blk_buf = alloc::vec![0u8; bsz];
            if !self.read_block_data(raw.block[12], &mut blk_buf) {
                return None;
            }
            let byte_off = (indirect_idx as usize) * 4;
            if byte_off + 4 > bsz { return None; }
            let entry = u32::from_le_bytes([
                blk_buf[byte_off], blk_buf[byte_off + 1],
                blk_buf[byte_off + 2], blk_buf[byte_off + 3],
            ]);
            if entry == 0 { return None; }
            return Some(entry);
        }

        let dbl_idx = indirect_idx.saturating_sub(ptrs_per_block);
        if dbl_idx < ptrs_per_block * ptrs_per_block && raw.block[13] != 0 {
            let bsz = self.block_size as usize;
            let mut blk_buf = alloc::vec![0u8; bsz];
            if !self.read_block_data(raw.block[13], &mut blk_buf) {
                return None;
            }
            let l1_idx = (dbl_idx / ptrs_per_block) as usize * 4;
            if l1_idx + 4 > bsz { return None; }
            let l1_block = u32::from_le_bytes([
                blk_buf[l1_idx], blk_buf[l1_idx + 1],
                blk_buf[l1_idx + 2], blk_buf[l1_idx + 3],
            ]);
            if l1_block == 0 { return None; }

            let mut blk_buf2 = alloc::vec![0u8; bsz];
            if !self.read_block_data(l1_block, &mut blk_buf2) {
                return None;
            }
            let l2_idx = (dbl_idx % ptrs_per_block) as usize * 4;
            if l2_idx + 4 > bsz { return None; }
            let entry = u32::from_le_bytes([
                blk_buf2[l2_idx], blk_buf2[l2_idx + 1],
                blk_buf2[l2_idx + 2], blk_buf2[l2_idx + 3],
            ]);
            if entry == 0 { return None; }
            return Some(entry);
        }

        None
    }

    fn inode_file_type(mode: u16) -> FileType {
        match mode & 0xF000 {
            0x8000 => FileType::File,
            0x4000 => FileType::Directory,
            _ => FileType::File,
        }
    }

    fn write_block_data(&self, block: u32, buf: &[u8]) -> bool {
        let sector = block as u64 * self.block_size / 512;
        let sectors = (self.block_size as usize + 511) / 512;
        for i in 0..sectors {
            let off = i * 512;
            if off >= buf.len() { break; }
            let end = core::cmp::min(off + 512, buf.len());
            if !crate::block_cache::bc_write(self.dev_id, sector + i as u64, &buf[off..end]) {
                return false;
            }
        }
        true
    }

    fn read_block_bitmap(&self, group: u32) -> Option<alloc::vec::Vec<u8>> {
        let bgd = self.read_bgdt(group)?;
        let sector = bgd.block_bitmap as u64 * self.block_size / 512;
        let sectors = (self.block_size as usize + 511) / 512;
        let bsz = self.block_size as usize;
        let mut buf = alloc::vec![0u8; bsz];
        for i in 0..sectors {
            let off = i * 512;
            if off >= bsz { break; }
            let end = core::cmp::min(off + 512, bsz);
            if !bc_read(self.dev_id, sector + i as u64, &mut buf[off..end]) {
                return None;
            }
        }
        Some(buf)
    }

    fn write_block_bitmap(&self, group: u32, bitmap: &[u8]) -> bool {
        let bgd = match self.read_bgdt(group) {
            Some(b) => b,
            None => return false,
        };
        let sector = bgd.block_bitmap as u64 * self.block_size / 512;
        let sectors = (self.block_size as usize + 511) / 512;
        for i in 0..sectors {
            let off = i * 512;
            if off >= bitmap.len() { break; }
            let end = core::cmp::min(off + 512, bitmap.len());
            if !crate::block_cache::bc_write(self.dev_id, sector + i as u64, &bitmap[off..end]) {
                return false;
            }
        }
        true
    }

    fn write_bgdt(&self, group: u32, bgd: &RawBlockGroupDescriptor) -> bool {
        let entry_size = core::mem::size_of::<RawBlockGroupDescriptor>() as u64;
        let offset = group as u64 * entry_size;
        let sector = (self.bgdt_start * self.block_size / 512) + (offset / 512);
        let offset_in_sector = offset % 512;

        let mut buf = [0u8; 512];
        if !crate::block_cache::bc_read(self.dev_id, sector, &mut buf) {
            return false;
        }
        write_unaligned_bgdt(&mut buf[offset_in_sector as usize..], bgd);
        crate::block_cache::bc_write(self.dev_id, sector, &buf)
    }

    fn alloc_block(&self) -> Option<u32> {
        let num_groups = (self.blocks_count as u64 + self.blocks_per_group as u64 - 1)
            / self.blocks_per_group as u64;

        for group in 0..num_groups as u32 {
            let mut bitmap = match self.read_block_bitmap(group) {
                Some(b) => b,
                None => continue,
            };
            let blocks_in_group =
                (self.block_size as usize * 8).min(self.blocks_per_group as usize);

            let start_idx = if group == 0 { 2 } else { 0 };
            for i in start_idx..blocks_in_group {
                let byte_idx = i / 8;
                let bit_idx = i % 8;
                if byte_idx >= bitmap.len() { break; }
                if (bitmap[byte_idx] & (1 << bit_idx)) == 0 {
                    bitmap[byte_idx] |= 1 << bit_idx;
                    self.write_block_bitmap(group, &bitmap);
                    if let Some(mut bgd) = self.read_bgdt(group) {
                        bgd.free_blocks_count =
                            bgd.free_blocks_count.saturating_sub(1);
                        self.write_bgdt(group, &bgd);
                    }
                    let phys = group * self.blocks_per_group + i as u32;
                    return Some(phys);
                }
            }
        }
        None
    }

    fn inode_set_block(&self, raw: &mut RawInode, block_idx: u32, phys: u32) -> bool {
        if (block_idx as usize) < 12 {
            raw.block[block_idx as usize] = phys;
            return true;
        }
        let bsz = self.block_size as usize;
        let ptrs_per_block = bsz as u32 / 4;
        if block_idx < 12 + ptrs_per_block {
            if raw.block[12] == 0 {
                let indirect = match self.alloc_block() {
                    Some(b) => b,
                    None => return false,
                };
                raw.block[12] = indirect;
                let zero = alloc::vec![0u8; bsz];
                if !self.write_block_data(indirect, &zero) {
                    return false;
                }
            }
            let entry_idx = block_idx - 12;
            let byte_off = entry_idx as usize * 4;
            let mut blk_buf = alloc::vec![0u8; bsz];
            if !self.read_block_data(raw.block[12], &mut blk_buf) {
                return false;
            }
            blk_buf[byte_off..byte_off + 4].copy_from_slice(&phys.to_le_bytes());
            return self.write_block_data(raw.block[12], &blk_buf);
        }
        false
    }

    fn write_inode_raw(&self, inode: u64, raw: &RawInode) -> bool {
        if inode == 0 || inode > self.inodes_count as u64 { return false; }
        let group = ((inode - 1) / self.inodes_per_group as u64) as u32;
        let local_idx = ((inode - 1) % self.inodes_per_group as u64) as u32;
        let bgd = match self.read_bgdt(group) {
            Some(b) => b,
            None => return false,
        };

        let inode_size = self.inode_size as u64;
        let inode_offset = local_idx as u64 * inode_size;
        let sector = (bgd.inode_table as u64 * self.block_size / 512) + (inode_offset / 512);
        let offset_in_sector = (inode_offset % 512) as usize;

        let raw_size = core::mem::size_of::<RawInode>();
        let inode_read_size = core::cmp::max(raw_size, self.inode_size as usize);
        let needed_sectors = (offset_in_sector + inode_read_size + 511) / 512;
        let mut buf = alloc::vec![0u8; needed_sectors * 512];

        for i in 0..needed_sectors as u64 {
            let s = i as usize;
            if !crate::block_cache::bc_read(
                self.dev_id, sector + i, &mut buf[s * 512..(s + 1) * 512],
            ) {
                return false;
            }
        }

        write_unaligned_inode(&mut buf[offset_in_sector..], raw);

        for i in 0..needed_sectors as u64 {
            let s = i as usize;
            if !crate::block_cache::bc_write(
                self.dev_id, sector + i, &buf[s * 512..(s + 1) * 512],
            ) {
                return false;
            }
        }
        true
    }
}

impl FileSystem for Ext2Fs {
    fn name(&self) -> &'static str { "ext2" }

    fn root_inode(&self) -> u64 { ROOT_INODE }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let raw = self.read_inode_raw(inode)?;
        let size = raw.size_low as u64;
        if offset >= size || buf.is_empty() {
            return Some(0);
        }

        let block_size = self.block_size;
        let start_block = (offset / block_size) as u32;
        let end = core::cmp::min(offset + buf.len() as u64, size);
        let end_block = ((end + block_size - 1) / block_size) as u32;
        let mut written = 0u64;

        for b in start_block..end_block {
            let phys = self.inode_read_block(&raw, b)?;
            let bsz = block_size as usize;
            let mut block_buf = alloc::vec![0u8; bsz];
            if !self.read_block_data(phys, &mut block_buf) {
                return None;
            }
            let block_start = b as u64 * block_size;
            let copy_start = if offset > block_start {
                (offset - block_start) as usize
            } else {
                0
            };
            let copy_end_unclamped = (end - block_start) as usize;
            let copy_end = core::cmp::min(copy_end_unclamped, bsz);
            let copy_len = copy_end.saturating_sub(copy_start);
            if copy_len == 0 { continue; }

            let dest_start = written as usize;
            let len = core::cmp::min(copy_len, buf.len() - dest_start);
            buf[dest_start..dest_start + len]
                .copy_from_slice(&block_buf[copy_start..copy_start + len]);
            written += len as u64;
        }
        Some(written)
    }

    fn write(&self, inode: u64, offset: u64, buf: &[u8]) -> Option<u64> {
        let mut raw = self.read_inode_raw(inode)?;
        let block_size = self.block_size as usize;
        let file_size = raw.size_low as u64;
        if offset > file_size {
            return None;
        }
        let mut written = 0u64;
        let len = buf.len();

        while written < len as u64 {
            let logical_block = ((offset + written) / self.block_size) as u32;
            let block_off = ((offset + written) % self.block_size) as usize;
            let to_copy =
                (block_size - block_off).min((len as u64 - written) as usize);

            let mut phys = self.inode_read_block(&raw, logical_block);
            if phys.is_none() && (offset + written) < file_size {
                return None;
            }
            if phys.is_none() {
                let nb = self.alloc_block()?;
                if !self.inode_set_block(&mut raw, logical_block, nb) {
                    return None;
                }
                phys = Some(nb);
            }
            let phys = phys?;

            let mut block_buf = alloc::vec![0u8; block_size];
            if (offset + written) < file_size && to_copy < block_size {
                if !self.read_block_data(phys, &mut block_buf) {
                    return None;
                }
            } else if to_copy < block_size {
                for b in block_buf.iter_mut() { *b = 0; }
            }
            block_buf[block_off..block_off + to_copy]
                .copy_from_slice(&buf[written as usize..written as usize + to_copy]);
            if !self.write_block_data(phys, &block_buf) {
                return None;
            }
            written += to_copy as u64;
        }

        let new_size = offset + written;
        if new_size > raw.size_low as u64 {
            if new_size > u32::MAX as u64 {
                return None;
            }
            raw.size_low = new_size as u32;
        }
        if !self.write_inode_raw(inode, &raw) {
            return None;
        }
        Some(written)
    }

    fn read_dir(&self, inode: u64) -> alloc::vec::Vec<DirEntry> {
        let mut entries = alloc::vec::Vec::with_capacity(32);
        let raw = match self.read_inode_raw(inode) {
            Some(r) => r,
            None => return entries,
        };
        if Self::inode_file_type(raw.mode) != FileType::Directory {
            return entries;
        }

        let size = raw.size_low as u64;
        let block_size = self.block_size as usize;
        let mut file_offset = 0u64;

        while file_offset < size {
            let block_idx = (file_offset / self.block_size) as u32;
            let block_start = block_idx as u64 * self.block_size;
            let phys = match self.inode_read_block(&raw, block_idx) {
                Some(p) => p,
                None => break,
            };
            let mut block_buf = alloc::vec![0u8; block_size];
            if !self.read_block_data(phys, &mut block_buf) {
                break;
            }

            let mut pos = (file_offset - block_start) as usize;
            while pos + core::mem::size_of::<RawDirEntry>() <= block_size {
                let de_ptr = &block_buf[pos] as *const u8 as *const RawDirEntry;
                let de: RawDirEntry = unsafe { core::ptr::read_unaligned(de_ptr) };
                if de.rec_len == 0 { break; }
                if de.inode != 0 {
                    let name_len = de.name_len as usize;
                    if name_len > 0 && name_len <= 255 {
                        let name_start = pos + core::mem::size_of::<RawDirEntry>();
                        if name_start + name_len <= block_size {
                            let name_bytes = &block_buf[name_start..name_start + name_len];
                            if let Ok(name) = core::str::from_utf8(name_bytes) {
                                if name != "." && name != ".." {
                                    let ft = if de.file_type == EXT2_FT_DIR {
                                        FileType::Directory
                                    } else {
                                        FileType::File
                                    };
                                    if entries.len() < 64 {
                                        entries.push(DirEntry {
                                            name: alloc::string::String::from(name),
                                            file_type: ft,
                                            inode: de.inode as u64,
                                        });
                                    }
                                }
                            }
                        }
                    }
                }
                pos += de.rec_len as usize;
            }
            file_offset = (block_idx as u64 + 1) * self.block_size;
        }
        entries
    }

    fn stat(&self, inode: u64) -> FileStat {
        match self.read_inode_raw(inode) {
            Some(raw) => {
                let size = raw.size_low as u64;
                FileStat {
                    size,
                    file_type: Self::inode_file_type(raw.mode),
                    inode,
                    blocks: (size + 511) / 512,
                    uid: raw.uid as u32,
                    gid: raw.gid as u32,
                    mode: raw.mode,
                }
            }
            None => FileStat {
                size: 0,
                file_type: FileType::None,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0,
            },
        }
    }

    fn create(&self, _parent_inode: u64, _name: &str, _file_type: FileType) -> Option<u64> {
        None
    }

    fn unlink(&self, _parent_inode: u64, _name: &str) -> bool {
        false
    }

    fn chmod(&self, inode: u64, mode: u16) -> bool {
        let mut raw = match self.read_inode_raw(inode) {
            Some(r) => r,
            None => return false,
        };
        raw.mode = (raw.mode & 0xF000) | (mode & 0x0FFF);
        self.write_inode_raw(inode, &raw)
    }

    fn chown(&self, inode: u64, uid: u32, gid: u32) -> bool {
        if uid > u16::MAX as u32 || gid > u16::MAX as u32 {
            return false;
        }
        let mut raw = match self.read_inode_raw(inode) {
            Some(r) => r,
            None => return false,
        };
        raw.uid = uid as u16;
        raw.gid = gid as u16;
        self.write_inode_raw(inode, &raw)
    }
}

#[cfg(feature = "testing")]
pub mod tests {
    use super::*;

    pub fn test_magic_constant() -> Result<(), &'static str> {
        if EXT2_MAGIC != 0xEF53 {
            return Err("EXT2_MAGIC should be 0xEF53");
        }
        Ok(())
    }

    pub fn test_root_inode_constant() -> Result<(), &'static str> {
        if ROOT_INODE != 2 {
            return Err("ROOT_INODE should be 2");
        }
        Ok(())
    }

    pub fn test_raw_superblock_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawSuperblock>();
        if s != 204 {
            return Err("RawSuperblock should be exactly 204 bytes");
        }
        Ok(())
    }

    pub fn test_raw_inode_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawInode>();
        if s < 100 || s > 160 {
            return Err("RawInode size out of expected range (100-160)");
        }
        Ok(())
    }

    pub fn test_raw_dir_entry_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawDirEntry>();
        if s != 8 {
            return Err("RawDirEntry should be exactly 8 bytes");
        }
        Ok(())
    }

    pub fn test_raw_bgdt_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawBlockGroupDescriptor>();
        if s < 24 || s > 40 {
            return Err("RawBlockGroupDescriptor size out of range");
        }
        Ok(())
    }

    pub fn test_inode_file_type() -> Result<(), &'static str> {
        if Ext2Fs::inode_file_type(0x4000) != FileType::Directory {
            return Err("0x4000 should be Directory");
        }
        if Ext2Fs::inode_file_type(0x8000) != FileType::File {
            return Err("0x8000 should be File");
        }
        if Ext2Fs::inode_file_type(0xA000) != FileType::File {
            return Err("0xA000 (socket) should fallback to File");
        }
        Ok(())
    }
}
