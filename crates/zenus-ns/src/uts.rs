use zenus_sync::spinlock::SpinLock;
use crate::{NsId, alloc_ns_id, NS_ROOT};

const MAX_HOSTNAME: usize = 64;
const MAX_NAMESPACES: usize = 16;

#[derive(Clone, Copy)]
struct UtsNamespace {
    id: NsId,
    hostname: [u8; MAX_HOSTNAME],
    domainname: [u8; MAX_HOSTNAME],
}

impl UtsNamespace {
    const fn new(id: NsId) -> Self {
        UtsNamespace {
            id,
            hostname: [0; MAX_HOSTNAME],
            domainname: [0; MAX_HOSTNAME],
        }
    }
}

struct UtsTable {
    namespaces: [Option<UtsNamespace>; MAX_NAMESPACES],
    count: usize,
}

impl UtsTable {
    const fn new() -> Self {
        UtsTable {
            namespaces: [None; MAX_NAMESPACES],
            count: 0,
        }
    }
}

static UTS_TABLE: SpinLock<UtsTable> = SpinLock::new(UtsTable::new());

pub fn init() {
    let mut table = UTS_TABLE.lock();
    let mut root = UtsNamespace::new(NS_ROOT);
    let hostname = b"zenus\0";
    let domainname = b"(none)\0";
    let hlen = hostname.len().min(MAX_HOSTNAME);
    root.hostname[..hlen].copy_from_slice(&hostname[..hlen]);
    let dlen = domainname.len().min(MAX_HOSTNAME);
    root.domainname[..dlen].copy_from_slice(&domainname[..dlen]);
    table.namespaces[0] = Some(root);
    table.count = 1;
}

pub fn create() -> Option<NsId> {
    let mut table = UTS_TABLE.lock();
    if table.count >= MAX_NAMESPACES {
        return None;
    }
    let id = alloc_ns_id();
    let ns = UtsNamespace::new(id);
    let idx = table.count;
    table.namespaces[idx] = Some(ns);
    table.count += 1;
    Some(id)
}

fn find_idx(table: &UtsTable, id: NsId) -> Option<usize> {
    for i in 0..table.count {
        if let Some(ref ns) = table.namespaces[i] {
            if ns.id == id {
                return Some(i);
            }
        }
    }
    None
}

pub fn set_hostname(ns_id: NsId, hostname: &[u8]) -> bool {
    let mut table = UTS_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return false,
    };
    let ns = match table.namespaces[idx].as_mut() {
        Some(n) => n,
        None => return false,
    };
    let len = hostname.len().min(MAX_HOSTNAME - 1);
    ns.hostname = [0; MAX_HOSTNAME];
    ns.hostname[..len].copy_from_slice(&hostname[..len]);
    true
}

pub fn set_domainname(ns_id: NsId, domainname: &[u8]) -> bool {
    let mut table = UTS_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return false,
    };
    let ns = match table.namespaces[idx].as_mut() {
        Some(n) => n,
        None => return false,
    };
    let len = domainname.len().min(MAX_HOSTNAME - 1);
    ns.domainname = [0; MAX_HOSTNAME];
    ns.domainname[..len].copy_from_slice(&domainname[..len]);
    true
}

pub fn get_hostname(ns_id: NsId) -> [u8; MAX_HOSTNAME] {
    let table = UTS_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return [0; MAX_HOSTNAME],
    };
    match table.namespaces[idx] {
        Some(ref ns) => ns.hostname,
        None => [0; MAX_HOSTNAME],
    }
}

pub fn get_domainname(ns_id: NsId) -> [u8; MAX_HOSTNAME] {
    let table = UTS_TABLE.lock();
    let idx = match find_idx(&table, ns_id) {
        Some(i) => i,
        None => return [0; MAX_HOSTNAME],
    };
    match table.namespaces[idx] {
        Some(ref ns) => ns.domainname,
        None => [0; MAX_HOSTNAME],
    }
}
