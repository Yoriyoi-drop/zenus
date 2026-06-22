use core::mem::MaybeUninit;
use crate::vfs::{self, FileSystem, FileType, FileStat, DirEntry};
use zenus_sync::spinlock::SpinLock;

const MAX_NODES: usize = 128;
const MAX_NAME: usize = 64;
const MAX_FILE_SIZE: usize = 4096;
const MAX_DIR_ENTRIES: usize = 256;

#[derive(Clone, Copy)]
struct TmpNode {
    name: [u8; MAX_NAME],
    name_len: u8,
    file_type: FileType,
    size: u32,
    data: [u8; MAX_FILE_SIZE],
    parent: u16,
    next_sibling: u16,
    first_child: u16,
    uid: u32,
    gid: u32,
    mode: u16,
}

fn nodes() -> &'static mut [TmpNode; MAX_NODES] {
    static mut NODES: MaybeUninit<[TmpNode; MAX_NODES]> = MaybeUninit::uninit();
    static mut INIT: bool = false;
    unsafe {
        if !INIT {
            core::ptr::write_bytes(NODES.as_mut_ptr(), 0, 1);
            (*NODES.as_mut_ptr())[0] = TmpNode {
                name: [0; MAX_NAME],
                name_len: 0,
                file_type: FileType::Directory,
                size: 0,
                data: [0; MAX_FILE_SIZE],
                parent: 0,
                next_sibling: 0,
                first_child: 0,
                uid: 0,
                gid: 0,
                mode: vfs::DEFAULT_DIR_MODE,
            };
            INIT = true;
        }
        &mut *NODES.as_mut_ptr()
    }
}

fn node_count() -> &'static mut usize {
    static mut COUNT: usize = 1;
    unsafe { &mut COUNT }
}

pub struct TmpFs;

impl TmpFs {
    pub fn new() -> &'static Self {
        let _ = nodes();
        &TmpFs
    }

    fn alloc_node() -> Option<usize> {
        let count = node_count();
        let idx = *count;
        if idx >= MAX_NODES {
            return None;
        }
        *count = idx + 1;
        Some(idx)
    }

    fn set_name(node: &mut TmpNode, name: &str) {
        let bytes = name.as_bytes();
        let len = bytes.len().min(MAX_NAME - 1) as u8;
        node.name[..len as usize].copy_from_slice(&bytes[..len as usize]);
        node.name_len = len;
    }

    fn find_child(nodes: &[TmpNode], parent_idx: usize, name: &str) -> Option<usize> {
        let mut child = nodes[parent_idx].first_child as usize;
        while child != 0 {
            if name_matches(&nodes[child], name) {
                return Some(child);
            }
            child = nodes[child].next_sibling as usize;
        }
        None
    }

    fn add_child(nodes: &mut [TmpNode], parent_idx: usize, child_idx: usize) {
        nodes[child_idx].parent = parent_idx as u16;
        let first = nodes[parent_idx].first_child;
        if first == 0 {
            nodes[parent_idx].first_child = child_idx as u16;
        } else {
            let mut last = first as usize;
            while nodes[last].next_sibling != 0 {
                last = nodes[last].next_sibling as usize;
            }
            nodes[last].next_sibling = child_idx as u16;
        }
    }
}

fn name_matches(node: &TmpNode, name: &str) -> bool {
    let len = node.name_len as usize;
    let name_bytes = name.as_bytes();
    if len != name_bytes.len() {
        return false;
    }
    &node.name[..len] == name_bytes
}

impl FileSystem for TmpFs {
    fn name(&self) -> &'static str {
        "tmpfs"
    }

    fn root_inode(&self) -> u64 {
        0
    }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let nodes = nodes();
        let idx = inode as usize;
        if idx >= *node_count() {
            return None;
        }
        let node = &nodes[idx];
        if node.file_type != FileType::File {
            return Some(0);
        }
        if offset >= node.size as u64 {
            return Some(0);
        }
        let read_len = core::cmp::min(buf.len() as u64, node.size as u64 - offset) as usize;
        buf[..read_len].copy_from_slice(&node.data[offset as usize..offset as usize + read_len]);
        Some(read_len as u64)
    }

    fn write(&self, inode: u64, offset: u64, buf: &[u8]) -> Option<u64> {
        let nodes = nodes();
        let idx = inode as usize;
        if idx >= *node_count() {
            return None;
        }
        if nodes[idx].file_type != FileType::File {
            return None;
        }
        let end = offset as usize + buf.len();
        if end > MAX_FILE_SIZE {
            return None;
        }
        nodes[idx].data[offset as usize..end].copy_from_slice(buf);
        if end > nodes[idx].size as usize {
            nodes[idx].size = end as u32;
        }
        Some(buf.len() as u64)
    }

    fn read_dir(&self, inode: u64) -> &'static [DirEntry] {
        static TMPFS_DIR_LOCK: SpinLock<()> = SpinLock::new(());
        let _rd_guard = TMPFS_DIR_LOCK.lock();
        static mut ENTRIES: [DirEntry; MAX_DIR_ENTRIES] = [DirEntry {
            name: "", file_type: FileType::None, inode: 0,
        }; MAX_DIR_ENTRIES];
        static mut COUNT: usize = 0;

        let nodes = nodes();
        let idx = inode as usize;
        if idx >= *node_count() {
            return &[];
        }

        unsafe {
            COUNT = 0;
            let mut child = nodes[idx].first_child as usize;
            while child != 0 && COUNT < MAX_DIR_ENTRIES {
                let node = &nodes[child];
                let name = if node.name_len == 0 {
                    "/"
                } else {
                    let len = node.name_len as usize;
                    core::str::from_utf8_unchecked(&node.name[..len])
                };
                ENTRIES[COUNT] = DirEntry {
                    name,
                    file_type: node.file_type,
                    inode: child as u64,
                };
                COUNT += 1;
                child = node.next_sibling as usize;
            }
            &ENTRIES[..COUNT]
        }
    }

    fn stat(&self, inode: u64) -> FileStat {
        let nodes = nodes();
        let idx = inode as usize;
        if idx >= *node_count() {
            return FileStat { size: 0, file_type: FileType::None, inode, blocks: 0, uid: 0, gid: 0, mode: 0 };
        }
        let node = &nodes[idx];
        FileStat {
            size: node.size as u64,
            file_type: node.file_type,
            inode: idx as u64,
            blocks: (node.size as u64 + 511) / 512,
            uid: node.uid,
            gid: node.gid,
            mode: node.mode,
        }
    }

    fn create(&self, parent_inode: u64, name: &str, file_type: FileType) -> Option<u64> {
        let nodes = nodes();
        let pidx = parent_inode as usize;
        if pidx >= *node_count() {
            return None;
        }
        if nodes[pidx].file_type != FileType::Directory {
            return None;
        }
        if Self::find_child(nodes, pidx, name).is_some() {
            return None;
        }
        let child_idx = Self::alloc_node()?;
        Self::set_name(&mut nodes[child_idx], name);
        nodes[child_idx].file_type = file_type;
        nodes[child_idx].uid = 0;
        nodes[child_idx].gid = 0;
        nodes[child_idx].mode = match file_type {
            FileType::Directory => vfs::DEFAULT_DIR_MODE,
            _ => vfs::DEFAULT_FILE_MODE,
        };
        Self::add_child(nodes, pidx, child_idx);
        Some(child_idx as u64)
    }

    fn unlink(&self, parent_inode: u64, name: &str) -> bool {
        let nodes = nodes();
        let pidx = parent_inode as usize;
        if pidx >= *node_count() {
            return false;
        }
        let mut prev: usize = 0;
        let mut child = nodes[pidx].first_child as usize;
        while child != 0 {
            if name_matches(&nodes[child], name) {
                if prev == 0 {
                    nodes[pidx].first_child = nodes[child].next_sibling;
                } else {
                    nodes[prev].next_sibling = nodes[child].next_sibling;
                }
                nodes[child].file_type = FileType::None;
                return true;
            }
            prev = child;
            child = nodes[child].next_sibling as usize;
        }
        false
    }

    fn chmod(&self, inode: u64, mode: u16) -> bool {
        let nodes = nodes();
        let idx = inode as usize;
        if idx >= *node_count() { return false; }
        nodes[idx].mode = (nodes[idx].mode & 0xF000) | (mode & 0x0FFF);
        true
    }

    fn chown(&self, inode: u64, uid: u32, gid: u32) -> bool {
        let nodes = nodes();
        let idx = inode as usize;
        if idx >= *node_count() { return false; }
        nodes[idx].uid = uid;
        nodes[idx].gid = gid;
        true
    }
}


