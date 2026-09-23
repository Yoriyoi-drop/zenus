use crate::vfs::{DirEntry, FileStat, FileSystem, FileType};
use zenus_sync::spinlock::SpinLock;

const MAX_BLOCK_DEVS: usize = 8;

#[derive(Clone, Copy)]
pub struct BlockDeviceOps {
    pub read: fn(u64, &mut [u8]) -> bool,
    pub write: fn(u64, &[u8]) -> bool,
    pub size: u64,
}

const DEVFS_NAMES: [&str; 4] = ["null", "zero", "console", "serial"];
const DEVFS_TYPES: [FileType; 4] = [
    FileType::CharDevice,
    FileType::CharDevice,
    FileType::CharDevice,
    FileType::CharDevice,
];
const DEVFS_INODES: [u64; 4] = [1, 2, 3, 4];

const BLOCK_INODE_BASE: u64 = 64;

struct BlockDevState {
    devs: [Option<(&'static str, BlockDeviceOps)>; MAX_BLOCK_DEVS],
    count: usize,
}

static BLOCK_DEV_STATE: SpinLock<BlockDevState> = SpinLock::new(BlockDevState {
    devs: [None; MAX_BLOCK_DEVS],
    count: 0,
});

fn with_block_devs<R>(f: impl FnOnce(&BlockDevState) -> R) -> R {
    let state = BLOCK_DEV_STATE.lock();
    f(&state)
}

fn with_block_devs_mut<R>(f: impl FnOnce(&mut BlockDevState) -> R) -> R {
    let mut state = BLOCK_DEV_STATE.lock();
    f(&mut state)
}

pub fn block_device_read(dev_idx: usize, lba: u64, buf: &mut [u8]) -> bool {
    with_block_devs(|state| {
        state
            .devs
            .get(dev_idx)
            .and_then(|o| o.as_ref())
            .map(|(_, ops)| (ops.read)(lba, buf))
            .unwrap_or(false)
    })
}

pub fn block_device_write(dev_idx: usize, lba: u64, buf: &[u8]) -> bool {
    with_block_devs(|state| {
        state
            .devs
            .get(dev_idx)
            .and_then(|o| o.as_ref())
            .map(|(_, ops)| (ops.write)(lba, buf))
            .unwrap_or(false)
    })
}

pub fn register_block_device(name: &'static str, ops: BlockDeviceOps) -> bool {
    with_block_devs_mut(|state| {
        if state.count >= MAX_BLOCK_DEVS {
            return false;
        }
        state.devs[state.count] = Some((name, ops));
        state.count += 1;
        true
    })
}

pub fn block_device_count() -> usize {
    with_block_devs(|state| state.count)
}

pub struct DevFs;

impl DevFs {
    fn block_entry_at(&self, idx: usize) -> Option<DirEntry> {
        with_block_devs(|state| {
            state
                .devs
                .get(idx)
                .and_then(|o| o.as_ref())
                .map(|(name, _)| DirEntry {
                    name: alloc::string::String::from(*name),
                    file_type: FileType::BlockDevice,
                    inode: BLOCK_INODE_BASE + idx as u64,
                })
        })
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
        for i in 0..DEVFS_NAMES.len() {
            if DEVFS_NAMES[i] == name {
                return Some(DEVFS_INODES[i]);
            }
        }
        with_block_devs(|state| {
            for i in 0..state.count {
                if let Some((n, _)) = &state.devs[i] {
                    if *n == name {
                        return Some(BLOCK_INODE_BASE + i as u64);
                    }
                }
            }
            None
        })
    }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        if inode >= BLOCK_INODE_BASE {
            let idx = (inode - BLOCK_INODE_BASE) as usize;
            return with_block_devs(|state| {
                if idx < state.devs.len() {
                    if let Some((_, ops)) = &state.devs[idx] {
                        let sector = offset / 512;
                        let off_in_sector = (offset % 512) as usize;
                        let mut sector_buf = [0u8; 512];
                        if !(ops.read)(sector, &mut sector_buf) {
                            return None;
                        }
                        let copy_len = core::cmp::min(buf.len(), 512 - off_in_sector);
                        buf[..copy_len]
                            .copy_from_slice(&sector_buf[off_in_sector..off_in_sector + copy_len]);
                        return Some(copy_len as u64);
                    }
                }
                Some(0)
            });
        }
        Some(0)
    }

    fn write(&self, inode: u64, offset: u64, buf: &[u8]) -> Option<u64> {
        match inode {
            1 => Some(buf.len() as u64),
            2 => Some(buf.len() as u64),
            3 => {
                use zenus_console::serial::SerialPort;
                let s = SerialPort::new(0x3F8);
                for &b in buf {
                    s.write_byte_serial(b);
                }
                Some(buf.len() as u64)
            }
            4 => {
                use zenus_console::serial::SerialPort;
                let s = SerialPort::new(0x3F8);
                for &b in buf {
                    s.write_byte_serial(b);
                }
                Some(buf.len() as u64)
            }
            _ if inode >= BLOCK_INODE_BASE => {
                let idx = (inode - BLOCK_INODE_BASE) as usize;
                with_block_devs_mut(|state| {
                    if idx < state.devs.len() {
                        if let Some((_, ops)) = &state.devs[idx] {
                            let sector = offset / 512;
                            let off_in_sector = (offset % 512) as usize;
                            let mut sector_buf = [0u8; 512];
                            if off_in_sector != 0 || buf.len() < 512 {
                                if !(ops.read)(sector, &mut sector_buf) {
                                    sector_buf = [0; 512];
                                }
                            }
                            let copy_len = core::cmp::min(buf.len(), 512 - off_in_sector);
                            sector_buf[off_in_sector..off_in_sector + copy_len]
                                .copy_from_slice(&buf[..copy_len]);
                            if !(ops.write)(sector, &sector_buf) {
                                return None;
                            }
                            return Some(copy_len as u64);
                        }
                    }
                    None
                })
            }
            _ => None,
        }
    }

    fn read_dir(&self, inode: u64) -> alloc::vec::Vec<DirEntry> {
        if inode != 0 {
            return alloc::vec::Vec::new();
        }
        let mut entries = alloc::vec::Vec::with_capacity(12);
        for i in 0..DEVFS_NAMES.len() {
            entries.push(DirEntry {
                name: alloc::string::String::from(DEVFS_NAMES[i]),
                file_type: DEVFS_TYPES[i],
                inode: DEVFS_INODES[i],
            });
        }
        with_block_devs(|state| {
            for i in 0..state.count {
                if entries.len() >= 12 {
                    return;
                }
                if let Some((name, _)) = &state.devs[i] {
                    entries.push(DirEntry {
                        name: alloc::string::String::from(*name),
                        file_type: FileType::BlockDevice,
                        inode: BLOCK_INODE_BASE + i as u64,
                    });
                }
            }
        });
        entries
    }

    fn stat(&self, inode: u64) -> FileStat {
        let dev_mode = |ft: FileType| FileStat {
            size: 0,
            file_type: ft,
            inode,
            blocks: 0,
            uid: 0,
            gid: 0,
            mode: 0o666,
        };
        match inode {
            0 => FileStat {
                size: 0,
                file_type: FileType::Directory,
                inode: 0,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o555 | 0o4000,
            },
            1 => dev_mode(FileType::CharDevice),
            2 => dev_mode(FileType::CharDevice),
            3 => dev_mode(FileType::CharDevice),
            4 => dev_mode(FileType::CharDevice),
            _ if inode >= BLOCK_INODE_BASE => with_block_devs(|state| {
                let idx = (inode - BLOCK_INODE_BASE) as usize;
                if idx < state.devs.len() {
                    if let Some((_, ops)) = &state.devs[idx] {
                        return FileStat {
                            size: ops.size,
                            file_type: FileType::BlockDevice,
                            inode,
                            blocks: ops.size / 512,
                            uid: 0,
                            gid: 0,
                            mode: 0o660,
                        };
                    }
                }
                FileStat {
                    size: 0,
                    file_type: FileType::BlockDevice,
                    inode,
                    blocks: 0,
                    uid: 0,
                    gid: 0,
                    mode: 0o660,
                }
            }),
            _ => FileStat {
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
}
