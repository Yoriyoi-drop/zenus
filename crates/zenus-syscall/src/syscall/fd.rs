use zenus_fs::vfs::{self, FileSystem, FileType, DirEntry};
use zenus_sync::spinlock::SpinLock;

const MAX_FDS: usize = 256;

#[derive(Clone, Copy)]
pub struct FdEntry {
    pub task_id: u64,
    pub fs: Option<&'static dyn FileSystem>,
    pub inode: u64,
    pub offset: u64,
    pub file_type: FileType,
}

unsafe impl Send for FdEntry {}
unsafe impl Sync for FdEntry {}

pub struct FdTable {
    entries: [Option<FdEntry>; MAX_FDS],
}

unsafe impl Send for FdTable {}
unsafe impl Sync for FdTable {}

impl FdTable {
    const fn new() -> Self {
        FdTable {
            entries: [None; MAX_FDS],
        }
    }

    fn alloc(&mut self, task_id: u64, fs: &'static dyn FileSystem, inode: u64, file_type: FileType) -> Option<u64> {
        for i in 0..MAX_FDS {
            if self.entries[i].is_none() {
                self.entries[i] = Some(FdEntry {
                    task_id,
                    fs: Some(fs),
                    inode,
                    offset: 0,
                    file_type,
                });
                return Some(i as u64);
            }
        }
        None
    }

    fn close(&mut self, fd: u64) -> bool {
        if fd as usize >= MAX_FDS { return false; }
        if self.entries[fd as usize].is_none() { return false; }
        self.entries[fd as usize] = None;
        true
    }

    fn get(&self, fd: u64) -> Option<&FdEntry> {
        if fd as usize >= MAX_FDS { return None; }
        self.entries[fd as usize].as_ref()
    }

    fn get_mut(&mut self, fd: u64) -> Option<&mut FdEntry> {
        if fd as usize >= MAX_FDS { return None; }
        self.entries[fd as usize].as_mut()
    }

    fn close_all_for_task(&mut self, task_id: u64) {
        for entry in self.entries.iter_mut() {
            if let Some(e) = entry {
                if e.task_id == task_id {
                    *entry = None;
                }
            }
        }
    }

    fn dup(&mut self, task_id: u64, fd: u64) -> Option<u64> {
        let entry = self.get(fd)?;
        self.alloc(task_id, entry.fs?, entry.inode, entry.file_type)
    }
}

static FD_TABLE: SpinLock<FdTable> = SpinLock::new(FdTable::new());

pub fn fd_open(task_id: u64, path: &str) -> Option<u64> {
    let node = vfs::open(path)?;
    let stat = node.fs.stat(node.inode);
    let uid = zenus_sched::scheduler::current_uid();
    let gid = zenus_sched::scheduler::current_gid();
    let euid = zenus_sched::scheduler::current_euid();
    let egid = zenus_sched::scheduler::current_egid();
    if !vfs::access_check(uid, gid, euid, egid, &stat, false) {
        return None;
    }
    let mut table = FD_TABLE.lock();
    table.alloc(task_id, node.fs, node.inode, stat.file_type)
}

pub fn fd_close(fd: u64) -> bool {
    let mut table = FD_TABLE.lock();
    table.close(fd)
}

pub fn fd_read(fd: u64, buf: &mut [u8]) -> Option<u64> {
    let mut table = FD_TABLE.lock();
    let entry = table.get_mut(fd)?;
    let fs = entry.fs?;
    // stdin (fd 0) is special
    if fd == 0 {
        let mut s = zenus_console::serial::SerialPort::new(0x3F8);
        let mut read = 0u64;
        for b in buf.iter_mut() {
            let byte = s.read_byte_serial();
            *b = byte;
            read += 1;
            if byte == b'\n' || byte == b'\r' { break; }
        }
        return Some(read);
    }
    let result = fs.read(entry.inode, entry.offset, buf);
    if let Some(n) = result {
        entry.offset += n;
    }
    result
}

pub fn fd_write(fd: u64, buf: &[u8]) -> Option<u64> {
    let mut table = FD_TABLE.lock();
    // stdout/stderr (fd 1, 2) are special
    if fd == 1 || fd == 2 {
        let mut s = zenus_console::serial::SerialPort::new(0x3F8);
        for &b in buf {
            s.write_byte_serial(b);
        }
        return Some(buf.len() as u64);
    }
    let entry = table.get_mut(fd)?;
    let fs = entry.fs?;
    let result = fs.write(entry.inode, entry.offset, buf);
    if let Some(n) = result {
        entry.offset += n;
    }
    result
}

pub fn fd_seek(fd: u64, offset: i64, whence: u64) -> Option<u64> {
    let mut table = FD_TABLE.lock();
    let entry = table.get_mut(fd)?;
    match whence {
        0 => entry.offset = offset as u64, // SEEK_SET
        1 => entry.offset = entry.offset.wrapping_add_signed(offset), // SEEK_CUR
        2 => { // SEEK_END
            let fs = entry.fs?;
            let stat = fs.stat(entry.inode);
            entry.offset = stat.size.wrapping_add_signed(offset);
        }
        _ => return None,
    }
    Some(entry.offset)
}

pub fn fd_dup(task_id: u64, fd: u64) -> Option<u64> {
    let mut table = FD_TABLE.lock();
    table.dup(task_id, fd)
}

pub fn fd_stat(fd: u64) -> Option<vfs::FileStat> {
    let table = FD_TABLE.lock();
    let entry = table.get(fd)?;
    let fs = entry.fs?;
    Some(fs.stat(entry.inode))
}

pub fn fd_close_all_for_task(task_id: u64) {
    let mut table = FD_TABLE.lock();
    table.close_all_for_task(task_id);
}

pub fn fd_readdir(fd: u64) -> &'static [DirEntry] {
    let table = FD_TABLE.lock();
    let entry = match table.get(fd) {
        Some(e) => e,
        None => return &[],
    };
    let fs = match entry.fs {
        Some(f) => f,
        None => return &[],
    };
    if entry.file_type != FileType::Directory {
        return &[];
    }
    fs.read_dir(entry.inode)
}
