use crate::vfs::{DirEntry, FileStat, FileSystem, FileType};
use alloc::string::String;
use alloc::vec::Vec;

pub struct CgroupFs;

const CGROUP_ROOT_INODE: u64 = 0;
const CGROUP_DEFAULTS_INODE: u64 = 1;
const CGROUP_PROCS_INODE: u64 = 2;
const CGROUP_TASKS_INODE: u64 = 3;
const CGROUP_CONTROLLERS_INODE: u64 = 4;
const CGROUP_SUBTREE_CONTROL_INODE: u64 = 5;
const CGROUP_CPU_PRESSURE_INODE: u64 = 6;
const CGROUP_IO_PRESSURE_INODE: u64 = 7;
const CGROUP_MEMORY_PRESSURE_INODE: u64 = 8;

struct CgroupNode {
    name: &'static str,
    inode: u64,
    file_type: FileType,
}

const ROOT_CHILDREN: &[CgroupNode] = &[
    CgroupNode {
        name: "cgroup.procs",
        inode: CGROUP_PROCS_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "cgroup.tasks",
        inode: CGROUP_TASKS_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "cgroup.controllers",
        inode: CGROUP_CONTROLLERS_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "cgroup.defaults",
        inode: CGROUP_DEFAULTS_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "cgroup.subtree_control",
        inode: CGROUP_SUBTREE_CONTROL_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "cpu.pressure",
        inode: CGROUP_CPU_PRESSURE_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "io.pressure",
        inode: CGROUP_IO_PRESSURE_INODE,
        file_type: FileType::File,
    },
    CgroupNode {
        name: "memory.pressure",
        inode: CGROUP_MEMORY_PRESSURE_INODE,
        file_type: FileType::File,
    },
];

impl FileSystem for CgroupFs {
    fn name(&self) -> &'static str {
        "cgroup2"
    }
    fn root_inode(&self) -> u64 {
        CGROUP_ROOT_INODE
    }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let content = match inode {
            CGROUP_PROCS_INODE => String::new(),
            CGROUP_TASKS_INODE => String::new(),
            CGROUP_CONTROLLERS_INODE | CGROUP_SUBTREE_CONTROL_INODE => {
                String::from("cpu io memory pids\n")
            }
            // Per-subsystem default limits: no controller is enforcing anything,
            // so every file is empty.
            CGROUP_DEFAULTS_INODE
            | CGROUP_CPU_PRESSURE_INODE
            | CGROUP_IO_PRESSURE_INODE
            | CGROUP_MEMORY_PRESSURE_INODE => String::new(),
            _ => return Some(0),
        };
        let bytes = content.as_bytes();
        let start = offset as usize;
        if start >= bytes.len() {
            return Some(0);
        }
        let len = core::cmp::min(buf.len(), bytes.len() - start);
        buf[..len].copy_from_slice(&bytes[start..start + len]);
        Some(len as u64)
    }

    fn write(&self, _inode: u64, _offset: u64, _buf: &[u8]) -> Option<u64> {
        // Nothing here is enforced yet: this filesystem is a read-only view of
        // the cgroup v2 layout. Returning the byte count used to make
        // `echo +cpu > /sys/fs/cgroup/cgroup.subtree_control` look like it
        // worked while the write went nowhere — callers must see a real error.
        None
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
            CGROUP_ROOT_INODE => FileStat {
                size: 0,
                file_type: FileType::Directory,
                inode: 0,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o755,
            },
            CGROUP_PROCS_INODE => FileStat {
                size: 0,
                file_type: FileType::File,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o644,
            },
            CGROUP_TASKS_INODE => FileStat {
                size: 0,
                file_type: FileType::File,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o644,
            },
            CGROUP_CONTROLLERS_INODE => FileStat {
                size: 30,
                file_type: FileType::File,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o444,
            },
            5 => FileStat {
                size: 30,
                file_type: FileType::File,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o644,
            },
            6 | 7 | 8 => FileStat {
                size: 0,
                file_type: FileType::File,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o444,
            },
            // Anything else in the tree (cgroup.defaults and future nodes) is a
            // plain read-only file. It used to stat as `FileType::None` with
            // mode 0 while `read_dir` advertised it as a file, so `ls -l` and
            // the permission check disagreed with the directory listing.
            _ => FileStat {
                size: 0,
                file_type: FileType::File,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o444,
            },
        }
    }

    fn create(&self, _parent_inode: u64, _name: &str, _file_type: FileType) -> Option<u64> {
        // No cgroup can actually be created: there are no controllers behind
        // this tree. Inventing an inode here made `mkdir /sys/fs/cgroup/foo`
        // report success and then find nothing.
        None
    }

    fn unlink(&self, _parent_inode: u64, _name: &str) -> bool {
        false
    }

    fn lookup(&self, _parent_inode: u64, name: &str) -> Option<u64> {
        for child in ROOT_CHILDREN {
            if child.name == name {
                return Some(child.inode);
            }
        }
        None
    }

    fn chmod(&self, _inode: u64, _mode: u16) -> bool {
        // Read-only view: report the refusal instead of pretending it worked.
        false
    }
    fn chown(&self, _inode: u64, _uid: u32, _gid: u32) -> bool {
        false
    }
}
