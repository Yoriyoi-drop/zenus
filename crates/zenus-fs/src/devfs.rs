use crate::vfs::{FileSystem, FileType, FileStat, DirEntry};

const MAX_BLOCK_DEVS: usize = 8;

#[derive(Clone, Copy)]
pub struct BlockDeviceOps {
    pub read: fn(u64, &mut [u8]) -> bool,
    pub write: fn(u64, &[u8]) -> bool,
    pub size: u64,
}

static STATIC_ENTRIES: &[DirEntry] = &[
    DirEntry { name: "null", file_type: FileType::CharDevice, inode: 1 },
    DirEntry { name: "zero", file_type: FileType::CharDevice, inode: 2 },
    DirEntry { name: "console", file_type: FileType::CharDevice, inode: 3 },
    DirEntry { name: "serial", file_type: FileType::CharDevice, inode: 4 },
];

const BLOCK_INODE_BASE: u64 = 64;
static mut BLOCK_DEVS: [Option<(&'static str, BlockDeviceOps)>; MAX_BLOCK_DEVS] = [None; MAX_BLOCK_DEVS];
static mut BLOCK_DEV_COUNT: usize = 0;

pub fn block_device_read(dev_idx: usize, lba: u64, buf: &mut [u8]) -> bool {
    unsafe {
        BLOCK_DEVS.get(dev_idx).and_then(|o| o.as_ref()).map(|(_, ops)| {
            (ops.read)(lba, buf)
        }).unwrap_or(false)
    }
}

pub fn block_device_write(dev_idx: usize, lba: u64, buf: &[u8]) -> bool {
    unsafe {
        BLOCK_DEVS.get(dev_idx).and_then(|o| o.as_ref()).map(|(_, ops)| {
            (ops.write)(lba, buf)
        }).unwrap_or(false)
    }
}

pub fn register_block_device(name: &'static str, ops: BlockDeviceOps) -> bool {
    unsafe {
        if BLOCK_DEV_COUNT >= MAX_BLOCK_DEVS {
            return false;
        }
        BLOCK_DEVS[BLOCK_DEV_COUNT] = Some((name, ops));
        BLOCK_DEV_COUNT += 1;
    }
    true
}

pub fn block_device_count() -> usize {
    unsafe { BLOCK_DEV_COUNT }
}

pub struct DevFs;

impl DevFs {
    fn block_entry_at(&self, idx: usize) -> Option<DirEntry> {
        unsafe {
            BLOCK_DEVS.get(idx).and_then(|o| o.as_ref()).map(|(name, _)| {
                DirEntry { name, file_type: FileType::BlockDevice, inode: BLOCK_INODE_BASE + idx as u64 }
            })
        }
    }
}

impl FileSystem for DevFs {
    fn name(&self) -> &'static str {
        "devfs"
    }

    fn root_inode(&self) -> u64 {
        0
    }

    fn lookup(&self, _parent_inode: u64, name: &str) -> Option<u64> {
        // Check static entries
        for e in STATIC_ENTRIES {
            if e.name == name {
                return Some(e.inode);
            }
        }
        // Check block devices
        unsafe {
            for i in 0..BLOCK_DEV_COUNT {
                if let Some((n, _)) = &BLOCK_DEVS[i] {
                    if *n == name {
                        return Some(BLOCK_INODE_BASE + i as u64);
                    }
                }
            }
        }
        None
    }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        if inode >= BLOCK_INODE_BASE {
            let idx = (inode - BLOCK_INODE_BASE) as usize;
            unsafe {
                if idx < BLOCK_DEVS.len() {
                    if let Some((_, ops)) = &BLOCK_DEVS[idx] {
                        let lba = offset / 512;
                        if (ops.read)(lba, buf) {
                            return Some(buf.len() as u64);
                        }
                        return None;
                    }
                }
            }
        }
        Some(0)
    }

    fn write(&self, inode: u64, offset: u64, buf: &[u8]) -> Option<u64> {
        match inode {
            1 => Some(buf.len() as u64),
            2 => Some(buf.len() as u64),
            3 => {
                use zenus_console::serial::SerialPort;
                let mut s = SerialPort::new(0x3F8);
                for &b in buf { s.write_byte_serial(b); }
                Some(buf.len() as u64)
            }
            4 => {
                use zenus_console::serial::SerialPort;
                let mut s = SerialPort::new(0x3F8);
                for &b in buf { s.write_byte_serial(b); }
                Some(buf.len() as u64)
            }
            _ if inode >= BLOCK_INODE_BASE => {
                let idx = (inode - BLOCK_INODE_BASE) as usize;
                unsafe {
                    if idx < BLOCK_DEVS.len() {
                        if let Some((_, ops)) = &BLOCK_DEVS[idx] {
                            let lba = offset / 512;
                            (ops.write)(lba, buf);
                            return Some(buf.len() as u64);
                        }
                    }
                }
                None
            }
            _ => None,
        }
    }

    fn read_dir(&self, inode: u64) -> &'static [DirEntry] {
        if inode != 0 {
            return &[];
        }

        unsafe {
            let total = STATIC_ENTRIES.len() + BLOCK_DEV_COUNT;
            if total <= 4 {
                return STATIC_ENTRIES;
            }
            static mut ALL_ENTRIES: [DirEntry; 12] = [DirEntry {
                name: "", file_type: FileType::None, inode: 0,
            }; 12];
            for i in 0..STATIC_ENTRIES.len() {
                ALL_ENTRIES[i] = STATIC_ENTRIES[i];
            }
            for i in 0..BLOCK_DEV_COUNT {
                let idx = STATIC_ENTRIES.len() + i;
                if idx >= ALL_ENTRIES.len() { break; }
                if let Some((name, _)) = &BLOCK_DEVS[i] {
                    ALL_ENTRIES[idx] = DirEntry {
                        name,
                        file_type: FileType::BlockDevice,
                        inode: BLOCK_INODE_BASE + i as u64,
                    };
                }
            }
            &ALL_ENTRIES[..core::cmp::min(total, ALL_ENTRIES.len())]
        }
    }

    fn stat(&self, inode: u64) -> FileStat {
        let dev_mode = |ft: FileType| FileStat {
            size: 0, file_type: ft, inode, blocks: 0, uid: 0, gid: 0, mode: 0o666,
        };
        match inode {
            0 => FileStat { size: 0, file_type: FileType::Directory, inode: 0, blocks: 0, uid: 0, gid: 0, mode: 0o555 | 0o4000 },
            1 => dev_mode(FileType::CharDevice),
            2 => dev_mode(FileType::CharDevice),
            3 => dev_mode(FileType::CharDevice),
            4 => dev_mode(FileType::CharDevice),
            _ if inode >= BLOCK_INODE_BASE => {
                let idx = (inode - BLOCK_INODE_BASE) as usize;
                unsafe {
                    if idx < BLOCK_DEVS.len() {
                        if let Some((_, ops)) = &BLOCK_DEVS[idx] {
                            return FileStat { size: ops.size, file_type: FileType::BlockDevice, inode, blocks: ops.size / 512, uid: 0, gid: 0, mode: 0o660 };
                        }
                    }
                }
                FileStat { size: 0, file_type: FileType::BlockDevice, inode, blocks: 0, uid: 0, gid: 0, mode: 0o660 }
            }
            _ => FileStat { size: 0, file_type: FileType::None, inode, blocks: 0, uid: 0, gid: 0, mode: 0 },
        }
    }

    fn create(&self, _parent_inode: u64, _name: &str, _file_type: FileType) -> Option<u64> {
        None
    }

    fn unlink(&self, _parent_inode: u64, _name: &str) -> bool {
        false
    }
}
