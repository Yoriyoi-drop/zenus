use zenus_sync::spinlock::SpinLock;
use crate::{NsId, alloc_ns_id, NS_ROOT};

const MAX_PID_NAMESPACES: usize = 16;

/// Per-namespace PID mapping: maps local PID to global task ID.
#[derive(Clone, Copy)]
struct PidEntry {
    local_pid: u64,
    global_tid: u64,
}

#[derive(Clone, Copy)]
struct PidNamespace {
    id: NsId,
    next_local_pid: u64,
    entries: [Option<PidEntry>; 64],
    entry_count: usize,
}

impl PidNamespace {
    const fn new(id: NsId) -> Self {
        PidNamespace {
            id,
            next_local_pid: 1,
            entries: [None; 64],
            entry_count: 0,
        }
    }
}

struct PidTable {
    namespaces: [Option<PidNamespace>; MAX_PID_NAMESPACES],
    count: usize,
}

impl PidTable {
    const fn new() -> Self {
        PidTable {
            namespaces: [None; MAX_PID_NAMESPACES],
            count: 0,
        }
    }
}

static PID_TABLE: SpinLock<PidTable> = SpinLock::new(PidTable::new());

pub fn init() {
    let mut table = PID_TABLE.lock();
    table.namespaces[0] = Some(PidNamespace::new(NS_ROOT));
    table.count = 1;
}

pub fn create() -> Option<NsId> {
    let mut table = PID_TABLE.lock();
    if table.count >= MAX_PID_NAMESPACES {
        return None;
    }
    let id = alloc_ns_id();
    let mut ns = PidNamespace::new(id);
    ns.next_local_pid = 1;
    let idx = table.count;
    table.namespaces[idx] = Some(ns);
    table.count += 1;
    Some(id)
}

fn find_idx(table: &PidTable, id: NsId) -> Option<usize> {
    for i in 0..table.count {
        if let Some(ref ns) = table.namespaces[i] {
            if ns.id == id {
                return Some(i);
            }
        }
    }
    None
}

/// Register a task (global task ID) in a PID namespace, returning the local PID.
pub fn register_task(ns_id: NsId, global_tid: u64) -> Option<u64> {
    let mut table = PID_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return None,
    };
    let ns = match table.namespaces[idx].as_mut() {
        Some(n) => n,
        None => return None,
    };
    if ns.entry_count >= 64 {
        return None;
    }
    let local_pid = ns.next_local_pid;
    ns.next_local_pid += 1;
    ns.entries[ns.entry_count] = Some(PidEntry { local_pid, global_tid });
    ns.entry_count += 1;
    Some(local_pid)
}

/// Look up the global task ID for a local PID in a namespace.
pub fn global_tid(ns_id: NsId, local_pid: u64) -> Option<u64> {
    let table = PID_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return None,
    };
    let ns = match table.namespaces[idx] {
        Some(ref n) => n,
        None => return None,
    };
    for i in 0..ns.entry_count {
        if let Some(ref entry) = ns.entries[i] {
            if entry.local_pid == local_pid {
                return Some(entry.global_tid);
            }
        }
    }
    None
}

/// Look up the local PID for a global task ID in a namespace.
pub fn local_pid(ns_id: NsId, global_tid: u64) -> Option<u64> {
    let table = PID_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return None,
    };
    let ns = match table.namespaces[idx] {
        Some(ref n) => n,
        None => return None,
    };
    for i in 0..ns.entry_count {
        if let Some(ref entry) = ns.entries[i] {
            if entry.global_tid == global_tid {
                return Some(entry.local_pid);
            }
        }
    }
    None
}

/// Unregister a task (by global task ID) from a PID namespace.
pub fn unregister_task(ns_id: NsId, global_tid: u64) {
    let mut table = PID_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return,
    };
    let ns = match table.namespaces[idx].as_mut() {
        Some(n) => n,
        None => return,
    };
    for i in 0..ns.entry_count {
        if let Some(ref entry) = ns.entries[i] {
            if entry.global_tid == global_tid {
                ns.entries[i] = None;
                // Compact
                for j in i..ns.entry_count.saturating_sub(1) {
                    ns.entries[j] = ns.entries[j + 1].take();
                }
                ns.entry_count = ns.entry_count.saturating_sub(1);
                return;
            }
        }
    }
}
