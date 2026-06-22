use core::slice;
use crate::vfs::{self, FileSystem, FileType, FileStat, DirEntry};
use zenus_sync::spinlock::SpinLock;

#[repr(C, packed)]
struct UstarHeader {
    name: [u8; 100],
    mode: [u8; 8],
    uid: [u8; 8],
    gid: [u8; 8],
    size: [u8; 12],
    mtime: [u8; 12],
    checksum: [u8; 8],
    type_flag: u8,
    link_name: [u8; 100],
    magic: [u8; 6],
    version: [u8; 2],
    uname: [u8; 32],
    gname: [u8; 32],
    dev_major: [u8; 8],
    dev_minor: [u8; 8],
    prefix: [u8; 155],
    padding: [u8; 12],
}

fn parse_octal(buf: &[u8]) -> u64 {
    let s = core::str::from_utf8(buf).unwrap_or("0");
    u64::from_str_radix(s.trim_end_matches('\0'), 8).unwrap_or(0)
}

fn name_skip_prefix(full: &str) -> &str {
    if let Some(stripped) = full.strip_prefix("./") {
        stripped
    } else {
        full
    }
}

#[derive(Debug, Clone, Copy)]
struct TarEntry {
    inode: u64,
    name: &'static str,
    file_type: FileType,
    data_off: u64,
    data_len: u64,
}

const MAX_ENTRIES: usize = 64;
const MAX_DIR_ENTRIES: usize = 128;

fn copy_name(name: &str) -> &'static str {
    static mut NAME_BUF: [u8; 4096] = [0; 4096];
    static mut NAME_OFF: usize = 0;
    let bytes = name.as_bytes();
    let len = bytes.len().min(255);
    unsafe {
        let off = NAME_OFF;
        if off + len + 1 > NAME_BUF.len() {
            return "";
        }
        let dst = &mut NAME_BUF[off..off + len];
        dst.copy_from_slice(&bytes[..len]);
        NAME_OFF = off + len + 1;
        core::str::from_utf8(&NAME_BUF[off..off + len]).unwrap_or("")
    }
}

pub struct TarFs {
    entries: &'static [TarEntry],
    data_base: u64,
}

impl TarFs {
    pub fn load(addr: u64, len: u64) -> Option<&'static Self> {
        static mut ENTRIES: [TarEntry; MAX_ENTRIES] = [TarEntry {
            inode: 0, name: "", file_type: FileType::None,
            data_off: 0, data_len: 0,
        }; MAX_ENTRIES];
        static mut FS: TarFs = TarFs {
            entries: &[],
            data_base: 0,
        };

        let data = unsafe { slice::from_raw_parts(addr as *const u8, len as usize) };
        let mut count = 0usize;
        let mut offset = 0usize;

        while offset + 512 <= len as usize && count < MAX_ENTRIES {
            let hdr = unsafe { &*(data.as_ptr().add(offset) as *const UstarHeader) };
            // ustar magic can be "ustar\0" (POSIX) or "ustar " (GNU)
            if &hdr.magic[..5] != b"ustar" {
                break;
            }

            let raw_name = core::str::from_utf8(&hdr.name).unwrap_or("");
            let file_name = name_skip_prefix(raw_name.trim_end_matches('\0'));
            let file_size = parse_octal(&hdr.size) as usize;
            let entry_type = hdr.type_flag;

                if !file_name.is_empty() && file_name != "." {
                let ft = match entry_type {
                    b'5' => FileType::Directory,
                    b'0' | b'\0' => FileType::File,
                    _ => FileType::None,
                };
                if ft != FileType::None {
                    // Normalize: strip trailing '/' on directory names so root
                    // read_dir and open() path lookups work correctly.
                    let normalized = if ft == FileType::Directory && file_name.ends_with('/') {
                        &file_name[..file_name.len() - 1]
                    } else {
                        file_name
                    };
                    unsafe {
                        ENTRIES[count] = TarEntry {
                            inode: count as u64 + 1,
                            name: copy_name(normalized),
                            file_type: ft,
                            data_off: addr + offset as u64 + 512,
                            data_len: file_size as u64,
                        };
                    }
                    count += 1;
                }
            }

            offset += 512;
            if entry_type == b'0' || entry_type == b'\0' {
                offset += (file_size + 511) / 512 * 512;
            }
        }

        if count == 0 {
            return None;
        }

        unsafe {
            FS.data_base = addr;
            FS.entries = core::slice::from_raw_parts(ENTRIES.as_ptr(), count);
            Some(&FS)
        }
    }

    fn find_inode(&self, name: &str) -> Option<u64> {
        self.entries.iter()
            .find(|e| e.name == name)
            .map(|e| e.inode)
    }
}

impl FileSystem for TarFs {
    fn name(&self) -> &'static str {
        "tarfs"
    }

    fn root_inode(&self) -> u64 {
        0
    }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let entry = self.entries.iter().find(|e| e.inode == inode)?;
        if entry.file_type != FileType::File {
            return Some(0);
        }
        if offset >= entry.data_len {
            return Some(0);
        }
        let read_len = core::cmp::min(buf.len() as u64, entry.data_len - offset) as usize;
        unsafe {
            core::ptr::copy_nonoverlapping(
                (entry.data_off + offset) as *const u8,
                buf.as_mut_ptr(),
                read_len,
            );
        }
        Some(read_len as u64)
    }

    fn write(&self, _inode: u64, _offset: u64, _buf: &[u8]) -> Option<u64> {
        // TarFs is read-only (initrd)
        None
    }

    fn read_dir(&self, inode: u64) -> &'static [DirEntry] {
        static TARFS_DIR_LOCK: SpinLock<()> = SpinLock::new(());
        let _rd_guard = TARFS_DIR_LOCK.lock();
        static mut DIR_BUF: [DirEntry; MAX_DIR_ENTRIES] = [DirEntry {
            name: "", file_type: FileType::None, inode: 0,
        }; MAX_DIR_ENTRIES];
        static mut DIR_COUNT: usize = 0;

        unsafe {
            DIR_COUNT = 0;
            let mut count = 0;

            // Get directory entry name; normalize to always end with '/'
            let dir_name: &str = if inode == 0 {
                ""
            } else {
                match self.entries.iter().find(|e| e.inode == inode) {
                    Some(e) => e.name,
                    None => return &[],
                }
            };
            let dir_ends_slash = dir_name.ends_with('/');

            for entry in self.entries.iter() {
                if count >= MAX_DIR_ENTRIES {
                    break;
                }

                let path = entry.name;

                // Determine the immediate child name
                let child = if inode == 0 {
                    // Root: only top-level entries (no '/')
                    if path.contains('/') {
                        continue;
                    }
                    path
                } else {
                    // Subdir: must start with dir_name
                    if !path.starts_with(dir_name) || path.len() <= dir_name.len() {
                        continue;
                    }
                    let after_dir = &path[dir_name.len()..];
                    // If dir_name doesn't end with '/', after_dir must start with '/'
                    let after_slash = if dir_ends_slash {
                        after_dir
                    } else {
                        if !after_dir.starts_with('/') {
                            continue;
                        }
                        &after_dir[1..]
                    };
                    if after_slash.is_empty() {
                        continue;
                    }
                    // Take only the first path component
                    match after_slash.find('/') {
                        Some(i) => &after_slash[..i],
                        None => after_slash,
                    }
                };

                if child.is_empty() {
                    continue;
                }

                // Deduplicate
                let mut dup = false;
                for j in 0..count {
                    if DIR_BUF[j].name == child {
                        dup = true;
                        break;
                    }
                }
                if dup {
                    continue;
                }

                // Find the entry that best represents this child to get its real type/inode.
                // Match entry.name that is exactly dir_name + separator + child
                // (possibly with trailing / for subdirectories).
                let child_entry = if inode == 0 {
                    Some(entry)
                } else {
                    self.entries.iter().find(|e| {
                        let en = e.name;
                        if !en.starts_with(dir_name) || en.len() <= dir_name.len() {
                            return false;
                        }
                        let rest = &en[dir_name.len()..];
                        let rest = if dir_ends_slash { rest } else {
                            if !rest.starts_with('/') { return false; }
                            &rest[1..]
                        };
                        // Check if rest starts with child and is either equal or followed by '/'
                        if rest.len() < child.len() { return false; }
                        if !rest.starts_with(child) { return false; }
                        if rest.len() == child.len() { return true; }
                        rest.as_bytes()[child.len()] == b'/'
                    })
                };

                let (ft, child_ino) = match child_entry {
                    Some(e) => (e.file_type, e.inode),
                    None => (FileType::File, 0),
                };

                DIR_BUF[count] = DirEntry {
                    name: child,
                    file_type: ft,
                    inode: child_ino,
                };
                count += 1;
            }

            DIR_COUNT = count;
            &DIR_BUF[..count]
        }
    }

    fn stat(&self, inode: u64) -> FileStat {
        if inode == 0 {
            return FileStat {
                size: 0, file_type: FileType::Directory, inode: 0, blocks: 0, uid: 0, gid: 0,                 mode: vfs::DEFAULT_DIR_MODE,
            };
        }
        match self.entries.iter().find(|e| e.inode == inode) {
            Some(e) => FileStat {
                size: e.data_len,
                file_type: e.file_type,
                inode: e.inode,
                blocks: (e.data_len + 511) / 512,
                uid: 0,
                gid: 0,
                mode: vfs::DEFAULT_FILE_MODE,
            },
            None => FileStat {
                size: 0, file_type: FileType::None, inode, blocks: 0, uid: 0, gid: 0, mode: 0,
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
