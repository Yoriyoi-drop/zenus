use crate::vfs::{FileSystem, FileType, FileStat, DirEntry};
use alloc::vec::Vec;
use alloc::string::String;
use core::fmt::Write;
use zenus_sync::spinlock::SpinLock;

pub struct ProcFs;

const ENTRIES: &[ProcEntry] = &[
    ProcEntry { inode: 1, name: "cpuinfo", file_type: FileType::File },
    ProcEntry { inode: 2, name: "meminfo", file_type: FileType::File },
    ProcEntry { inode: 3, name: "uptime", file_type: FileType::File },
    ProcEntry { inode: 4, name: "version", file_type: FileType::File },
    ProcEntry { inode: 5, name: "stat", file_type: FileType::File },
    ProcEntry { inode: 6, name: "loadavg", file_type: FileType::File },
    ProcEntry { inode: 7, name: "self", file_type: FileType::Symlink },
    ProcEntry { inode: 8, name: "modules", file_type: FileType::File },
];

struct ProcEntry {
    inode: u64,
    name: &'static str,
    file_type: FileType,
}

type DataFn = fn() -> String;
type CountFn = fn() -> u64;

static CPUINFO_FN: SpinLock<Option<DataFn>> = SpinLock::new(None);
static MEMINFO_FN: SpinLock<Option<DataFn>> = SpinLock::new(None);
static UPTIME_FN: SpinLock<Option<DataFn>> = SpinLock::new(None);
static STAT_FN: SpinLock<Option<DataFn>> = SpinLock::new(None);
static LOADAVG_FN: SpinLock<Option<DataFn>> = SpinLock::new(None);
static TASK_COUNT_FN: SpinLock<Option<CountFn>> = SpinLock::new(None);

pub fn register_cpuinfo(f: DataFn) { *CPUINFO_FN.lock() = Some(f); }
pub fn register_meminfo(f: DataFn) { *MEMINFO_FN.lock() = Some(f); }
pub fn register_uptime(f: DataFn) { *UPTIME_FN.lock() = Some(f); }
pub fn register_stat(f: DataFn) { *STAT_FN.lock() = Some(f); }
pub fn register_loadavg(f: DataFn) { *LOADAVG_FN.lock() = Some(f); }
pub fn register_task_count(f: CountFn) { *TASK_COUNT_FN.lock() = Some(f); }

fn call_data(f: &SpinLock<Option<DataFn>>, fallback: &str) -> String {
    let lock = f.lock();
    lock.as_ref().map(|f| f()).unwrap_or_else(|| String::from(fallback))
}

fn call_count() -> u64 {
    let lock = TASK_COUNT_FN.lock();
    lock.as_ref().map(|f| f()).unwrap_or(0)
}

fn find_entry(inode: u64) -> Option<&'static ProcEntry> {
    ENTRIES.iter().find(|e| e.inode == inode)
}

impl FileSystem for ProcFs {
    fn name(&self) -> &'static str { "proc" }
    fn root_inode(&self) -> u64 { 0 }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let entry = find_entry(inode)?;
        if entry.file_type != FileType::File { return Some(0); }
        let content = match inode {
            1 => call_data(&CPUINFO_FN, "processor\t: 0\n"),
            2 => call_data(&MEMINFO_FN, "MemTotal: 0 kB\n"),
            3 => call_data(&UPTIME_FN, "0 0\n"),
            4 => String::from("Zenus OS version 0.1.0\n"),
            5 => call_data(&STAT_FN, "cpu 0 0 0 0\n"),
            6 => call_data(&LOADAVG_FN, "0.00 0.00 0.00 1/1 0\n"),
            8 => String::from("kernel 0x0 0x0\n"),
            _ => return Some(0),
        };
        let bytes = content.as_bytes();
        let start = offset as usize;
        if start >= bytes.len() { return Some(0); }
        let len = core::cmp::min(buf.len(), bytes.len() - start);
        buf[..len].copy_from_slice(&bytes[start..start + len]);
        Some(len as u64)
    }

    fn write(&self, _inode: u64, _offset: u64, _buf: &[u8]) -> Option<u64> { None }

    fn read_dir(&self, inode: u64) -> Vec<DirEntry> {
        if inode != 0 { return Vec::new(); }
        let mut entries = Vec::with_capacity(ENTRIES.len() + 4);
        for e in ENTRIES {
            entries.push(DirEntry {
                name: String::from(e.name),
                file_type: e.file_type,
                inode: e.inode,
            });
        }
        let count = call_count();
        for pid in 1..=count {
            entries.push(DirEntry {
                name: alloc::format!("{}", pid),
                file_type: FileType::Directory,
                inode: 1000 + pid,
            });
        }
        entries
    }

    fn stat(&self, inode: u64) -> FileStat {
        if inode == 0 {
            return FileStat { size: 0, file_type: FileType::Directory, inode: 0, blocks: 0, uid: 0, gid: 0, mode: 0o555 };
        }
        if inode >= 1000 {
            return FileStat { size: 0, file_type: FileType::Directory, inode, blocks: 0, uid: 0, gid: 0, mode: 0o555 };
        }
        if let Some(entry) = find_entry(inode) {
            if entry.file_type == FileType::Symlink {
                return FileStat { size: 0, file_type: FileType::Symlink, inode, blocks: 0, uid: 0, gid: 0, mode: 0o777 };
            }
            return FileStat { size: 0, file_type: entry.file_type, inode, blocks: 0, uid: 0, gid: 0, mode: 0o444 };
        }
        FileStat { size: 0, file_type: FileType::None, inode, blocks: 0, uid: 0, gid: 0, mode: 0 }
    }

    fn create(&self, _parent_inode: u64, _name: &str, _file_type: FileType) -> Option<u64> { None }
    fn unlink(&self, _parent_inode: u64, _name: &str) -> bool { false }

    fn lookup(&self, _parent_inode: u64, name: &str) -> Option<u64> {
        for e in ENTRIES {
            if e.name == name { return Some(e.inode); }
        }
        if let Ok(pid) = name.parse::<u64>() {
            if pid > 0 && pid <= call_count() { return Some(1000 + pid); }
        }
        None
    }
}
