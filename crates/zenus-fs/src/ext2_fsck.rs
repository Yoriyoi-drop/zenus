use crate::block_cache::bc_read;
use crate::ext2::*;

const EXT2_VALID_FS: u16 = 1;
const EXT2_ERROR_FS: u16 = 2;
const EXT2_GOOD_OLD_REV: u32 = 0;
const EXT2_DYNAMIC_REV: u32 = 1;

const EXT2_FEATURE_INCOMPAT_FILETYPE: u32 = 0x0002;
const EXT2_FEATURE_INCOMPAT_EXTENTS: u32 = 0x0040;

const EXT2_FEATURE_RO_COMPAT_SPARSE_SUPER: u32 = 0x0001;

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum FsckSeverity {
    Info = 0,
    Warning = 1,
    Error = 2,
    Fatal = 3,
}

#[derive(Clone, Copy)]
pub struct FsckMessage {
    pub severity: FsckSeverity,
    pub msg: &'static str,
    pub code: u16,
}

pub struct FsckReport {
    pub messages: [FsckMessage; 48],
    pub count: usize,
    pub errors: u32,
    pub warnings: u32,
}

impl FsckReport {
    pub fn passed(&self) -> bool {
        self.errors == 0
    }
}

fn add_msg(report: &mut FsckReport, sev: FsckSeverity, code: u16, msg: &'static str) {
    if report.count >= 48 {
        return;
    }
    match sev {
        FsckSeverity::Error | FsckSeverity::Fatal => report.errors += 1,
        FsckSeverity::Warning => report.warnings += 1,
        _ => {}
    }
    report.messages[report.count] = FsckMessage {
        severity: sev,
        msg,
        code,
    };
    report.count += 1;
}

fn read_raw_sb(dev_id: u8) -> Option<RawSuperblock> {
    let mut sb_buf = [0u8; 2048];
    for i in 0..4 {
        let start = (i * 512) as usize;
        let end = ((i + 1) * 512) as usize;
        if !bc_read(dev_id, i, &mut sb_buf[start..end]) {
            return None;
        }
    }
    Some(unsafe { *(sb_buf.as_ptr().add(1024) as *const RawSuperblock) })
}

pub fn fsck(dev_id: u8) -> FsckReport {
    let mut report = FsckReport {
        messages: [FsckMessage {
            severity: FsckSeverity::Info,
            msg: "",
            code: 0,
        }; 48],
        count: 0,
        errors: 0,
        warnings: 0,
    };

    let raw_sb = match read_raw_sb(dev_id) {
        Some(sb) => sb,
        None => {
            add_msg(&mut report, FsckSeverity::Fatal, 1, "Cannot read superblock");
            return report;
        }
    };

    if raw_sb.magic != EXT2_MAGIC {
        add_msg(&mut report, FsckSeverity::Fatal, 2, "Bad magic: not ext2");
        return report;
    }
    add_msg(&mut report, FsckSeverity::Info, 3, "Superblock magic OK");

    let state = raw_sb.state;
    if state == EXT2_VALID_FS {
        add_msg(&mut report, FsckSeverity::Info, 4, "Filesystem was cleanly unmounted");
    } else if state == EXT2_ERROR_FS {
        add_msg(&mut report, FsckSeverity::Error, 5, "Filesystem has errors (not cleanly unmounted)");
    } else {
        add_msg(&mut report, FsckSeverity::Warning, 6, "Unknown filesystem state");
    }

    let rev = raw_sb.rev_level;
    if rev != EXT2_GOOD_OLD_REV && rev != EXT2_DYNAMIC_REV {
        add_msg(&mut report, FsckSeverity::Error, 7, "Unknown revision level");
    }

    let log_block_size = raw_sb.log_block_size;
    let block_size = (1024u64) << log_block_size;
    if log_block_size > 2 {
        add_msg(&mut report, FsckSeverity::Error, 8, "Unsupported block size (max 4096)");
    }

    let blocks_per_group = raw_sb.blocks_per_group;
    if blocks_per_group == 0 || blocks_per_group % 8 != 0 {
        add_msg(&mut report, FsckSeverity::Error, 9, "Invalid blocks_per_group");
    }

    let inodes_per_group = raw_sb.inodes_per_group;
    if inodes_per_group == 0 {
        add_msg(&mut report, FsckSeverity::Error, 10, "inodes_per_group is zero");
    }

    let inodes_count = raw_sb.inodes_count;
    let blocks_count = raw_sb.blocks_count;

    let num_groups = (inodes_count + inodes_per_group - 1) / inodes_per_group;
    let blocks_groups = (blocks_count + blocks_per_group - 1) / blocks_per_group;
    if num_groups != blocks_groups {
        add_msg(&mut report, FsckSeverity::Warning, 11, "Group count mismatch (inodes vs blocks)");
    }
    let num_groups = core::cmp::max(num_groups, blocks_groups);
    if num_groups == 0 {
        add_msg(&mut report, FsckSeverity::Error, 12, "Zero groups");
        return report;
    }

    if raw_sb.free_blocks_count > raw_sb.blocks_count {
        add_msg(&mut report, FsckSeverity::Error, 13, "free_blocks > blocks_count");
    }
    if raw_sb.free_inodes_count > raw_sb.inodes_count {
        add_msg(&mut report, FsckSeverity::Error, 14, "free_inodes > inodes_count");
    }

    let inode_size = if rev >= EXT2_DYNAMIC_REV {
        raw_sb.inode_size_raw
    } else {
        128
    };
    if inode_size < 128 {
        add_msg(&mut report, FsckSeverity::Error, 15, "inode_size < 128");
    }

    let last_group_blocks = blocks_count as u64 - (num_groups - 1) as u64 * blocks_per_group as u64;
    let last_group_inodes = inodes_count - (num_groups - 1) * inodes_per_group;

    add_msg(
        &mut report,
        FsckSeverity::Info,
        16,
        "Filesystem geometry OK",
    );

    let feature_incompat = raw_sb.feature_incompat;
    let supported = EXT2_FEATURE_INCOMPAT_FILETYPE;
    let unsupported = feature_incompat & !supported;
    if unsupported != 0 {
        if unsupported & EXT2_FEATURE_INCOMPAT_EXTENTS != 0 {
            add_msg(&mut report, FsckSeverity::Warning, 18, "EXTENTS feature (ext4) not supported");
        } else {
            add_msg(&mut report, FsckSeverity::Error, 17, "Unsupported feature_incompat flags");
        }
    }

    let feature_ro_compat = raw_sb.feature_ro_compat;
    let supported_ro = EXT2_FEATURE_RO_COMPAT_SPARSE_SUPER;
    let unsupported_ro = feature_ro_compat & !supported_ro;
    if unsupported_ro != 0 {
        add_msg(&mut report, FsckSeverity::Warning, 19, "Unsupported read-only compat features");
    }

    let bgdt_start = if block_size == 1024 { 2u64 } else { 1u64 };

    for g in 0..num_groups {
        let bgd = match read_bgd(dev_id, bgdt_start, block_size, g) {
            Some(b) => b,
            None => {
                add_msg(&mut report, FsckSeverity::Error, 20, "Cannot read BGDT entry");
                continue;
            }
        };

        if bgd.block_bitmap == 0 {
            add_msg(&mut report, FsckSeverity::Error, 21, "block_bitmap is zero");
        }
        if bgd.inode_bitmap == 0 {
            add_msg(&mut report, FsckSeverity::Error, 22, "inode_bitmap is zero");
        }
        if bgd.inode_table == 0 {
            add_msg(&mut report, FsckSeverity::Error, 23, "inode_table is zero");
        }

        let is_last = g == num_groups - 1;
        let this_group_blocks = if is_last {
            last_group_blocks
        } else {
            blocks_per_group as u64
        };
        let this_group_inodes = if is_last {
            last_group_inodes
        } else {
            inodes_per_group
        };

        if bgd.free_blocks_count as u64 > this_group_blocks {
            add_msg(&mut report, FsckSeverity::Warning, 24, "free_blocks > group block count");
        }
        if bgd.free_inodes_count as u64 > this_group_inodes as u64 {
            add_msg(&mut report, FsckSeverity::Warning, 25, "free_inodes > group inode count");
        }

        if bgd.block_bitmap as u64 >= blocks_count as u64 {
            add_msg(&mut report, FsckSeverity::Error, 26, "block_bitmap beyond device");
        }
        if bgd.inode_bitmap as u64 >= blocks_count as u64 {
            add_msg(&mut report, FsckSeverity::Error, 27, "inode_bitmap beyond device");
        }
        if bgd.inode_table as u64 >= blocks_count as u64 {
            add_msg(&mut report, FsckSeverity::Error, 28, "inode_table beyond device");
        }

        let inode_table_blocks = (this_group_inodes as u64 * inode_size as u64 + block_size - 1) / block_size;
        if bgd.inode_table as u64 + inode_table_blocks > blocks_count as u64 {
            add_msg(&mut report, FsckSeverity::Error, 29, "inode_table spans beyond device");
        }

        let mut buf = [0u8; 4096];
        let sectors = (block_size as usize + 511) / 512;
        let bitmap_sector = bgd.block_bitmap as u64 * block_size / 512;
        let mut ok = true;
        if block_size as usize <= buf.len() {
            for i in 0..sectors {
                let off = i * 512;
                if off + 512 > buf.len() { break; }
                if !bc_read(dev_id, bitmap_sector + i as u64, &mut buf[off..off + 512]) {
                    ok = false;
                    break;
                }
            }
        } else {
            ok = false;
        }
        if ok {
            let mut set_bits = 0u64;
            let max_bits = core::cmp::min(this_group_blocks, block_size * 8);
            for i in 0..max_bits {
                if buf[(i / 8) as usize] & (1 << (i % 8)) != 0 {
                    set_bits += 1;
                }
            }
            let expected_used = this_group_blocks - bgd.free_blocks_count as u64;
            if set_bits != expected_used {
                add_msg(
                    &mut report,
                    FsckSeverity::Warning,
                    30,
                    "Block bitmap count mismatch",
                );
            }
        }
    }

    let root_raw = match read_inode(dev_id, inode_size, block_size, inodes_per_group, 2) {
        Some(r) => r,
        None => {
            add_msg(&mut report, FsckSeverity::Error, 31, "Cannot read root inode");
            return report;
        }
    };

    if root_raw.mode & 0xF000 != 0x4000 {
        add_msg(&mut report, FsckSeverity::Error, 32, "Root inode is not a directory");
    } else {
        add_msg(&mut report, FsckSeverity::Info, 33, "Root inode is a directory");
    }

    if root_raw.links_count < 2 {
        add_msg(&mut report, FsckSeverity::Warning, 34, "Root inode links_count < 2");
    }

    add_msg(
        &mut report,
        FsckSeverity::Info,
        35,
        "fsck complete",
    );

    report
}

fn read_bgd(dev_id: u8, bgdt_start: u64, block_size: u64, group: u32) -> Option<RawBlockGroupDescriptor> {
    let entry_size = core::mem::size_of::<RawBlockGroupDescriptor>() as u64;
    let offset = group as u64 * entry_size;
    let sector = (bgdt_start * block_size / 512) + (offset / 512);
    let offset_in_sector = offset % 512;

    let mut buf = [0u8; 512];
    if !bc_read(dev_id, sector, &mut buf) {
        return None;
    }
    let ptr = unsafe { buf.as_ptr().add(offset_in_sector as usize) as *const RawBlockGroupDescriptor };
    Some(unsafe { *ptr })
}

fn read_inode(
    dev_id: u8,
    inode_size: u16,
    block_size: u64,
    inodes_per_group: u32,
    inode_no: u64,
) -> Option<RawInode> {
    let group = ((inode_no - 1) / inodes_per_group as u64) as u32;
    let local_idx = ((inode_no - 1) % inodes_per_group as u64) as u32;

    let bgdt_start = if block_size == 1024 { 2u64 } else { 1u64 };
    let bgd = read_bgd(dev_id, bgdt_start, block_size, group)?;
    let inode_table_block = bgd.inode_table as u64;

    let inode_offset = local_idx as u64 * inode_size as u64;
    let sector = (inode_table_block * block_size / 512) + (inode_offset / 512);
    let offset_in_sector = (inode_offset % 512) as usize;

    let mut buf = [0u8; 1024];
    let needed_sectors = (offset_in_sector + inode_size as usize + 511) / 512;
    for i in 0..needed_sectors {
        let base = i * 512;
        if !bc_read(dev_id, sector + i as u64, &mut buf[base..base + 512]) {
            return None;
        }
    }

    let ptr = unsafe { buf.as_ptr().add(offset_in_sector) as *const RawInode };
    Some(unsafe { *ptr })
}
