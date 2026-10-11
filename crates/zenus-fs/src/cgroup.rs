//! cgroup v2 — the controller table and the `/sys/fs/cgroup` view of it.
//!
//! This used to be a read-only mock: `mkdir`, `unlink` and every `write`
//! returned failure, and the only honest thing about it was that it said so.
//! It is now a real `pids` and `memory` controller with a filesystem on top,
//! backed by [`CgroupTable`].
//!
//! ## What is enforced, and where
//!
//! The table stores the limits and the membership; the *enforcement points* are
//! the callers, and they are deliberately few:
//!
//! * `pids.max` — `zenus_sched` calls [`attach_task`] when it creates a task
//!   (fork, clone, exec) and refuses the creation when it returns `false`.
//! * `memory.max` — `zenus_syscall` charges the pages it maps through
//!   [`charge_memory`] and refuses the mapping when it returns `false`.
//!
//! Both are per *task id*, which is the scheduler's identifier, not the pid
//! number a program sees. Nothing else in the tree has to know cgroups exist.
//!
//! ## Hierarchy
//!
//! Flat, for now: every cgroup is a child of the root and there are no nested
//! groups. `cgroup.controllers` still advertises `cpu` and `io`, and
//! `cgroup.subtree_control` is still present for layout compatibility, but
//! neither is enforced and `subtree_control` refuses writes rather than
//! pretending an enablement it does not model.
//!
//! ## The root group
//!
//! The root is cgroup `0`. It is not one of the entries in `groups`: the root is
//! unremovable and unlimited *by construction*, so its members live in their own
//! list and its limits have no field to write. Every task is a member of its
//! parent's cgroup, and a task with no parent is a member of the root, so
//! `pids.current` at the root is the live task count and not a placeholder.
//!
//! Keeping the root out of `groups` also means the table needs no lazy
//! initialisation: `CgroupTable::empty()` is a `const fn` that already *is* a
//! valid table, and the first `attach` from the scheduler succeeds. An earlier
//! version stored the root as the first vector element and only added it in
//! `reset()`, so the global table began life rootless and every task creation
//! failed — the shell never started, which is exactly what the boot smoke test
//! caught.

use crate::vfs::{DirEntry, FileStat, FileSystem, FileType};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use zenus_sync::spinlock::SpinLock;

// ── the root group's file inodes ─────────────────────────────────────────
//
// The numbers are the layout the old read-only view shipped, so anything that
// hard-coded one still lands on the same file.
const CGROUP_ROOT_INODE: u64 = 0;
const CGROUP_DEFAULTS_INODE: u64 = 1;
const CGROUP_PROCS_INODE: u64 = 2;
const CGROUP_TASKS_INODE: u64 = 3;
const CGROUP_CONTROLLERS_INODE: u64 = 4;
const CGROUP_SUBTREE_CONTROL_INODE: u64 = 5;
const CGROUP_CPU_PRESSURE_INODE: u64 = 6;
const CGROUP_IO_PRESSURE_INODE: u64 = 7;
const CGROUP_MEMORY_PRESSURE_INODE: u64 = 8;
const CGROUP_PIDS_MAX_INODE: u64 = 9;
const CGROUP_PIDS_CURRENT_INODE: u64 = 10;
const CGROUP_MEMORY_MAX_INODE: u64 = 11;
const CGROUP_MEMORY_CURRENT_INODE: u64 = 12;

// ── derived inodes for created cgroups ───────────────────────────────────
//
// A created cgroup `id` gets the directory inode `DIR_BASE + id * DIR_STRIDE`
// and its files are the seven slots after it. `id` starts at 1, so no derived
// inode collides with the root's fixed numbers above.
const DIR_BASE: u64 = 0x1000;
const DIR_STRIDE: u64 = 0x40;

const F_PROCS: u64 = 1;
const F_CONTROLLERS: u64 = 2;
const F_SUBTREE: u64 = 3;
const F_PIDS_MAX: u64 = 4;
const F_PIDS_CURRENT: u64 = 5;
const F_MEM_MAX: u64 = 6;
const F_MEM_CURRENT: u64 = 7;
const PER_CGROUP_FILES: u64 = 7;

fn dir_inode(id: u64) -> u64 {
    DIR_BASE + id * DIR_STRIDE
}

fn file_inode(id: u64, idx: u64) -> u64 {
    dir_inode(id) + idx
}

/// Split a derived *file* inode back into `(cgroup id, file index)`.
fn decode_inode(inode: u64) -> Option<(u64, u64)> {
    if inode < DIR_BASE {
        return None;
    }
    let idx = inode % DIR_STRIDE;
    if idx < F_PROCS || idx > PER_CGROUP_FILES {
        return None;
    }
    Some(((inode - DIR_BASE) / DIR_STRIDE, idx))
}

/// The cgroup id a derived *directory* inode names, if it is one.
///
/// A directory inode is `dir_inode(id)`, whose offset within the stride is `0`,
/// which `decode_inode` rejects — the two must not share a decoder.
fn decode_dir_inode(inode: u64) -> Option<u64> {
    if inode < DIR_BASE || inode % DIR_STRIDE != 0 {
        return None;
    }
    Some((inode - DIR_BASE) / DIR_STRIDE)
}

/// The `(cgroup id, file index)` a *root* inode names.
fn root_spec(inode: u64) -> Option<(u64, u64)> {
    Some((
        CGROUP_ROOT,
        match inode {
            CGROUP_PROCS_INODE | CGROUP_TASKS_INODE => F_PROCS,
            CGROUP_CONTROLLERS_INODE => F_CONTROLLERS,
            CGROUP_SUBTREE_CONTROL_INODE => F_SUBTREE,
            CGROUP_PIDS_MAX_INODE => F_PIDS_MAX,
            CGROUP_PIDS_CURRENT_INODE => F_PIDS_CURRENT,
            CGROUP_MEMORY_MAX_INODE => F_MEM_MAX,
            CGROUP_MEMORY_CURRENT_INODE => F_MEM_CURRENT,
            _ => return None,
        },
    ))
}

/// The files every cgroup directory contains, in listing order.
const CGROUP_FILES: &[(&str, u64, FileType)] = &[
    ("cgroup.procs", F_PROCS, FileType::File),
    ("cgroup.controllers", F_CONTROLLERS, FileType::File),
    ("cgroup.subtree_control", F_SUBTREE, FileType::File),
    ("pids.max", F_PIDS_MAX, FileType::File),
    ("pids.current", F_PIDS_CURRENT, FileType::File),
    ("memory.max", F_MEM_MAX, FileType::File),
    ("memory.current", F_MEM_CURRENT, FileType::File),
];

/// The root directory's fixed children, in listing order.
const ROOT_FILES: &[(&str, u64, FileType)] = &[
    ("cgroup.procs", CGROUP_PROCS_INODE, FileType::File),
    ("cgroup.tasks", CGROUP_TASKS_INODE, FileType::File),
    ("cgroup.controllers", CGROUP_CONTROLLERS_INODE, FileType::File),
    ("cgroup.defaults", CGROUP_DEFAULTS_INODE, FileType::File),
    (
        "cgroup.subtree_control",
        CGROUP_SUBTREE_CONTROL_INODE,
        FileType::File,
    ),
    ("cpu.pressure", CGROUP_CPU_PRESSURE_INODE, FileType::File),
    ("io.pressure", CGROUP_IO_PRESSURE_INODE, FileType::File),
    (
        "memory.pressure",
        CGROUP_MEMORY_PRESSURE_INODE,
        FileType::File,
    ),
    ("pids.max", CGROUP_PIDS_MAX_INODE, FileType::File),
    ("pids.current", CGROUP_PIDS_CURRENT_INODE, FileType::File),
    ("memory.max", CGROUP_MEMORY_MAX_INODE, FileType::File),
    ("memory.current", CGROUP_MEMORY_CURRENT_INODE, FileType::File),
];

// ── the controller table ─────────────────────────────────────────────────

/// The root cgroup. Always present, never removable, never limited.
pub const CGROUP_ROOT: u64 = 0;

/// No limit. The filesystem spells this `max`.
pub const CGROUP_UNLIMITED: u64 = u64::MAX;

/// How many cgroups can exist at once. Small on purpose: this is a kernel
/// feature demonstration with a fixed table, not a general-purpose hierarchy.
const MAX_CGROUPS: usize = 32;

/// The longest name a cgroup accepts, so a hostile `mkdir` cannot grow the
/// table without bound through one entry.
const MAX_NAME_LEN: usize = 64;

#[derive(Clone)]
struct Member {
    task: u64,
    /// Bytes this task currently has charged to the group.
    charged: u64,
}

#[derive(Clone)]
struct Cgroup {
    id: u64,
    name: String,
    memory_max: u64,
    pids_max: u64,
    members: Vec<Member>,
}

/// The controller table.
///
/// Pure: no locks, no hardware, no globals. Everything the tests need to prove
/// is a method here, so `pids` and `memory` enforcement can be exercised on the
/// host without a scheduler or a page table.
pub struct CgroupTable {
    /// Every non-root group. The root lives in `root_members` instead, so it has
    /// no id, name or limits to get wrong or to remove.
    groups: Vec<Cgroup>,
    root_members: Vec<Member>,
    next_id: u64,
}

/// Why [`CgroupTable::attach`] refused.
#[derive(Debug, PartialEq, Eq)]
pub enum AttachError {
    /// The group does not exist.
    NoSuchGroup,
    /// The group already holds `pids.max` tasks.
    PidsLimit,
}

impl CgroupTable {
    /// A table with only the root. `const`, so the global is a valid table from
    /// the first instruction — see the module docs on the rootless bug this
    /// avoids.
    pub const fn empty() -> Self {
        CgroupTable {
            groups: Vec::new(),
            root_members: Vec::new(),
            next_id: 1,
        }
    }

    /// Drop every group and every membership, keeping only the root. The boot
    /// path never calls it; the tests do.
    pub fn reset(&mut self) {
        self.groups.clear();
        self.root_members.clear();
        self.next_id = 1;
    }

    fn index_of(&self, id: u64) -> Option<usize> {
        self.groups.iter().position(|g| g.id == id)
    }

    /// Whether `id` names a group (including the root).
    pub fn exists(&self, id: u64) -> bool {
        id == CGROUP_ROOT || self.index_of(id).is_some()
    }

    fn members(&self, id: u64) -> Option<&Vec<Member>> {
        if id == CGROUP_ROOT {
            Some(&self.root_members)
        } else {
            self.index_of(id).map(|i| &self.groups[i].members)
        }
    }

    fn members_mut(&mut self, id: u64) -> Option<&mut Vec<Member>> {
        if id == CGROUP_ROOT {
            Some(&mut self.root_members)
        } else {
            self.index_of(id).map(|i| &mut self.groups[i].members)
        }
    }

    /// Create a cgroup named `name` under the root.
    ///
    /// Rejects an empty or over-long name, a duplicate, a name that is a
    /// reserved path, or a full table. Returns the new group's id.
    pub fn create(&mut self, name: &str) -> Option<u64> {
        if name.is_empty() || name.len() > MAX_NAME_LEN || name == "/" {
            return None;
        }
        if self.groups.iter().any(|g| g.name == name) {
            return None;
        }
        if self.groups.len() >= MAX_CGROUPS {
            return None;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.groups.push(Cgroup {
            id,
            name: name.to_string(),
            memory_max: CGROUP_UNLIMITED,
            pids_max: CGROUP_UNLIMITED,
            members: Vec::new(),
        });
        Some(id)
    }

    /// Remove a cgroup. Refuses the root, an unknown id, and a group that still
    /// holds tasks — moving them somewhere implicitly would hide the mistake.
    pub fn remove(&mut self, id: u64) -> bool {
        if id == CGROUP_ROOT {
            return false;
        }
        let Some(i) = self.index_of(id) else {
            return false;
        };
        if !self.groups[i].members.is_empty() {
            return false;
        }
        self.groups.remove(i);
        true
    }

    /// Look up a cgroup by name (`"/"` is the root).
    pub fn lookup_name(&self, name: &str) -> Option<u64> {
        if name == "/" {
            return Some(CGROUP_ROOT);
        }
        self.groups.iter().find(|g| g.name == name).map(|g| g.id)
    }

    /// id → name, for the filesystem's directory listing.
    pub fn name_of(&self, id: u64) -> Option<&str> {
        if id == CGROUP_ROOT {
            return Some("/");
        }
        self.index_of(id).map(|i| self.groups[i].name.as_str())
    }

    pub fn ids(&self) -> impl Iterator<Item = u64> + '_ {
        core::iter::once(CGROUP_ROOT).chain(self.groups.iter().map(|g| g.id))
    }

    /// The task ids in `id`, in attachment order.
    pub fn member_tasks(&self, id: u64) -> Vec<u64> {
        self.members(id)
            .map(|v| v.iter().map(|m| m.task).collect())
            .unwrap_or_default()
    }

    /// The number of tasks currently in `id`.
    pub fn pids_current(&self, id: u64) -> usize {
        self.members(id).map(|v| v.len()).unwrap_or(0)
    }

    /// The bytes currently charged to `id`.
    pub fn memory_current(&self, id: u64) -> u64 {
        self.members(id)
            .map(|v| v.iter().map(|m| m.charged).sum())
            .unwrap_or(0)
    }

    /// `pids.max` for `id`, or `None` if it does not exist. The root is always
    /// unlimited.
    pub fn pids_max(&self, id: u64) -> Option<u64> {
        if id == CGROUP_ROOT {
            return Some(CGROUP_UNLIMITED);
        }
        self.index_of(id).map(|i| self.groups[i].pids_max)
    }

    /// `memory.max` for `id`, or `None` if it does not exist. The root is always
    /// unlimited.
    pub fn memory_max(&self, id: u64) -> Option<u64> {
        if id == CGROUP_ROOT {
            return Some(CGROUP_UNLIMITED);
        }
        self.index_of(id).map(|i| self.groups[i].memory_max)
    }

    /// Set `pids.max`. `0` would freeze the group (no new task can ever attach),
    /// which is a legal cgroup v2 state, so it is accepted. The root cannot be
    /// limited.
    pub fn set_pids_max(&mut self, id: u64, max: u64) -> bool {
        if id == CGROUP_ROOT {
            return false;
        }
        match self.index_of(id) {
            Some(i) => {
                self.groups[i].pids_max = max;
                true
            }
            None => false,
        }
    }

    /// Set `memory.max`. The root cannot be limited.
    pub fn set_memory_max(&mut self, id: u64, max: u64) -> bool {
        if id == CGROUP_ROOT {
            return false;
        }
        match self.index_of(id) {
            Some(i) => {
                self.groups[i].memory_max = max;
                true
            }
            None => false,
        }
    }

    /// Which group a task belongs to, or `None` if it is not attached anywhere.
    pub fn group_of_task(&self, task: u64) -> Option<u64> {
        if self.root_members.iter().any(|m| m.task == task) {
            return Some(CGROUP_ROOT);
        }
        self.groups
            .iter()
            .find(|g| g.members.iter().any(|m| m.task == task))
            .map(|g| g.id)
    }

    /// Put `task` in `id`, enforcing `pids.max`.
    ///
    /// Idempotent for a task already in `id`. A task in another group is *moved*,
    /// carrying whatever memory it had charged, so a task is never in two groups
    /// and its charge is never counted twice.
    pub fn attach(&mut self, id: u64, task: u64) -> Result<(), AttachError> {
        if !self.exists(id) {
            return Err(AttachError::NoSuchGroup);
        }
        if self.group_of_task(task) == Some(id) {
            return Ok(());
        }
        // Take the task (and its charge) out of wherever it was first.
        let carried = self.detach(task);
        let current = self.pids_current(id) as u64;
        let max = self.pids_max(id).unwrap_or(CGROUP_UNLIMITED);
        if current >= max {
            return Err(AttachError::PidsLimit);
        }
        self.members_mut(id)
            .expect("exists checked above")
            .push(Member {
                task,
                charged: carried,
            });
        Ok(())
    }

    /// Remove `task` from whatever group holds it, returning the bytes it had
    /// charged so a move can carry them. A task in no group returns `0`.
    pub fn detach(&mut self, task: u64) -> u64 {
        if let Some(pos) = self.root_members.iter().position(|m| m.task == task) {
            return self.root_members.remove(pos).charged;
        }
        for g in self.groups.iter_mut() {
            if let Some(pos) = g.members.iter().position(|m| m.task == task) {
                return g.members.remove(pos).charged;
            }
        }
        0
    }

    /// Charge `bytes` to `task`'s group.
    ///
    /// `false` means the charge would push the group past `memory.max` and
    /// nothing was charged — the caller must not map the pages. A task in no
    /// group (or the unlimited root) always succeeds.
    pub fn charge(&mut self, task: u64, bytes: u64) -> bool {
        let Some(gid) = self.group_of_task(task) else {
            return true;
        };
        let max = self.memory_max(gid).unwrap_or(CGROUP_UNLIMITED);
        if max != CGROUP_UNLIMITED && self.memory_current(gid).saturating_add(bytes) > max {
            return false;
        }
        if let Some(m) = self
            .members_mut(gid)
            .and_then(|v| v.iter_mut().find(|m| m.task == task))
        {
            m.charged = m.charged.saturating_add(bytes);
        }
        true
    }

    /// Give `bytes` back to `task`'s group.
    ///
    /// Saturates at zero: an over-release is a bug in the caller, but turning it
    /// into a panic would make it a kernel panic on the `munmap` path.
    pub fn uncharge(&mut self, task: u64, bytes: u64) {
        let Some(gid) = self.group_of_task(task) else {
            return;
        };
        if let Some(m) = self
            .members_mut(gid)
            .and_then(|v| v.iter_mut().find(|m| m.task == task))
        {
            m.charged = m.charged.saturating_sub(bytes);
        }
    }
}

/// The one table the kernel uses.
pub static CGROUP_TABLE: SpinLock<CgroupTable> = SpinLock::new(CgroupTable::empty());

// ── the enforcement API the scheduler and the syscall layer call ─────────

/// Attach a new task to `group`, enforcing `pids.max`.
///
/// `false` means the group is at its limit and the task must not be created.
pub fn attach_task(group: u64, task: u64) -> bool {
    CGROUP_TABLE.lock().attach(group, task).is_ok()
}

/// Remove a task from its group. Idempotent, and a no-op for an unknown task, so
/// every teardown path can call it without coordinating with the others.
pub fn detach_task(task: u64) {
    CGROUP_TABLE.lock().detach(task);
}

/// Which group a task is in, or [`CGROUP_ROOT`] when it is in none.
pub fn group_of_task(task: u64) -> u64 {
    CGROUP_TABLE
        .lock()
        .group_of_task(task)
        .unwrap_or(CGROUP_ROOT)
}

/// Charge `bytes` to a task's group, enforcing `memory.max`.
pub fn charge_memory(task: u64, bytes: u64) -> bool {
    CGROUP_TABLE.lock().charge(task, bytes)
}

/// Return `bytes` to a task's group.
pub fn uncharge_memory(task: u64, bytes: u64) {
    CGROUP_TABLE.lock().uncharge(task, bytes);
}

/// Empty the table. Tests only.
#[cfg(test)]
pub fn reset_for_tests() {
    CGROUP_TABLE.lock().reset();
}

// ── the filesystem ───────────────────────────────────────────────────────

pub struct CgroupFs;

/// Render a limit the way cgroup v2 does: `max` for unlimited, else decimal.
fn render_limit(v: u64) -> String {
    if v == CGROUP_UNLIMITED {
        String::from("max\n")
    } else {
        format!("{v}\n")
    }
}

/// Parse a written limit: `max` or a decimal number.
fn parse_limit(s: &str) -> Option<u64> {
    let s = s.trim();
    if s == "max" {
        return Some(CGROUP_UNLIMITED);
    }
    s.parse::<u64>().ok()
}

fn read_into(content: &str, offset: u64, buf: &mut [u8]) -> Option<u64> {
    let bytes = content.as_bytes();
    let start = offset as usize;
    if start >= bytes.len() {
        return Some(0);
    }
    let len = core::cmp::min(buf.len(), bytes.len() - start);
    buf[..len].copy_from_slice(&bytes[start..start + len]);
    Some(len as u64)
}

impl CgroupFs {
    fn read_spec(&self, cg: u64, idx: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        let table = CGROUP_TABLE.lock();
        if !table.exists(cg) {
            return Some(0);
        }
        let content = match idx {
            F_PROCS => {
                let mut s = String::new();
                for t in table.member_tasks(cg) {
                    s.push_str(&format!("{t}\n"));
                }
                s
            }
            F_CONTROLLERS => String::from("cpu io memory pids\n"),
            // Present for layout compatibility. Enabling a controller for
            // children is not modelled (the hierarchy is flat and the limit is
            // enforced whether or not this was written), so it reads empty and
            // `write` refuses rather than accept an enablement it ignores.
            F_SUBTREE => String::new(),
            F_PIDS_MAX => render_limit(table.pids_max(cg).unwrap_or(CGROUP_UNLIMITED)),
            F_PIDS_CURRENT => format!("{}\n", table.pids_current(cg)),
            F_MEM_MAX => render_limit(table.memory_max(cg).unwrap_or(CGROUP_UNLIMITED)),
            F_MEM_CURRENT => format!("{}\n", table.memory_current(cg)),
            _ => return Some(0),
        };
        read_into(&content, offset, buf)
    }

    fn write_spec(&self, cg: u64, idx: u64, buf: &[u8]) -> Option<u64> {
        let Ok(text) = core::str::from_utf8(buf) else {
            return None;
        };
        let mut table = CGROUP_TABLE.lock();
        match idx {
            F_PROCS => {
                let task: u64 = text.trim().parse().ok()?;
                // A write to `cgroup.procs` moves the task into this group; the
                // pids limit applies exactly as it does at task creation.
                table.attach(cg, task).ok().map(|_| buf.len() as u64)
            }
            F_PIDS_MAX => {
                let max = parse_limit(text)?;
                table.set_pids_max(cg, max).then_some(buf.len() as u64)
            }
            F_MEM_MAX => {
                let max = parse_limit(text)?;
                table.set_memory_max(cg, max).then_some(buf.len() as u64)
            }
            // Enabling a controller would be a silent no-op; refuse it.
            _ => None,
        }
    }
}

impl FileSystem for CgroupFs {
    fn name(&self) -> &'static str {
        "cgroup2"
    }
    fn root_inode(&self) -> u64 {
        CGROUP_ROOT_INODE
    }

    fn read(&self, inode: u64, offset: u64, buf: &mut [u8]) -> Option<u64> {
        // Static root files that are not part of the controller: pressure stubs
        // and cgroup.defaults. They are empty and read-only, as before.
        match inode {
            CGROUP_DEFAULTS_INODE
            | CGROUP_CPU_PRESSURE_INODE
            | CGROUP_IO_PRESSURE_INODE
            | CGROUP_MEMORY_PRESSURE_INODE => return Some(0),
            _ => {}
        }
        if let Some((cg, idx)) = root_spec(inode) {
            return self.read_spec(cg, idx, offset, buf);
        }
        if let Some((cg, idx)) = decode_inode(inode) {
            return self.read_spec(cg, idx, offset, buf);
        }
        Some(0)
    }

    fn write(&self, inode: u64, _offset: u64, buf: &[u8]) -> Option<u64> {
        if let Some((cg, idx)) = root_spec(inode) {
            return self.write_spec(cg, idx, buf);
        }
        if let Some((cg, idx)) = decode_inode(inode) {
            return self.write_spec(cg, idx, buf);
        }
        None
    }

    fn read_dir(&self, inode: u64) -> Vec<DirEntry> {
        if inode == CGROUP_ROOT_INODE {
            let mut entries: Vec<DirEntry> = ROOT_FILES
                .iter()
                .map(|(name, ino, ft)| DirEntry {
                    name: String::from(*name),
                    file_type: *ft,
                    inode: *ino,
                })
                .collect();
            let table = CGROUP_TABLE.lock();
            for id in table.ids().collect::<Vec<u64>>() {
                if id == CGROUP_ROOT {
                    continue;
                }
                entries.push(DirEntry {
                    name: table.name_of(id).unwrap_or("").to_string(),
                    file_type: FileType::Directory,
                    inode: dir_inode(id),
                });
            }
            return entries;
        }
        if let Some(cg) = decode_dir_inode(inode) {
            if CGROUP_TABLE.lock().exists(cg) {
                return CGROUP_FILES
                    .iter()
                    .map(|(name, idx, ft)| DirEntry {
                        name: String::from(*name),
                        file_type: *ft,
                        inode: file_inode(cg, *idx),
                    })
                    .collect();
            }
        }
        Vec::new()
    }

    fn stat(&self, inode: u64) -> FileStat {
        let file = FileStat {
            size: 0,
            file_type: FileType::File,
            inode,
            blocks: 0,
            uid: 0,
            gid: 0,
            mode: 0o644,
        };
        if inode == CGROUP_ROOT_INODE {
            return FileStat {
                size: 0,
                file_type: FileType::Directory,
                inode: 0,
                blocks: 0,
                uid: 0,
                gid: 0,
                mode: 0o755,
            };
        }
        if let Some(cg) = decode_dir_inode(inode) {
            if CGROUP_TABLE.lock().exists(cg) {
                return FileStat {
                    size: 0,
                    file_type: FileType::Directory,
                    inode,
                    blocks: 0,
                    uid: 0,
                    gid: 0,
                    mode: 0o755,
                };
            }
        }
        // Every root file, including the pressure stubs, is a readable file —
        // the listing and the stat must agree (a past bug had them not).
        file
    }

    fn create(&self, parent_inode: u64, name: &str, file_type: FileType) -> Option<u64> {
        // Only directories, and only under the root: the hierarchy is flat.
        if parent_inode != CGROUP_ROOT_INODE || file_type != FileType::Directory {
            return None;
        }
        let id = CGROUP_TABLE.lock().create(name)?;
        Some(dir_inode(id))
    }

    fn unlink(&self, parent_inode: u64, name: &str) -> bool {
        if parent_inode != CGROUP_ROOT_INODE {
            return false;
        }
        let mut table = CGROUP_TABLE.lock();
        match table.lookup_name(name) {
            Some(id) if id != CGROUP_ROOT => table.remove(id),
            _ => false,
        }
    }

    fn lookup(&self, parent_inode: u64, name: &str) -> Option<u64> {
        if parent_inode == CGROUP_ROOT_INODE {
            if let Some((_n, ino, _ft)) = ROOT_FILES.iter().find(|(n, _, _)| *n == name) {
                return Some(*ino);
            }
            let id = CGROUP_TABLE.lock().lookup_name(name)?;
            if id == CGROUP_ROOT {
                return None;
            }
            return Some(dir_inode(id));
        }
        // Inside a cgroup directory: its standard files.
        let cg = decode_dir_inode(parent_inode)?;
        if !CGROUP_TABLE.lock().exists(cg) {
            return None;
        }
        CGROUP_FILES
            .iter()
            .find(|(n, _, _)| *n == name)
            .map(|(_, idx, _)| file_inode(cg, *idx))
    }

    fn chmod(&self, _inode: u64, _mode: u16) -> bool {
        false
    }
    fn chown(&self, _inode: u64, _uid: u32, _gid: u32) -> bool {
        false
    }
}
