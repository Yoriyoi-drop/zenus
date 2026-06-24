use crate::{NsId, alloc_ns_id};

/// Create a new mount namespace. The VFS layer handles the actual
/// mount table copy (see `vfs::create_mnt_ns`).
pub fn create() -> Option<NsId> {
    let id = alloc_ns_id();
    if crate::NS_ROOT == id {
        return None;
    }
    // VFS stores the per-namespace mount table.
    // The caller must call vfs::create_mnt_ns() to copy the parent's table.
    Some(id)
}
