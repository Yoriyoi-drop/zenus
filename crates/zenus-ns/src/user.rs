use zenus_sync::spinlock::SpinLock;
use crate::{NsId, alloc_ns_id, NS_ROOT};

const MAX_USER_NAMESPACES: usize = 16;
const MAX_UID_MAP: usize = 16;

#[derive(Clone, Copy)]
struct UserNamespace {
    id: NsId,
    uid_map: [(u32, u32); MAX_UID_MAP],
    count: usize,
}

impl UserNamespace {
    const fn new(id: NsId) -> Self {
        UserNamespace {
            id,
            uid_map: [(0, 0); MAX_UID_MAP],
            count: 0,
        }
    }
}

struct UserTable {
    namespaces: [Option<UserNamespace>; MAX_USER_NAMESPACES],
    count: usize,
}

impl UserTable {
    const fn new() -> Self {
        UserTable {
            namespaces: [None; MAX_USER_NAMESPACES],
            count: 0,
        }
    }
}

static USER_TABLE: SpinLock<UserTable> = SpinLock::new(UserTable::new());

pub fn init() {
    let mut table = USER_TABLE.lock();
    let mut root = UserNamespace::new(NS_ROOT);
    root.uid_map[0] = (0, 0);
    root.count = 1;
    table.namespaces[0] = Some(root);
    table.count = 1;
    zenus_console::kinfo!("User namespace initialized");
}

pub fn create() -> Option<NsId> {
    let mut table = USER_TABLE.lock();
    if table.count >= MAX_USER_NAMESPACES {
        return None;
    }
    let id = alloc_ns_id();
    let ns = UserNamespace::new(id);
    let idx = table.count;
    table.namespaces[idx] = Some(ns);
    table.count += 1;
    Some(id)
}

pub fn destroy(id: NsId) {
    let mut table = USER_TABLE.lock();
    let idx = match find_idx(&table, id) {
        Some(i) => i,
        None => return,
    };
    for i in idx..table.count.saturating_sub(1) {
        table.namespaces[i] = table.namespaces[i + 1].take();
    }
    table.count = table.count.saturating_sub(1);
}

fn find_idx(table: &UserTable, id: NsId) -> Option<usize> {
    for i in 0..table.count {
        if let Some(ref ns) = table.namespaces[i] {
            if ns.id == id {
                return Some(i);
            }
        }
    }
    None
}

pub fn map_uid(ns_id: NsId, inside: u32, outside: u32) -> bool {
    let mut table = USER_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return false,
    };
    let ns = match table.namespaces[idx].as_mut() {
        Some(n) => n,
        None => return false,
    };
    if ns.count >= MAX_UID_MAP {
        return false;
    }
    ns.uid_map[ns.count] = (inside, outside);
    ns.count += 1;
    true
}

pub fn translate_uid(ns_id: NsId, inside: u32) -> Option<u32> {
    let table = USER_TABLE.lock();
    let idx = find_idx(&table, ns_id)?;
    let ns = table.namespaces[idx].as_ref()?;
    for i in 0..ns.count {
        if ns.uid_map[i].0 == inside {
            return Some(ns.uid_map[i].1);
        }
    }
    None
}
