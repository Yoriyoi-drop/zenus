use crate::vfs::{FileSystem, FileType, FileStat, DirEntry};
use crate::block_cache::bc_read;

pub(crate) const EXT2_MAGIC: u16 = 0xEF53;
const ROOT_INODE: u64 = 2;
const EXT2_S_IFDIR: u16 = 0x4000;
const EXT2_S_IFREG: u16 = 0x8000;

const EXT2_GOOD_OLD_REV: u32 = 0;
const EXT2_DYNAMIC_REV: u32 = 1;

const EXT2_FT_REG_FILE: u8 = 1;
const EXT2_FT_DIR: u8 = 2;

static mut EXT2_FS: Ext2Fs = Ext2Fs {
    dev_id: 0,
    block_size: 1024,
    blocks_per_group: 0,
    inodes_per_group: 0,
    inodes_count: 0,
    blocks_count: 0,
    bgdt_start: 0,
    inode_size: 128,
    mounted: false,
};

#[derive(Clone, Copy)]
#[repr(C, packed)]
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
#[repr(C, packed)]
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
#[repr(C, packed)]
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
#[repr(C, packed)]
struct RawDirEntry {
    inode: u32,
    rec_len: u16,
    name_len: u8,
    file_type: u8,
}

#[derive(Clone, Copy)]
pub struct Ext2Fs {
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

impl Ext2Fs {
    pub fn mount(dev_id: u8) -> Option<&'static Self> {
        let mut sb_buf = [0u8; 2048];
        for i in 0..4 {
            let start = (i * 512) as usize;
            let end = ((i + 1) * 512) as usize;
            if !bc_read(dev_id, i, &mut sb_buf[start..end]) {
                return None;
            }
        }

        let raw_sb = unsafe { &*(sb_buf.as_ptr().add(1024) as *const RawSuperblock) };

        if raw_sb.magic != EXT2_MAGIC {
            return None;
        }

        let rev = raw_sb.rev_level;
        let inode_size = if rev >= EXT2_DYNAMIC_REV {
            raw_sb.inode_size_raw
        } else {
            128
        };

        let log_block_size = raw_sb.log_block_size;
        let block_size = (1024u64) << log_block_size;
        let blocks_per_group = raw_sb.blocks_per_group;
        let inodes_per_group = raw_sb.inodes_per_group;
        let inodes_count = raw_sb.inodes_count;
        let blocks_count = raw_sb.blocks_count;

        let bgdt_start = if block_size == 1024 { 2u64 } else { 1u64 };

        unsafe {
            EXT2_FS = Ext2Fs {
                dev_id,
                block_size,
                blocks_per_group,
                inodes_per_group,
                inodes_count,
                blocks_count,
                bgdt_start,
                inode_size,
                mounted: true,
            };
            Some(&EXT2_FS)
        }
    }

    fn read_bgdt(&self, group: u32) -> Option<RawBlockGroupDescriptor> {
        let entry_size = core::mem::size_of::<RawBlockGroupDescriptor>() as u64;
        let offset = group as u64 * entry_size;
        let sector = (self.bgdt_start * self.block_size / 512) + (offset / 512);
        let offset_in_sector = offset % 512;

        let mut buf = [0u8; 512];
        if !bc_read(self.dev_id, sector, &mut buf) {
            return None;
        }

        let ptr = unsafe { buf.as_ptr().add(offset_in_sector as usize) as *const RawBlockGroupDescriptor };
        Some(unsafe { *ptr })
    }

    fn read_inode_raw(&self, inode: u64) -> Option<RawInode> {
        if inode == 0 || inode > self.inodes_count as u64 { return None; }
        let group = ((inode - 1) / self.inodes_per_group as u64) as u32;
        let local_idx = ((inode - 1) % self.inodes_per_group as u64) as u32;
        let bgd = self.read_bgdt(group)?;
        let inode_table_block = bgd.inode_table as u64;

        let inode_offset = local_idx as u64 * self.inode_size as u64;
        let sector = (inode_table_block * self.block_size / 512) + (inode_offset / 512);
        let offset_in_sector = (inode_offset % 512) as usize;

        let mut buf = [0u8; 1024];
        let needed_sectors = (offset_in_sector + self.inode_size as usize + 511) / 512;
        for i in 0..needed_sectors as u64 {
            if !bc_read(self.dev_id, sector + i, &mut buf[i as usize * 512..(i as usize + 1) * 512]) {
                return None;
            }
        }

        let ptr = unsafe { buf.as_ptr().add(offset_in_sector) as *const RawInode };
        Some(unsafe { *ptr })
    }

    fn read_block_data(&self, block: u32, buf: &mut [u8]) -> bool {
        let sector = block as u64 * self.block_size / 512;
        let sectors = (self.block_size as usize + 511) / 512;
        for i in 0..sectors {
            let off = i * 512;
            if off >= buf.len() {
                break;
            }
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

        if block_idx == 12 && raw.block[12] != 0 {
            let mut buf = [0u8; 512];
            let sector = raw.block[12] as u64 * self.block_size / 512;
            if !bc_read(self.dev_id, sector, &mut buf) {
                return None;
            }
            return Some(unsafe { *(buf.as_ptr() as *const u32) });
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

    fn read_block_bitmap(&self, group: u32) -> Option<[u8; 4096]> {
        let bgd = self.read_bgdt(group)?;
        let bitmap_block = bgd.block_bitmap as u64;
        let mut buf = [0u8; 4096];
        let sector = bitmap_block * self.block_size / 512;
        let sectors = (self.block_size as usize + 511) / 512;
        for i in 0..sectors {
            let off = i * 512;
            if off >= buf.len() { break; }
            let end = core::cmp::min(off + 512, buf.len());
            if !crate::block_cache::bc_read(self.dev_id, sector + i as u64, &mut buf[off..end]) {
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
        let bitmap_block = bgd.block_bitmap as u64;
        let sector = bitmap_block * self.block_size / 512;
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
        let ptr = unsafe { buf.as_mut_ptr().add(offset_in_sector as usize) as *mut RawBlockGroupDescriptor };
        unsafe { *ptr = *bgd; }
        crate::block_cache::bc_write(self.dev_id, sector, &buf)
    }

    fn alloc_block(&self) -> Option<u32> {
        let mut bitmap = self.read_block_bitmap(0)?;
        let blocks_in_group = (self.block_size as usize * 8).min(self.blocks_per_group as usize);
        for i in 2..blocks_in_group {
            let byte_idx = i / 8;
            let bit_idx = i % 8;
            if byte_idx >= bitmap.len() { break; }
            if (bitmap[byte_idx] & (1 << bit_idx)) == 0 {
                bitmap[byte_idx] |= 1 << bit_idx;
                self.write_block_bitmap(0, &bitmap);
                let mut bgd = self.read_bgdt(0)?;
                bgd.free_blocks_count -= 1;
                self.write_bgdt(0, &bgd);
                crate::block_cache::bc_flush();
                return Some(i as u32);
            }
        }
        None
    }

    fn inode_set_block(&self, raw: &mut RawInode, block_idx: u32, phys: u32) -> bool {
        if (block_idx as usize) < 12 {
            raw.block[block_idx as usize] = phys;
            return true;
        }
        if block_idx < 12 + (self.block_size as u32 / 4) {
            if raw.block[12] == 0 {
                let indirect = match self.alloc_block() {
                    Some(b) => b,
                    None => return false,
                };
                raw.block[12] = indirect;
                let zero = [0u8; 4096];
                if !self.write_block_data(indirect, &zero[..self.block_size as usize]) {
                    return false;
                }
            }
            let indirect_block = raw.block[12];
            let entry_idx = block_idx - 12;
            let byte_off = entry_idx as u64 * 4;
            let sector = (indirect_block as u64 * self.block_size / 512) + (byte_off / 512);
            let off_in_sector = (byte_off % 512) as usize;
            let mut buf = [0u8; 512];
            if !crate::block_cache::bc_read(self.dev_id, sector, &mut buf) {
                return false;
            }
            let ptr = unsafe { buf.as_mut_ptr().add(off_in_sector) as *mut u32 };
            unsafe { *ptr = phys; }
            return crate::block_cache::bc_write(self.dev_id, sector, &buf);
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
        let inode_table_block = bgd.inode_table as u64;
        let inode_offset = local_idx as u64 * self.inode_size as u64;
        let sector = (inode_table_block * self.block_size / 512) + (inode_offset / 512);
        let offset_in_sector = (inode_offset % 512) as usize;

        let raw_size = core::mem::size_of::<RawInode>();
        let mut buf = [0u8; 1024];
        let needed_sectors = (offset_in_sector + raw_size + 511) / 512;
        for i in 0..needed_sectors as u64 {
            if !crate::block_cache::bc_read(self.dev_id, sector + i, &mut buf[i as usize * 512..(i as usize + 1) * 512]) {
                return false;
            }
        }

        let ptr = unsafe { buf.as_mut_ptr().add(offset_in_sector) as *mut RawInode };
        unsafe { *ptr = *raw; }

        for i in 0..needed_sectors as u64 {
            if !crate::block_cache::bc_write(self.dev_id, sector + i, &buf[i as usize * 512..(i as usize + 1) * 512]) {
                return false;
            }
        }
        crate::block_cache::bc_flush();
        true
    }
}

impl FileSystem for Ext2Fs {
    fn name(&self) -> &'static str {
        "ext2"
    }

    fn root_inode(&self) -> u64 {
        ROOT_INODE
    }

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
            let mut block_buf = [0u8; 4096];
            if !self.read_block_data(phys, &mut block_buf[..block_size as usize]) {
                return None;
            }

            let block_start = b as u64 * block_size;
            let copy_start = if offset > block_start { (offset - block_start) as usize } else { 0 };
            let copy_end_unclamped = (end - block_start) as usize;
            let copy_end = core::cmp::min(copy_end_unclamped, block_size as usize);
            let copy_len = copy_end.saturating_sub(copy_start);
            if copy_len == 0 {
                continue;
            }

            let dest_start = written as usize;
            let len = core::cmp::min(copy_len, buf.len() - dest_start);
            buf[dest_start..dest_start + len].copy_from_slice(&block_buf[copy_start..copy_start + len]);
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
            let to_copy = (block_size - block_off).min((len as u64 - written) as usize);

            let mut phys = self.inode_read_block(&raw, logical_block);
            if phys.is_none() && (offset + written) < file_size {
                return None;
            }
            if phys.is_none() {
                let nb = self.alloc_block()?;
                self.inode_set_block(&mut raw, logical_block, nb);
                phys = Some(nb);
            }
            let phys = phys?;

            let mut block_buf = [0u8; 4096];
            if (offset + written) < file_size && to_copy < block_size {
                if !self.read_block_data(phys, &mut block_buf[..block_size]) {
                    return None;
                }
            } else if to_copy < block_size {
                for b in block_buf.iter_mut() { *b = 0; }
            }
            block_buf[block_off..block_off + to_copy].copy_from_slice(
                &buf[written as usize..written as usize + to_copy],
            );
            if !self.write_block_data(phys, &block_buf[..block_size]) {
                return None;
            }
            written += to_copy as u64;
        }
        let new_size = offset + written;
        if new_size > raw.size_low as u64 {
            raw.size_low = new_size as u32;
        }
        self.write_inode_raw(inode, &raw);
        crate::block_cache::bc_flush();
        Some(written)
    }

    fn read_dir(&self, inode: u64) -> &'static [DirEntry] {
        static mut ENTRIES: [DirEntry; 64] = [DirEntry {
            name: "", file_type: FileType::None, inode: 0,
        }; 64];
        static mut COUNT: usize = 0;
        static mut NAME_BUF: [u8; 4096] = [0; 4096];
        static mut NAME_OFF: usize = 0;

        let raw = match self.read_inode_raw(inode) {
            Some(r) => r,
            None => return &[],
        };

        if Self::inode_file_type(raw.mode) != FileType::Directory {
            return &[];
        }

        let size = raw.size_low as u64;
        let block_size = self.block_size as usize;
        let mut block_buf = [0u8; 4096];
        let mut count = 0usize;

        unsafe { COUNT = 0; NAME_OFF = 0; }

        let mut file_offset = 0u64;
        while file_offset < size {
            let block_idx = (file_offset / self.block_size) as u32;
            let block_start = block_idx as u64 * self.block_size;

            let phys = match self.inode_read_block(&raw, block_idx) {
                Some(p) => p,
                None => break,
            };

            if !self.read_block_data(phys, &mut block_buf[..block_size]) {
                break;
            }

            let mut pos = (file_offset - block_start) as usize;
            while pos + core::mem::size_of::<RawDirEntry>() <= block_size {
                let de = unsafe { &*(block_buf.as_ptr().add(pos) as *const RawDirEntry) };
                if de.rec_len == 0 {
                    break;
                }
                if de.inode != 0 {
                    let name_len = de.name_len as usize;
                    if name_len > 0 && name_len <= 255 {
                        let name_start = pos + core::mem::size_of::<RawDirEntry>();
                        if name_start + name_len <= block_size {
                            unsafe {
                                if NAME_OFF + name_len + 1 > NAME_BUF.len() {
                                    break;
                                }
                                NAME_BUF[NAME_OFF..NAME_OFF + name_len].copy_from_slice(
                                    &block_buf[name_start..name_start + name_len],
                                );
                                let name = core::str::from_utf8_unchecked(
                                    &NAME_BUF[NAME_OFF..NAME_OFF + name_len],
                                );
                                NAME_OFF += name_len + 1;

                                if name != "." && name != ".." && count < 64 {
                                    let ft = match de.file_type {
                                        EXT2_FT_DIR => FileType::Directory,
                                        _ => FileType::File,
                                    };
                                    ENTRIES[count] = DirEntry {
                                        name,
                                        file_type: ft,
                                        inode: de.inode as u64,
                                    };
                                    count += 1;
    }
}   // end impl FileSystem for Ext2Fs
                        }
                    }
                }
                pos += de.rec_len as usize;
            }

            file_offset = ((file_offset / self.block_size) + 1) * self.block_size;
        }

        unsafe { COUNT = count; }
        unsafe { &ENTRIES[..COUNT] }
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
        // 22 u32 = 88, 10 u16 = 20, uuid[16] + volume_name[16] + last_mounted[64] = 96
        // Total with packed repr: 88 + 20 + 96 = 204
        if s != 204 {
            return Err("RawSuperblock should be exactly 204 bytes");
        }
        Ok(())
    }

    pub fn test_raw_inode_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawInode>();
        // Standard ext2 inode is 128 bytes
        if s < 100 || s > 160 {
            return Err("RawInode size out of expected range (100-160)");
        }
        Ok(())
    }

    pub fn test_raw_dir_entry_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawDirEntry>();
        // DirEntry is 8 bytes: inode(4) + rec_len(2) + name_len(1) + file_type(1)
        if s != 8 {
            return Err("RawDirEntry should be exactly 8 bytes");
        }
        Ok(())
    }

    pub fn test_raw_bgdt_size() -> Result<(), &'static str> {
        let s = core::mem::size_of::<RawBlockGroupDescriptor>();
        // Standard BGDT entry is 32 bytes
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
