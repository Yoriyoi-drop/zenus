use zenus_sync::spinlock::SpinLock;
use crate::{NsId, alloc_ns_id, NS_ROOT};

const MAX_IPC_NAMESPACES: usize = 16;

#[derive(Clone, Copy)]
struct IpcNamespace {
    id: NsId,
}

impl IpcNamespace {
    const fn new(id: NsId) -> Self {
        IpcNamespace { id }
    }
}

struct IpcTable {
    namespaces: [Option<IpcNamespace>; MAX_IPC_NAMESPACES],
    count: usize,
}

impl IpcTable {
    const fn new() -> Self {
        IpcTable {
            namespaces: [None; MAX_IPC_NAMESPACES],
            count: 0,
        }
    }
}

static IPC_TABLE: SpinLock<IpcTable> = SpinLock::new(IpcTable::new());

pub fn init() {
    let mut table = IPC_TABLE.lock();
    table.namespaces[0] = Some(IpcNamespace::new(NS_ROOT));
    table.count = 1;
    zenus_console::kinfo!("IPC namespace initialized");
}

pub fn create() -> Option<NsId> {
    let mut table = IPC_TABLE.lock();
    if table.count >= MAX_IPC_NAMESPACES {
        return None;
    }
    let id = alloc_ns_id();
    let ns = IpcNamespace::new(id);
    let idx = table.count;
    table.namespaces[idx] = Some(ns);
    table.count += 1;
    Some(id)
}

pub fn destroy(id: NsId) {
    let mut table = IPC_TABLE.lock();
    let idx = match find_idx(&table, id) {
        Some(i) => i,
        None => return,
    };
    for i in idx..table.count.saturating_sub(1) {
        table.namespaces[i] = table.namespaces[i + 1].take();
    }
    table.count = table.count.saturating_sub(1);
}

fn find_idx(table: &IpcTable, id: NsId) -> Option<usize> {
    for i in 0..table.count {
        if let Some(ref ns) = table.namespaces[i] {
            if ns.id == id {
                return Some(i);
            }
        }
    }
    None
}

pub fn exists(id: NsId) -> bool {
    let table = IPC_TABLE.lock();
    find_idx(&table, id).is_some()
}
