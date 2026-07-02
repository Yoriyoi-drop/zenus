
use zenus_sync::spinlock::SpinLock;

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

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: alloc::string::String,
    pub file_type: FileType,
    pub inode: u64,
}

pub trait FileSystem: Send + Sync {
    fn name(&self) -> &'static str;
    fn root_inode(&self) -> u64;
    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64>;
    fn write(&self, inode: u64, offset: u64, buf: &[u8]) -> Option<u64>;
    fn read_dir(&self, inode: u64) -> alloc::vec::Vec<DirEntry>;
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

#[derive(Clone, Copy)]
pub struct VfsNode {
    pub fs: &'static dyn FileSystem,
    pub inode: u64,
}

#[derive(Clone, Copy)]
struct Mount {
    path: &'static str,
    fs: &'static dyn FileSystem,
}

const MAX_MOUNTS: usize = 32;

#[derive(Clone, Copy)]
struct MountTable {
    mounts: [Mount; MAX_MOUNTS],
    count: usize,
}

const EMPTY_MOUNT: Mount = Mount { path: "", fs: &crate::devfs::DevFs as &dyn FileSystem };

fn empty_dir_entry() -> DirEntry {
    DirEntry {
        name: alloc::string::String::new(),
        file_type: FileType::None,
        inode: 0,
    }
}

impl MountTable {
    const fn new() -> Self {
        MountTable {
            mounts: [EMPTY_MOUNT; MAX_MOUNTS],
            count: 0,
        }
    }
}

static MOUNT_TABLE: SpinLock<MountTable> = SpinLock::new(MountTable::new());
static VFS_ROOT: SpinLock<Option<VfsNode>> = SpinLock::new(None);

// Per-namespace mount tables
const MAX_MNT_NS: usize = 8;

#[derive(Clone, Copy)]
struct MntNsEntry {
    ns_id: zenus_ns::NsId,
    table: MountTable,
}

#[derive(Clone, Copy)]
struct MntNsTables {
    entries: [Option<MntNsEntry>; MAX_MNT_NS],
    count: usize,
}

impl MntNsTables {
    const fn new() -> Self {
        MntNsTables {
            entries: [None; MAX_MNT_NS],
            count: 0,
        }
    }
}

static MNT_NS_TABLES: SpinLock<MntNsTables> = SpinLock::new(MntNsTables::new());

/// Copy the root mount table into a new mount namespace.
pub fn create_mnt_ns(ns_id: zenus_ns::NsId) -> bool {
    let root_table = MOUNT_TABLE.lock();
    let mut tables = MNT_NS_TABLES.lock();
    if tables.count >= MAX_MNT_NS {
        return false;
    }
    let idx = tables.count;
    tables.entries[idx] = Some(MntNsEntry {
        ns_id,
        table: MountTable {
            mounts: root_table.mounts,
            count: root_table.count,
        },
    });
    tables.count = idx + 1;
    true
}

fn with_mount_table<R>(ns_id: zenus_ns::NsId, f: impl FnOnce(&mut MountTable) -> R) -> R {
    if ns_id == zenus_ns::NS_ROOT {
        let mut mt = MOUNT_TABLE.lock();
        f(&mut mt)
    } else {
        let mut tables = MNT_NS_TABLES.lock();
        for i in 0..tables.count {
            if let Some(ref mut entry) = tables.entries[i] {
                if entry.ns_id == ns_id {
                    return f(&mut entry.table);
                }
            }
        }
        // Fallback to root
        let mut mt = MOUNT_TABLE.lock();
        f(&mut mt)
    }
}

fn find_mount_in_table(ns_id: zenus_ns::NsId, path: &str) -> Option<(&'static (dyn FileSystem + 'static), &'static str)> {
    if ns_id == zenus_ns::NS_ROOT {
        let mt = MOUNT_TABLE.lock();
        let mut best: Option<(&dyn FileSystem, &str)> = None;
        let mut best_len = 0usize;
        for i in 0..mt.count {
            let m = &mt.mounts[i];
            if path.starts_with(m.path) && m.path.len() > best_len {
                best = Some((m.fs, m.path));
                best_len = m.path.len();
            }
        }
        return best;
    }
    let tables = MNT_NS_TABLES.lock();
    for i in 0..tables.count {
        if let Some(ref entry) = tables.entries[i] {
            if entry.ns_id == ns_id {
                let mut best: Option<(&dyn FileSystem, &str)> = None;
                let mut best_len = 0usize;
                for j in 0..entry.table.count {
                    let m = &entry.table.mounts[j];
                    if path.starts_with(m.path) && m.path.len() > best_len {
                        best = Some((m.fs, m.path));
                        best_len = m.path.len();
                    }
                }
                return best;
            }
        }
    }
    find_mount_to_pair(path)
}

fn find_mount_to_pair(path: &str) -> Option<(&'static (dyn FileSystem + 'static), &'static str)> {
    let mt = MOUNT_TABLE.lock();
    let mut best: Option<(&dyn FileSystem, &str)> = None;
    let mut best_len = 0usize;
    for i in 0..mt.count {
        let m = &mt.mounts[i];
        if path.starts_with(m.path) && m.path.len() > best_len {
            best = Some((m.fs, m.path));
            best_len = m.path.len();
        }
    }
    best
}

pub fn init() {
    let tmp_fs = crate::tmpfs::TmpFs::new();
    let root = VfsNode {
        fs: tmp_fs,
        inode: tmp_fs.root_inode(),
    };
    {
        let mut root_lock = VFS_ROOT.lock();
        *root_lock = Some(root);
    }
    {
        let mut mt = MOUNT_TABLE.lock();
        mt.mounts[0] = Mount { path: "/", fs: tmp_fs };
        mt.count = 1;
    }

    zenus_console::kinfo!("VFS initialized");
}

pub fn mount(path: &'static str, fs: &'static dyn FileSystem) -> bool {
    mount_in_ns(zenus_ns::NS_ROOT, path, fs)
}

pub fn mount_in_ns(ns_id: zenus_ns::NsId, path: &'static str, fs: &'static dyn FileSystem) -> bool {
    if ns_id != zenus_ns::NS_ROOT {
        let mut tables = MNT_NS_TABLES.lock();
        for i in 0..tables.count {
            if let Some(ref mut entry) = tables.entries[i] {
                if entry.ns_id == ns_id {
                    if entry.table.count >= MAX_MOUNTS {
                        return false;
                    }
                    let j = entry.table.count;
                    entry.table.mounts[j] = Mount { path, fs };
                    entry.table.count += 1;
                    return true;
                }
            }
        }
    }
    // Fallback to root
    let mut mt = MOUNT_TABLE.lock();
    if mt.count >= MAX_MOUNTS {
        return false;
    }
    let i = mt.count;
    mt.mounts[i] = Mount { path, fs };
    mt.count += 1;
    true
}

pub fn root() -> Option<VfsNode> {
    let root_lock = VFS_ROOT.lock();
    root_lock.clone()
}

pub fn create_file(path: &str) -> bool {
    create_file_in_ns(zenus_ns::NS_ROOT, path)
}

pub fn create_file_in_ns(ns_id: zenus_ns::NsId, path: &str) -> bool {
    let parent = match parent_dir(path) {
        Some(p) => p,
        None => return false,
    };
    let name = file_name(path);
    match open_in_ns(ns_id, &parent) {
        Some(node) => node.fs.create(node.inode, name, FileType::File).is_some(),
        None => false,
    }
}

pub fn create_dir(path: &str) -> bool {
    create_dir_in_ns(zenus_ns::NS_ROOT, path)
}

pub fn create_dir_in_ns(ns_id: zenus_ns::NsId, path: &str) -> bool {
    let parent = match parent_dir(path) {
        Some(p) => p,
        None => return false,
    };
    let name = file_name(path);
    match open_in_ns(ns_id, &parent) {
        Some(node) => node.fs.create(node.inode, name, FileType::Directory).is_some(),
        None => false,
    }
}

pub fn remove(path: &str) -> bool {
    remove_in_ns(zenus_ns::NS_ROOT, path)
}

pub fn remove_in_ns(ns_id: zenus_ns::NsId, path: &str) -> bool {
    let parent = match parent_dir(path) {
        Some(p) => p,
        None => return false,
    };
    let name = file_name(path);
    match open_in_ns(ns_id, &parent) {
        Some(node) => node.fs.unlink(node.inode, name),
        None => false,
    }
}

pub(crate) fn parent_dir<'a>(path: &'a str) -> Option<&'a str> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() { return None; }
    match trimmed.rfind('/') {
        Some(pos) if pos == 0 => Some("/"),
        Some(pos) => Some(&trimmed[..pos]),
        None => Some("/"),
    }
}

pub(crate) fn file_name<'a>(path: &'a str) -> &'a str {
    let trimmed = path.trim_end_matches('/');
    match trimmed.rfind('/') {
        Some(pos) => &trimmed[pos + 1..],
        None => trimmed,
    }
}

/// Read directory entries for a given VFS path, merging mount points into the listing.
pub fn read_dir(path: &str) -> alloc::vec::Vec<DirEntry> {
    read_dir_in_ns(zenus_ns::NS_ROOT, path)
}

pub fn read_dir_in_ns(ns_id: zenus_ns::NsId, path: &str) -> alloc::vec::Vec<DirEntry> {
    if path == "/" || path.is_empty() {
        return read_dir_root_in_ns(ns_id);
    }

    match open_in_ns(ns_id, path) {
        Some(node) => node.fs.read_dir(node.inode),
        None => alloc::vec::Vec::new(),
    }
}

fn read_dir_root() -> alloc::vec::Vec<DirEntry> {
    read_dir_root_in_ns(zenus_ns::NS_ROOT)
}

fn read_dir_root_in_ns(ns_id: zenus_ns::NsId) -> alloc::vec::Vec<DirEntry> {
    let mut entries = alloc::vec::Vec::with_capacity(32);

    if let Some(root_node) = root() {
        for e in root_node.fs.read_dir(root_node.inode) {
            entries.push(e);
        }
    }

    let (mount_count, mounts_copy) = if ns_id == zenus_ns::NS_ROOT {
        let mt = MOUNT_TABLE.lock();
        (mt.count, mt.mounts)
    } else {
        let tables = MNT_NS_TABLES.lock();
        let mut idx = None;
        for i in 0..tables.count {
            if let Some(ref entry) = tables.entries[i] {
                if entry.ns_id == ns_id {
                    idx = Some(i);
                    break;
                }
            }
        }
        match idx {
            Some(i) => (tables.entries[i].unwrap().table.count, tables.entries[i].unwrap().table.mounts),
            None => (0, [EMPTY_MOUNT; MAX_MOUNTS]),
        }
    };

    for i in 1..mount_count {
        let m = &mounts_copy[i];
        let dir_name = m.path.trim_start_matches('/');
        if !dir_name.is_empty() {
            let dup = entries.iter().any(|e| e.name == dir_name);
            if !dup {
                entries.push(DirEntry {
                    name: alloc::string::String::from(dir_name),
                    file_type: FileType::Directory,
                    inode: 0,
                });
            }
        }
    }

    entries
}

pub fn open(path: &str) -> Option<VfsNode> {
    open_in_ns(zenus_ns::NS_ROOT, path)
}

pub fn open_in_ns(ns_id: zenus_ns::NsId, path: &str) -> Option<VfsNode> {
    if path == "/" || path.is_empty() {
        return root().map(|r| VfsNode { fs: r.fs, inode: r.inode });
    }

    let (fs, mount_prefix) = find_mount_in_table(ns_id, path)?;
    let root_inode = fs.root_inode();

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
    let root_inode_num = root_inode;
    let mut path_segments: [&str; 32] = [""; 32];
    let mut seg_count = 0;

    for part in trimmed.split('/') {
        if part.is_empty() || part == "." { continue; }
        if part == ".." {
            if seg_count > 0 {
                // Check if removing this segment would cross mount boundary
                if seg_count == 1 && current.inode == root_inode_num {
                    continue;
                }
                seg_count -= 1;
            }
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
    if euid == stat.uid {
        if want_write {
            if (mode & S_IWUSR) == 0 { return false; }
            if (mode & S_IXUSR) == 0 && stat.file_type == FileType::Directory { return false; }
            return true;
        } else {
            return (mode & S_IRUSR) != 0;
        }
    } else if egid == stat.gid {
        if want_write {
            if (mode & S_IWGRP) == 0 { return false; }
            if (mode & S_IXGRP) == 0 && stat.file_type == FileType::Directory { return false; }
            return true;
        } else {
            return (mode & S_IRGRP) != 0;
        }
    } else {
        if want_write {
            return (mode & S_IWOTH) != 0;
        } else {
            return (mode & S_IROTH) != 0;
        }
    }
}

pub fn perm_str(mode: u16) -> [u8; 10] {
    let mut buf = *b"----------";
    let ft = (mode >> 12) & 0xF;
    buf[0] = match ft {
        0x4 => b'd', 0x8 => b'-', 0x2 => b'c', 0x6 => b'b', 0xA => b'l', _ => b'?',
    };
    if mode & S_IRUSR != 0 { buf[1] = b'r'; }
    if mode & S_IWUSR != 0 { buf[2] = b'w'; }
    if mode & S_IXUSR != 0 { buf[3] = b'x'; }
    if mode & S_IRGRP != 0 { buf[4] = b'r'; }
    if mode & S_IWGRP != 0 { buf[5] = b'w'; }
    if mode & S_IXGRP != 0 { buf[6] = b'x'; }
    if mode & S_IROTH != 0 { buf[7] = b'r'; }
    if mode & S_IWOTH != 0 { buf[8] = b'w'; }
    if mode & S_IXOTH != 0 { buf[9] = b'x'; }
    buf
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
