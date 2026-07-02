use zenus_sync::spinlock::SpinLock;
use crate::{NsId, alloc_ns_id, NS_ROOT};

const MAX_MNT_NAMESPACES: usize = 16;

#[derive(Clone, Copy)]
struct MntNamespace {
    id: NsId,
}

impl MntNamespace {
    const fn new(id: NsId) -> Self {
        MntNamespace { id }
    }
}

struct MntTable {
    namespaces: [Option<MntNamespace>; MAX_MNT_NAMESPACES],
    count: usize,
}

impl MntTable {
    const fn new() -> Self {
        MntTable {
            namespaces: [None; MAX_MNT_NAMESPACES],
            count: 0,
        }
    }
}

static MNT_TABLE: SpinLock<MntTable> = SpinLock::new(MntTable::new());

pub fn init() {
    let mut table = MNT_TABLE.lock();
    table.namespaces[0] = Some(MntNamespace::new(NS_ROOT));
    table.count = 1;
    zenus_console::kinfo!("Mount namespace initialized");
}

pub fn create() -> Option<NsId> {
    let mut table = MNT_TABLE.lock();
    if table.count >= MAX_MNT_NAMESPACES {
        return None;
    }
    let id = alloc_ns_id();
    let ns = MntNamespace::new(id);
    let idx = table.count;
    table.namespaces[idx] = Some(ns);
    table.count += 1;
    Some(id)
}

pub fn destroy(id: NsId) {
    let mut table = MNT_TABLE.lock();
    let idx = match find_idx(&table, id) {
        Some(i) => i,
        None => return,
    };
    for i in idx..table.count.saturating_sub(1) {
        table.namespaces[i] = table.namespaces[i + 1].take();
    }
    table.count = table.count.saturating_sub(1);
}

fn find_idx(table: &MntTable, id: NsId) -> Option<usize> {
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
    let table = MNT_TABLE.lock();
    find_idx(&table, id).is_some()
}
