use crate::vfs::{self, DirEntry, FileStat, FileSystem, FileType};
use core::sync::atomic::{AtomicUsize, Ordering};
use zenus_sync::spinlock::{SpinLock, SpinLockGuard};

const MAX_NODES: usize = 128;
const MAX_NAME: usize = 64;
const MAX_FILE_SIZE: usize = 1024;
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

/// The node table.
///
/// This used to be a `static mut` handed out as `&'static mut`, which is
/// unsound the moment two callers hold it at once (aliased `&mut`) — and on a
/// host test run with parallel threads that is not hypothetical. A `SpinLock`
/// makes the aliasing impossible; the critical sections are a handful of
/// instructions each.
static NODES: SpinLock<[TmpNode; MAX_NODES]> = SpinLock::new([EMPTY_NODE; MAX_NODES]);

/// How many slots have ever been handed out. Slot 0 is always the root.
static NODE_COUNT: AtomicUsize = AtomicUsize::new(1);

const EMPTY_NODE: TmpNode = TmpNode {
    name: [0; MAX_NAME],
    name_len: 0,
    file_type: FileType::None,
    size: 0,
    data: [0; MAX_FILE_SIZE],
    parent: 0,
    next_sibling: 0,
    first_child: 0,
    uid: 0,
    gid: 0,
    mode: 0,
};

/// Locked node table.
fn nodes() -> SpinLockGuard<'static, [TmpNode; MAX_NODES]> {
    NODES.lock()
}

fn node_count() -> usize {
    NODE_COUNT.load(Ordering::Acquire)
}

pub struct TmpFs;

impl TmpFs {
    pub fn new() -> &'static Self {
        // Slot 0 is the root directory. Done once via `NODE_COUNT`: the old
        // lazy `INIT` flag raced with itself and, being a plain `static mut`,
        // also had to be re-checked by every caller.
        let mut nodes = nodes();
        nodes[0] = TmpNode {
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
        &TmpFs
    }

    /// Drop every node and start again from an empty root.
    ///
    /// Node slots are never recycled, so a long-lived `/tmp` (or a test that
    /// creates files in several cases) eventually hits `MAX_NODES` and every
    /// later `create` fails. Resetting is the only way to get the space back;
    /// the host test suite calls it between cases for exactly that reason.
    pub fn reset() {
        NODE_COUNT.store(1, Ordering::Release);
        let mut nodes = nodes();
        for node in nodes.iter_mut() {
            *node = EMPTY_NODE;
        }
        nodes[0] = TmpNode {
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
    }

    fn alloc_node() -> Option<usize> {
        let idx = NODE_COUNT.fetch_add(1, Ordering::AcqRel);
        if idx >= MAX_NODES {
            // Do not leak the reservation on failure.
            NODE_COUNT.fetch_sub(1, Ordering::AcqRel);
            return None;
        }
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

/// The exclusive end offset of a write, or `None` if the write does not fit.
///
/// The end used to be computed as `offset as usize + buf.len()` and compared
/// against `MAX_FILE_SIZE` afterwards. `offset` comes straight from `lseek`,
/// so `lseek(fd, -1, SEEK_SET)` followed by a 2-byte write put `end` at
/// `0xFFFF_FFFF_FFFF_FFFF + 2`, which wraps to `1`. `1 > MAX_FILE_SIZE` is
/// false, so the write proceeded to
/// `data[0xFFFF_FFFF_FFFF_FFFF..1]` and the kernel aborted on a slice index
/// that starts past the end and ends before it — one `lseek` plus one `write`
/// from any ring-3 program, on a release build where the add wraps silently.
///
/// Computing the end with `checked_add` in `usize` and comparing *before*
/// slicing fixes both halves: no wrap to a small value, and no slice whose
/// start is already out of range.
pub fn write_end(offset: u64, buf_len: usize) -> Option<usize> {
    let start = usize::try_from(offset).ok()?;
    let end = start.checked_add(buf_len)?;
    if end > MAX_FILE_SIZE {
        return None;
    }
    Some(end)
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
        if idx >= node_count() {
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
        let mut nodes = nodes();
        let idx = inode as usize;
        if idx >= node_count() {
            return None;
        }
        if nodes[idx].file_type != FileType::File {
            return None;
        }
        let end = write_end(offset, buf.len())?;
        let start = end - buf.len();
        nodes[idx].data[start..end].copy_from_slice(buf);
        if end > nodes[idx].size as usize {
            nodes[idx].size = end as u32;
        }
        Some(buf.len() as u64)
    }

    fn read_dir(&self, inode: u64) -> alloc::vec::Vec<DirEntry> {
        let mut entries = alloc::vec::Vec::with_capacity(MAX_DIR_ENTRIES);

        let nodes = nodes();
        let idx = inode as usize;
        if idx >= node_count() {
            return entries;
        }

        let mut child = nodes[idx].first_child as usize;
        while child != 0 && entries.len() < MAX_DIR_ENTRIES {
            let node = &nodes[child];
            let name = if node.name_len == 0 {
                "/"
            } else {
                let len = node.name_len as usize;
                core::str::from_utf8(&node.name[..len]).unwrap_or("/")
            };
            entries.push(DirEntry {
                name: alloc::string::String::from(name),
                file_type: node.file_type,
                inode: child as u64,
            });
            child = node.next_sibling as usize;
        }
        entries
    }

    fn stat(&self, inode: u64) -> FileStat {
        let nodes = nodes();
        let idx = inode as usize;
        if idx >= node_count() {
            return FileStat {
                size: 0,
                file_type: FileType::None,
                inode,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0,
            };
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
        let mut nodes = nodes();
        let pidx = parent_inode as usize;
        if pidx >= node_count() {
            return None;
        }
        if nodes[pidx].file_type != FileType::Directory {
            return None;
        }
        if Self::find_child(&nodes[..], pidx, name).is_some() {
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
        Self::add_child(&mut nodes[..], pidx, child_idx);
        Some(child_idx as u64)
    }

    fn unlink(&self, parent_inode: u64, name: &str) -> bool {
        let mut nodes = nodes();
        let pidx = parent_inode as usize;
        if pidx >= node_count() {
            return false;
        }
        let mut prev: usize = 0;
        let mut child = nodes[pidx].first_child as usize;
        while child != 0 {
            if name_matches(&nodes[child], name) {
                // POSIX `rmdir` semantics: a directory that still has children
                // is not empty and cannot be removed. Unlinking it anyway made
                // `rm -r /dir/b` leave `/dir` reporting success while `/dir/b`
                // became unreachable — an orphaned subtree, not a deletion.
                if nodes[child].file_type == FileType::Directory
                    && nodes[child].first_child != 0
                {
                    return false;
                }
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
        let mut nodes = nodes();
        let idx = inode as usize;
        if idx >= node_count() {
            return false;
        }
        nodes[idx].mode = (nodes[idx].mode & 0xF000) | (mode & 0x0FFF);
        true
    }

    fn chown(&self, inode: u64, uid: u32, gid: u32) -> bool {
        let mut nodes = nodes();
        let idx = inode as usize;
        if idx >= node_count() {
            return false;
        }
        nodes[idx].uid = uid;
        nodes[idx].gid = gid;
        true
    }
}

#[cfg(test)]
mod host_tests {
    use super::{write_end, TmpFs, MAX_FILE_SIZE};
    use crate::vfs::FileSystem;

    /// Regression: `TmpFs::write` computed its end offset as
    /// `offset as usize + buf.len()` and compared it *afterwards* against
    /// `MAX_FILE_SIZE`.
    ///
    /// `offset` reaches this function straight from `lseek`, so
    /// `lseek(fd, -1, SEEK_SET)` (`SEEK_SET` with a negative offset wraps to
    /// `u64::MAX`) followed by a 2-byte write made the addition wrap to `1`.
    /// `1 > MAX_FILE_SIZE` is false, so the guard passed and the code sliced
    /// `data[0xFFFF_FFFF_FFFF_FFFF..1]` — a range that starts past the end of
    /// the array and ends before it. The kernel aborted on the slice index.
    ///
    /// The end must be computed with a checked add and bounds-checked *before*
    /// any slicing happens.
    #[test]
    fn a_write_offset_that_wraps_is_refused_instead_of_panicking() {
        // The exact shape of the crash: u64::MAX + 2 wraps to 1.
        assert!(
            write_end(u64::MAX, 2).is_none(),
            "an offset whose end wraps to a small value must be refused"
        );

        // u64::MAX is what `lseek(fd, -1, SEEK_SET)` produces.
        assert!(write_end(u64::MAX, 0).is_none());
        assert!(write_end(u64::MAX, 1).is_none());

        // A start *past* the end of the buffer is refused even when the end
        // lands back in range — this is the case an unchecked add could not
        // produce, and it must not slice `data[2000..1024]`.
        assert!(write_end(MAX_FILE_SIZE as u64 + 1, 0).is_none());
        assert!(write_end(MAX_FILE_SIZE as u64 * 2, 0).is_none());

        // Exactly filling the file is allowed, and an empty write landing
        // precisely on the boundary is a valid no-op, as POSIX write() at EOF
        // is.
        assert_eq!(write_end(MAX_FILE_SIZE as u64, 0), Some(MAX_FILE_SIZE));
        assert_eq!(write_end(0, MAX_FILE_SIZE), Some(MAX_FILE_SIZE));
        assert_eq!(write_end(0, MAX_FILE_SIZE + 1), None);
        assert_eq!(write_end(10, 0), Some(10));
    }

    /// The same guard, exercised through the real `FileSystem` entry point so
    /// the test fails if someone reintroduces the unchecked add in `write`
    /// while leaving the helper correct.
    #[test]
    fn tmpfs_write_with_a_wrapping_offset_returns_none() {
        let fs = TmpFs;
        let root = fs.root_inode();

        // Create a regular file under the root.
        let file = fs.create(root, "wrap", crate::vfs::FileType::File);
        let inode = file.expect("create a file");

        let payload = [b'a'; 2];
        assert_eq!(
            fs.write(inode, u64::MAX, &payload),
            None,
            "an out-of-range offset must fail the write, not slice out of bounds"
        );

        // The file is untouched: size 0, and a read at 0 reports nothing.
        assert_eq!(fs.stat(inode).size, 0);
        let mut buf = [0u8; 8];
        assert_eq!(fs.read(inode, 0, &mut buf), Some(0));

        // An in-range write still works, so the guard is not over-broad.
        assert_eq!(fs.write(inode, 0, b"ok"), Some(2));
        assert_eq!(fs.stat(inode).size, 2);
    }
}
