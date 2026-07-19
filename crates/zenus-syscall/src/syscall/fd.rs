use zenus_fs::vfs::{self, FileSystem, FileType, DirEntry};
use zenus_sync::spinlock::SpinLock;

const MAX_FDS: usize = 256;

pub const PIPE_BUF_SIZE: usize = 4096;
pub const MAX_PIPES: usize = 64;

pub struct PipeBuffer {
    data: [u8; PIPE_BUF_SIZE],
    read_pos: usize,
    write_pos: usize,
    count: usize,
    read_open: bool,
    write_open: bool,
}

impl PipeBuffer {
    const fn new() -> Self {
        PipeBuffer {
            data: [0; PIPE_BUF_SIZE],
            read_pos: 0,
            write_pos: 0,
            count: 0,
            read_open: false,
            write_open: false,
        }
    }

    fn write(&mut self, buf: &[u8]) -> Option<usize> {
        if !self.write_open || !self.read_open {
            return None;
        }
        let mut written = 0;
        for &byte in buf {
            if self.count >= PIPE_BUF_SIZE {
                break;
            }
            self.data[self.write_pos] = byte;
            self.write_pos = (self.write_pos + 1) % PIPE_BUF_SIZE;
            self.count += 1;
            written += 1;
        }
        Some(written)
    }

    fn read(&mut self, buf: &mut [u8]) -> Option<usize> {
        if !self.read_open {
            return None;
        }
        if self.count == 0 && !self.write_open {
            return Some(0);
        }
        if self.count == 0 {
            return None;
        }
        let mut read = 0;
        for byte in buf.iter_mut() {
            if self.count == 0 {
                break;
            }
            *byte = self.data[self.read_pos];
            self.read_pos = (self.read_pos + 1) % PIPE_BUF_SIZE;
            self.count -= 1;
            read += 1;
        }
        Some(read)
    }
}

unsafe impl Send for PipeBuffer {}
unsafe impl Sync for PipeBuffer {}

#[derive(Clone, Copy)]
pub struct FdEntry {
    pub task_id: u64,
    pub fs: Option<&'static dyn FileSystem>,
    pub inode: u64,
    pub offset: u64,
    pub file_type: FileType,
    pub pipe_id: u64,    // u64::MAX if not a pipe
    pub socket_id: u64,  // u64::MAX if not a socket
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
                    pipe_id: u64::MAX,
                    socket_id: u64::MAX,
                });
                return Some(i as u64);
            }
        }
        None
    }

    fn alloc_pipe(&mut self, task_id: u64, pipe_id: u64, for_read: bool) -> Option<u64> {
        for i in 0..MAX_FDS {
            if self.entries[i].is_none() {
                let file_type = if for_read { FileType::CharDevice } else { FileType::CharDevice };
                self.entries[i] = Some(FdEntry {
                    task_id,
                    fs: None,
                    inode: 0,
                    offset: 0,
                    file_type,
                    pipe_id,
                    socket_id: u64::MAX,
                });
                return Some(i as u64);
            }
        }
        None
    }

    fn alloc_socket(&mut self, task_id: u64, socket_id: u64) -> Option<u64> {
        for i in 0..MAX_FDS {
            if self.entries[i].is_none() {
                self.entries[i] = Some(FdEntry {
                    task_id,
                    fs: None,
                    inode: 0,
                    offset: 0,
                    file_type: FileType::CharDevice,
                    pipe_id: u64::MAX,
                    socket_id,
                });
                return Some(i as u64);
            }
        }
        None
    }

    fn close(&mut self, fd: u64) -> bool {
        if fd as usize >= MAX_FDS { return false; }
        let entry = match &self.entries[fd as usize] {
            Some(e) => e,
            None => return false,
        };
        let was_pipe = entry.pipe_id != u64::MAX;
        let was_socket = entry.socket_id != u64::MAX;
        if was_pipe {
            let pid = entry.pipe_id;
            let mut pipes = PIPE_TABLE.lock();
            if let Some(ref mut pipe) = pipes[pid as usize] {
                if fd % 2 == 0 {
                    pipe.read_open = false;
                } else {
                    pipe.write_open = false;
                }
            }
            drop(pipes);
        }
        if was_socket {
            let _sock_id = entry.socket_id;
            // Socket cleanup handled by syscall close handler
        }
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
        if entry.pipe_id != u64::MAX {
            self.alloc_pipe(task_id, entry.pipe_id, fd % 2 == 0)
        } else if entry.socket_id != u64::MAX {
            self.alloc_socket(task_id, entry.socket_id)
        } else {
            self.alloc(task_id, entry.fs?, entry.inode, entry.file_type)
        }
    }

    fn clone_all_for_task(&mut self, src_task_id: u64, dst_task_id: u64) {
        let fds: alloc::vec::Vec<(u64, FdEntry)> = self.entries.iter()
            .enumerate()
            .filter_map(|(fd, e)| {
                e.as_ref().filter(|e| e.task_id == src_task_id).map(|e| (fd as u64, *e))
            })
            .collect();
        for (fd, entry) in fds {
            if entry.pipe_id != u64::MAX {
                self.alloc_pipe(dst_task_id, entry.pipe_id, fd % 2 == 0);
            } else if entry.socket_id != u64::MAX {
                self.alloc_socket(dst_task_id, entry.socket_id);
            } else if let Some(fs) = entry.fs {
                self.alloc(dst_task_id, fs, entry.inode, entry.file_type);
            }
        }
    }
}

static FD_TABLE: SpinLock<FdTable> = SpinLock::new(FdTable::new());

static PIPE_TABLE: SpinLock<[Option<PipeBuffer>; MAX_PIPES]> =
    SpinLock::new([
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
        None, None, None, None, None, None, None, None,
    ]);

pub fn fd_pipe(task_id: u64) -> Option<(u64, u64)> {
    let mut pipes = PIPE_TABLE.lock();
    let mut pipe_id = None;
    for i in 0..MAX_PIPES {
        if pipes[i].is_none() {
            pipes[i] = Some(PipeBuffer::new());
            pipe_id = Some(i);
            break;
        }
    }
    let pipe_id = pipe_id?;
    let pipe = pipes[pipe_id].as_mut()?;
    pipe.read_open = true;
    pipe.write_open = true;
    drop(pipes);

    let mut table = FD_TABLE.lock();
    let read_fd = table.alloc_pipe(task_id, pipe_id as u64, true);
    let write_fd = table.alloc_pipe(task_id, pipe_id as u64, false);
    match (read_fd, write_fd) {
        (Some(r), Some(w)) => Some((r, w)),
        _ => {
            // Cleanup on failure
            let mut pipes = PIPE_TABLE.lock();
            pipes[pipe_id] = None;
            drop(pipes);
            None
        }
    }
}

pub fn fd_socket(task_id: u64, socket_id: u64) -> Option<u64> {
    let mut table = FD_TABLE.lock();
    table.alloc_socket(task_id, socket_id)
}

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
    // stdin (fd 0) is special — handle BEFORE table lookup
    // so user tasks without explicit FD entries can still read serial input.
    if fd == 0 {
        let s = zenus_console::serial::SerialPort::new(0x3F8);
        let mut read = 0u64;
        for b in buf.iter_mut() {
            let byte = if s.is_data_available() {
                s.read_byte_serial()
            } else if zenus_arch::keyboard::is_key_available() {
                zenus_arch::keyboard::read_key().unwrap_or(0)
            } else {
                s.read_byte_serial()
            };
            *b = byte;
            read += 1;
            if byte == b'\n' || byte == b'\r' { break; }
        }
        return Some(read);
    }

    let mut table = FD_TABLE.lock();
    let entry = match table.get_mut(fd) {
        Some(e) => e,
        None => return None,
    };

    // Pipe read
    if entry.pipe_id != u64::MAX {
        let mut pipes = PIPE_TABLE.lock();
        let pipe = match pipes[entry.pipe_id as usize].as_mut() {
            Some(p) => p,
            None => return None,
        };
        return pipe.read(buf).map(|n| n as u64);
    }

    let fs = entry.fs?;
    let result = fs.read(entry.inode, entry.offset, buf);
    if let Some(n) = result {
        entry.offset += n;
    }
    result
}

pub fn fd_write(fd: u64, buf: &[u8]) -> Option<u64> {
    // DEBUG: write 'W' to Bochs port when fd_write is entered
    unsafe { core::arch::asm!("out 0xe9, al", in("al") b'\x57'); }
    // stdout/stderr (fd 1, 2) are special — handle BEFORE table lookup
    // so user tasks without explicit FD entries can still write.
    if fd == 1 || fd == 2 {
        let s = zenus_console::serial::SerialPort::new(0x3F8);
        for &b in buf {
            s.write_byte_serial(b);
        }
        if let Ok(s) = core::str::from_utf8(buf) {
            zenus_console::display::write_str(s);
        }
        return Some(buf.len() as u64);
    }

    let mut table = FD_TABLE.lock();
    let entry = match table.get_mut(fd) {
        Some(e) => e,
        None => return None,
    };

    // Pipe write
    if entry.pipe_id != u64::MAX {
        let mut pipes = PIPE_TABLE.lock();
        let pipe = match pipes[entry.pipe_id as usize].as_mut() {
            Some(p) => p,
            None => return None,
        };
        return pipe.write(buf).map(|n| n as u64);
    }

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
    if entry.pipe_id != u64::MAX {
        return None; // Pipes don't support seek
    }
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
    if entry.pipe_id != u64::MAX {
        return Some(vfs::FileStat {
            size: 0,
            file_type: FileType::CharDevice,
            inode: 0,
            blocks: 0,
            uid: 0,
            gid: 0,
            mode: 0x21A4,
        });
    }
    let fs = entry.fs?;
    Some(fs.stat(entry.inode))
}

pub fn fd_close_all_for_task(task_id: u64) {
    let mut table = FD_TABLE.lock();
    table.close_all_for_task(task_id);
}

pub fn fd_clone_all_for_task(src_task_id: u64, dst_task_id: u64) {
    let mut table = FD_TABLE.lock();
    table.clone_all_for_task(src_task_id, dst_task_id);
}

pub fn fd_readdir(fd: u64) -> alloc::vec::Vec<DirEntry> {
    let table = FD_TABLE.lock();
    let entry = match table.get(fd) {
        Some(e) => e,
        None => return alloc::vec::Vec::new(),
    };
    let fs = match entry.fs {
        Some(f) => f,
        None => return alloc::vec::Vec::new(),
    };
    if entry.file_type != FileType::Directory {
        return alloc::vec::Vec::new();
    }
    fs.read_dir(entry.inode)
}

pub fn fd_get(fd: u64) -> Option<FdEntry> {
    let table = FD_TABLE.lock();
    table.get(fd).copied()
}

pub fn fd_dup2(task_id: u64, oldfd: u64, newfd: u64) -> Option<u64> {
    let mut table = FD_TABLE.lock();
    let entry = table.get(oldfd)?;
    let entry_copy = *entry;
    drop(table);

    let mut table = FD_TABLE.lock();
    if (newfd as usize) < 256 {
        table.entries[newfd as usize] = Some(FdEntry {
            task_id,
            fs: entry_copy.fs,
            inode: entry_copy.inode,
            offset: entry_copy.offset,
            file_type: entry_copy.file_type,
            pipe_id: entry_copy.pipe_id,
            socket_id: entry_copy.socket_id,
        });
        Some(newfd)
    } else {
        None
    }
}

pub fn current_task() -> u64 {
    zenus_sched::scheduler::current_task_id()
}

pub fn vfs_mkdir(path: &str) -> bool {
    vfs::create_dir(path)
}

pub fn vfs_unlink(path: &str) -> bool {
    vfs::remove(path)
}

pub fn vfs_rmdir(path: &str) -> bool {
    let node = match vfs::open(path) {
        Some(n) => n,
        None => return false,
    };
    let entries = node.fs.read_dir(node.inode);
    if !entries.is_empty() { return false; }
    vfs::remove(path)
}

pub fn vfs_rename(_old: &str, _new: &str) -> bool {
    false
}

pub fn vfs_chmod(path: &str, mode: u16) -> bool {
    let node = match vfs::open(path) {
        Some(n) => n,
        None => return false,
    };
    node.fs.chmod(node.inode, mode)
}

pub fn vfs_chown(path: &str, _owner: u32, _group: u32) -> bool {
    let _node = match vfs::open(path) {
        Some(n) => n,
        None => return false,
    };
    true
}

pub fn vfs_access(path: &str) -> bool {
    vfs::open(path).is_some()
}
