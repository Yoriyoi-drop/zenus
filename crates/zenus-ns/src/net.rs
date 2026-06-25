use zenus_sync::spinlock::SpinLock;
use crate::{NsId, alloc_ns_id, NS_ROOT};

const MAX_NET_NAMESPACES: usize = 16;

#[derive(Clone, Copy)]
struct NetNamespace {
    id: NsId,
    interfaces: [u8; 8],
}

impl NetNamespace {
    const fn new(id: NsId) -> Self {
        NetNamespace {
            id,
            interfaces: [0; 8],
        }
    }
}

struct NetTable {
    namespaces: [Option<NetNamespace>; MAX_NET_NAMESPACES],
    count: usize,
}

impl NetTable {
    const fn new() -> Self {
        NetTable {
            namespaces: [None; MAX_NET_NAMESPACES],
            count: 0,
        }
    }
}

static NET_TABLE: SpinLock<NetTable> = SpinLock::new(NetTable::new());

pub fn init() {
    let mut table = NET_TABLE.lock();
    table.namespaces[0] = Some(NetNamespace::new(NS_ROOT));
    table.count = 1;
}

pub fn create() -> Option<NsId> {
    let mut table = NET_TABLE.lock();
    if table.count >= MAX_NET_NAMESPACES {
        return None;
    }
    let id = alloc_ns_id();
    let ns = NetNamespace::new(id);
    let idx = table.count;
    table.namespaces[idx] = Some(ns);
    table.count += 1;
    Some(id)
}

pub fn destroy(id: NsId) {
    let mut table = NET_TABLE.lock();
    let idx = match find_idx(&table, id) {
        Some(i) => i,
        None => return,
    };
    for i in idx..table.count.saturating_sub(1) {
        table.namespaces[i] = table.namespaces[i + 1].take();
    }
    table.count = table.count.saturating_sub(1);
}

fn find_idx(table: &NetTable, id: NsId) -> Option<usize> {
    for i in 0..table.count {
        if let Some(ref ns) = table.namespaces[i] {
            if ns.id == id {
                return Some(i);
            }
        }
    }
    None
}

pub fn get_interfaces(id: NsId) -> [u8; 8] {
    let table = NET_TABLE.lock();
    let idx = match find_idx(&table, id) {
        Some(i) => i,
        None => return [0; 8],
    };
    match table.namespaces[idx] {
        Some(ref ns) => ns.interfaces,
        None => [0; 8],
    }
}

pub fn add_interface(id: NsId, iface_idx: u8) -> bool {
    if iface_idx >= 64 {
        return false;
    }
    let mut table = NET_TABLE.lock();
    let idx = match find_idx(&table, id) {
        Some(i) => i,
        None => return false,
    };
    let ns = match table.namespaces[idx].as_mut() {
        Some(n) => n,
        None => return false,
    };
    let byte = (iface_idx / 8) as usize;
    let bit = iface_idx % 8;
    ns.interfaces[byte] |= 1 << bit;
    true
}
