use zenus_console::serial::SerialPort;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FileType {
    None,
    File,
    Directory,
    BlockDevice,
    CharDevice,
    Symlink,
}

#[repr(C)]
pub struct FileStat {
    pub size: u64,
    pub file_type: FileType,
    pub inode: u64,
    pub blocks: u64,
    pub uid: u32,
    pub gid: u32,
    pub mode: u16,
}

#[derive(Debug, Clone, Copy)]
pub struct DirEntry {
    pub name: &'static str,
    pub file_type: FileType,
    pub inode: u64,
}

pub trait FileSystem {
    fn name(&self) -> &'static str;
    fn root_inode(&self) -> u64;
    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64>;
    fn write(&self, inode: u64, offset: u64, buf: &[u8]) -> Option<u64>;
    fn read_dir(&self, inode: u64) -> &'static [DirEntry];
    fn stat(&self, inode: u64) -> FileStat;
    fn create(&self, parent_inode: u64, name: &str, file_type: FileType) -> Option<u64>;
    fn unlink(&self, parent_inode: u64, name: &str) -> bool;
    fn lookup(&self, parent_inode: u64, name: &str) -> Option<u64> {
        for e in self.read_dir(parent_inode) {
            if e.name == name {
                return Some(e.inode);
            }
        }
        None
    }
    fn chmod(&self, _inode: u64, _mode: u16) -> bool { false }
    fn chown(&self, _inode: u64, _uid: u32, _gid: u32) -> bool { false }
}

pub struct VfsNode {
    pub fs: &'static dyn FileSystem,
    pub inode: u64,
}

struct Mount {
    path: &'static str,
    fs: &'static dyn FileSystem,
}

const MAX_MOUNTS: usize = 8;

static mut MOUNTS: [Mount; MAX_MOUNTS] = [
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
    Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem },
];
static mut MOUNT_COUNT: usize = 1;

static mut VFS_ROOT: Option<VfsNode> = None;

pub fn init() {
    let mut s = SerialPort::new(0x3F8);

    let tmp_fs = crate::tmpfs::TmpFs::new();
    let root = VfsNode {
        fs: tmp_fs,
        inode: tmp_fs.root_inode(),
    };
    unsafe {
        VFS_ROOT = Some(root);
        MOUNTS[0] = Mount { path: "/", fs: tmp_fs };
        MOUNT_COUNT = 1;
    };

    s.write_str("[OK] VFS initialized (tmpfs root)\n");
}

pub fn mount(path: &'static str, fs: &'static dyn FileSystem) -> bool {
    unsafe {
        if MOUNT_COUNT >= MAX_MOUNTS {
            return false;
        }
        let i = MOUNT_COUNT;
        MOUNTS[i] = Mount { path, fs };
        MOUNT_COUNT += 1;
        true
    }
}

fn find_mount(path: &str) -> Option<&'static (dyn FileSystem + 'static)> {
    unsafe {
        let mut best: Option<&dyn FileSystem> = None;
        let mut best_len = 0usize;
        for i in 0..MOUNT_COUNT {
            let m = &MOUNTS[i];
            if path.starts_with(m.path) && m.path.len() > best_len {
                best = Some(m.fs);
                best_len = m.path.len();
            }
        }
        best
    }
}

pub fn root() -> Option<&'static VfsNode> {
    unsafe { VFS_ROOT.as_ref() }
}

pub fn create_file(path: &str) -> bool {
    let parent = match parent_dir(path) {
        Some(p) => p,
        None => return false,
    };
    let name = file_name(path);
    match open(&parent) {
        Some(node) => node.fs.create(node.inode, name, FileType::File).is_some(),
        None => false,
    }
}

pub fn create_dir(path: &str) -> bool {
    let parent = match parent_dir(path) {
        Some(p) => p,
        None => return false,
    };
    let name = file_name(path);
    match open(&parent) {
        Some(node) => node.fs.create(node.inode, name, FileType::Directory).is_some(),
        None => false,
    }
}

pub fn remove(path: &str) -> bool {
    let parent = match parent_dir(path) {
        Some(p) => p,
        None => return false,
    };
    let name = file_name(path);
    match open(&parent) {
        Some(node) => node.fs.unlink(node.inode, name),
        None => false,
    }
}

fn parent_dir<'a>(path: &'a str) -> Option<&'a str> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { return None; }
    match trimmed.rfind('/') {
        Some(pos) if pos == 0 => Some("/"),
        Some(pos) => Some(&trimmed[..pos]),
        None => Some("/"),
    }
}

fn file_name<'a>(path: &'a str) -> &'a str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    }
}

/// Read directory entries for a given VFS path, merging mount points into the listing.
pub fn read_dir(path: &str) -> &'static [DirEntry] {
    // For root, merge underlying fs entries with mount points
    if path == "/" || path.is_empty() {
        return read_dir_root();
    }

    match open(path) {
        Some(node) => node.fs.read_dir(node.inode),
        None => &[],
    }
}

fn read_dir_root() -> &'static [DirEntry] {
    // Max entries: devfs entries + mount points
    const MAX: usize = 16;
    static mut BUF: [DirEntry; MAX] = [DirEntry {
        name: "", file_type: FileType::None, inode: 0,
    }; MAX];
    static mut COUNT: usize = 0;

    unsafe {
        COUNT = 0;

        // Add devfs entries from root fs
        if let Some(root_node) = root() {
            for e in root_node.fs.read_dir(root_node.inode) {
                if COUNT < MAX {
                    BUF[COUNT] = *e;
                    COUNT += 1;
                }
            }
        }

        // Add mount point directories (skip "/")
        for i in 1..MOUNT_COUNT {
            let m = &MOUNTS[i];
            if COUNT >= MAX { break; }
            let dir_name = m.path.trim_start_matches('/');
            if !dir_name.is_empty() {
                // deduplicate
                let mut dup = false;
                for j in 0..COUNT {
                    if BUF[j].name == dir_name {
                        dup = true;
                        break;
                    }
                }
                if !dup {
                    BUF[COUNT] = DirEntry {
                        name: dir_name,
                        file_type: FileType::Directory,
                        inode: 0,
                    };
                    COUNT += 1;
                }
            }
        }

        &BUF[..COUNT]
    }
}

pub fn open(path: &str) -> Option<VfsNode> {
    if path == "/" || path.is_empty() {
        return root().map(|r| VfsNode { fs: r.fs, inode: r.inode });
    }

    let fs = find_mount(path)?;
    let root_inode = fs.root_inode();

    // Find relative path by stripping the mount prefix
    let mount_prefix = unsafe {
        let mut longest = "";
        for i in 0..MOUNT_COUNT {
            let m = &MOUNTS[i];
            if core::ptr::eq(m.fs as *const _, fs as *const _) {
                if m.path.len() > longest.len() {
                    longest = m.path;
                }
            }
        }
        longest
    };

    let rel_path = if mount_prefix.is_empty() || mount_prefix == "/" {
        path
    } else if path.starts_with(mount_prefix) {
        let rest = &path[mount_prefix.len()..];
        if rest.is_empty() { "/" } else { rest }
    } else {
        path
    };

    if rel_path == "/" || rel_path.is_empty() {
        return Some(VfsNode { fs, inode: root_inode });
    }

    let trimmed = rel_path.trim_start_matches('/');
    let mut current = VfsNode { fs, inode: root_inode };
    let mut path_segments: [&str; 32] = [""; 32];
    let mut seg_count = 0;

    for part in trimmed.split('/') {
        if part.is_empty() || part == "." { continue; }
        if part == ".." {
            if seg_count > 0 { seg_count -= 1; }
            continue;
        }
        if seg_count < 32 {
            path_segments[seg_count] = part;
            seg_count += 1;
        }
    }

    for i in 0..seg_count {
        let part = path_segments[i];
        match current.fs.lookup(current.inode, part) {
            Some(inode) => { current = VfsNode { fs: current.fs, inode }; }
            None => { return None; }
        }
    }
    Some(current)
}

pub const S_IRUSR: u16 = 0o400;
pub const S_IWUSR: u16 = 0o200;
pub const S_IXUSR: u16 = 0o100;
pub const S_IRGRP: u16 = 0o040;
pub const S_IWGRP: u16 = 0o020;
pub const S_IXGRP: u16 = 0o010;
pub const S_IROTH: u16 = 0o004;
pub const S_IWOTH: u16 = 0o002;
pub const S_IXOTH: u16 = 0o001;
pub const S_IFREG: u16 = 0x8000;
pub const S_IFDIR: u16 = 0x4000;
pub const DEFAULT_FILE_MODE: u16 = 0x81A4;
pub const DEFAULT_DIR_MODE: u16 = 0x41ED;

pub fn access_check(_uid: u32, _gid: u32, euid: u32, egid: u32, stat: &FileStat, want_write: bool) -> bool {
    let mode = stat.mode;
    if euid == 0 {
        return true;
    }
    let mut ok = false;
    if euid == stat.uid {
        if !want_write || (mode & S_IWUSR) != 0 { ok = true; }
        if want_write && (mode & S_IXUSR) == 0 && stat.file_type == FileType::Directory { ok = false; }
    } else if egid == stat.gid {
        if !want_write || (mode & S_IWGRP) != 0 { ok = true; }
        if want_write && (mode & S_IXGRP) == 0 && stat.file_type == FileType::Directory { ok = false; }
    } else {
        if !want_write || (mode & S_IWOTH) != 0 { ok = true; }
    }
    ok
}

pub fn perm_str(mode: u16) -> &'static str {
    static mut BUF: [u8; 11] = [b'-'; 11];
    unsafe {
        BUF[0..10].copy_from_slice(b"----------");
        let ft = (mode >> 12) & 0xF;
        BUF[0] = match ft {
            0x4 => b'd',
            0x8 => b'-',
            0x2 => b'c',
            0x6 => b'b',
            0xA => b'l',
            _ => b'?',
        };
        if mode & S_IRUSR != 0 { BUF[1] = b'r'; }
        if mode & S_IWUSR != 0 { BUF[2] = b'w'; }
        if mode & S_IXUSR != 0 { BUF[3] = b'x'; }
        if mode & S_IRGRP != 0 { BUF[4] = b'r'; }
        if mode & S_IWGRP != 0 { BUF[5] = b'w'; }
        if mode & S_IXGRP != 0 { BUF[6] = b'x'; }
        if mode & S_IROTH != 0 { BUF[7] = b'r'; }
        if mode & S_IWOTH != 0 { BUF[8] = b'w'; }
        if mode & S_IXOTH != 0 { BUF[9] = b'x'; }
        core::str::from_utf8_unchecked(&BUF[..10])
    }
}

#[cfg(feature = "testing")]
pub mod tests {
    use super::*;

    pub fn test_parent_dir_root() -> Result<(), &'static str> {
        if parent_dir("/") != None {
            return Err("parent_dir('/') should be None");
        }
        Ok(())
    }

    pub fn test_parent_dir_simple() -> Result<(), &'static str> {
        match parent_dir("/foo/bar") {
            Some(p) if p == "/foo" => Ok(()),
            Some(_) => Err("wrong parent_dir('/foo/bar') result"),
            None => Err("parent_dir('/foo/bar') returned None"),
        }
    }

    pub fn test_parent_dir_top_level() -> Result<(), &'static str> {
        match parent_dir("/foo") {
            Some(p) if p == "/" => Ok(()),
            _ => Err("parent_dir('/foo') should be Some('/')"),
        }
    }

    pub fn test_parent_dir_trailing_slash() -> Result<(), &'static str> {
        match parent_dir("/foo/bar/") {
            Some(p) if p == "/foo" => Ok(()),
            _ => Err("parent_dir('/foo/bar/') should be Some('/foo')"),
        }
    }

    pub fn test_file_name_simple() -> Result<(), &'static str> {
        if file_name("/foo/bar.txt") != "bar.txt" {
            return Err("file_name('/foo/bar.txt') should be 'bar.txt'");
        }
        Ok(())
    }

    pub fn test_file_name_root() -> Result<(), &'static str> {
        if file_name("/") != "" {
            return Err("file_name('/') should be empty");
        }
        Ok(())
    }

    pub fn test_file_name_top() -> Result<(), &'static str> {
        if file_name("/foo") != "foo" {
            return Err("file_name('/foo') should be 'foo'");
        }
        Ok(())
    }

    pub fn test_file_name_trailing_slash() -> Result<(), &'static str> {
        if file_name("/foo/bar/") != "bar" {
            return Err("file_name('/foo/bar/') should be 'bar'");
        }
        Ok(())
    }
}
