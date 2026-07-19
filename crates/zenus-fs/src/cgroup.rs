use crate::vfs::{FileSystem, FileType, FileStat, DirEntry};
use alloc::vec::Vec;
use alloc::string::String;
use core::fmt::Write;
use zenus_sync::spinlock::SpinLock;

pub struct CgroupFs;

const CGROUP_ROOT_INODE: u64 = 0;
const CGROUP_DEFAULTS_INODE: u64 = 1;
const CGROUP_PROCS_INODE: u64 = 2;
const CGROUP_TASKS_INODE: u64 = 3;
const CGROUP_CONTROLLERS_INODE: u64 = 4;

struct CgroupNode {
    name: &'static str,
    inode: u64,
    file_type: FileType,
}

const ROOT_CHILDREN: &[CgroupNode] = &[
    CgroupNode { name: "cgroup.procs", inode: CGROUP_PROCS_INODE, file_type: FileType::File },
    CgroupNode { name: "cgroup.tasks", inode: CGROUP_TASKS_INODE, file_type: FileType::File },
    CgroupNode { name: "cgroup.controllers", inode: CGROUP_CONTROLLERS_INODE, file_type: FileType::File },
    CgroupNode { name: "cgroup.subtree_control", inode: 5, file_type: FileType::File },
    CgroupNode { name: "cpu.pressure", inode: 6, file_type: FileType::File },
    CgroupNode { name: "io.pressure", inode: 7, file_type: FileType::File },
    CgroupNode { name: "memory.pressure", inode: 8, file_type: FileType::File },
];

impl FileSystem for CgroupFs {
    fn name(&self) -> &'static str { "cgroup2" }
    fn root_inode(&self) -> u64 { CGROUP_ROOT_INODE }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let content = match inode {
            CGROUP_PROCS_INODE => {
                String::new()
            }
            CGROUP_TASKS_INODE => {
                String::new()
            }
            CGROUP_CONTROLLERS_INODE => {
                String::from("cpu io memory pids\n")
            }
            5 => {
                String::from("cpu io memory pids\n")
            }
            6 | 7 | 8 => String::new(),
            _ => return Some(0),
        };
        let bytes = content.as_bytes();
        let start = offset as usize;
        if start >= bytes.len() { return Some(0); }
        let len = core::cmp::min(buf.len(), bytes.len() - start);
        buf[..len].copy_from_slice(&bytes[start..start + len]);
        Some(len as u64)
    }

    fn write(&self, inode: u64, _offset: u64, buf: &[u8]) -> Option<u64> {
        match inode {
            CGROUP_PROCS_INODE | CGROUP_TASKS_INODE | 5 => {
                Some(buf.len() as u64)
            }
            _ => None,
        }
    }

    fn read_dir(&self, inode: u64) -> Vec<DirEntry> {
        if inode != CGROUP_ROOT_INODE {
            return Vec::new();
        }
        let mut entries = Vec::with_capacity(ROOT_CHILDREN.len() + 1);
        for child in ROOT_CHILDREN {
            entries.push(DirEntry {
                name: String::from(child.name),
                file_type: child.file_type,
                inode: child.inode,
            });
        }
        entries
    }

    fn stat(&self, inode: u64) -> FileStat {
        match inode {
            CGROUP_ROOT_INODE => FileStat { size: 0, file_type: FileType::Directory, inode: 0, blocks: 0, uid: 0, gid: 0, mode: 0o755 },
            CGROUP_PROCS_INODE => FileStat { size: 0, file_type: FileType::File, inode, blocks: 0, uid: 0, gid: 0, mode: 0o644 },
            CGROUP_TASKS_INODE => FileStat { size: 0, file_type: FileType::File, inode, blocks: 0, uid: 0, gid: 0, mode: 0o644 },
            CGROUP_CONTROLLERS_INODE => FileStat { size: 30, file_type: FileType::File, inode, blocks: 0, uid: 0, gid: 0, mode: 0o444 },
            5 => FileStat { size: 30, file_type: FileType::File, inode, blocks: 0, uid: 0, gid: 0, mode: 0o644 },
            6 | 7 | 8 => FileStat { size: 0, file_type: FileType::File, inode, blocks: 0, uid: 0, gid: 0, mode: 0o444 },
            _ => FileStat { size: 0, file_type: FileType::None, inode, blocks: 0, uid: 0, gid: 0, mode: 0 },
        }
    }

    fn create(&self, parent_inode: u64, _name: &str, _file_type: FileType) -> Option<u64> {
        if parent_inode == CGROUP_ROOT_INODE {
            Some(100)
        } else {
            None
        }
    }

    fn unlink(&self, _parent_inode: u64, _name: &str) -> bool {
        true
    }

    fn lookup(&self, _parent_inode: u64, name: &str) -> Option<u64> {
        for child in ROOT_CHILDREN {
            if child.name == name {
                return Some(child.inode);
            }
        }
        None
    }

    fn chmod(&self, _inode: u64, _mode: u16) -> bool { true }
    fn chown(&self, _inode: u64, _uid: u32, _gid: u32) -> bool { true }
}
