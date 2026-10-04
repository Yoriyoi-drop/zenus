use zenus_sync::spinlock::SpinLock;

#[derive(Clone, Copy)]
pub struct Route {
    pub dest: [u8; 4],
    pub mask: [u8; 4],
    pub gateway: [u8; 4],
    pub iface: usize,
}

const MAX_ROUTES: usize = 8;

/// Route table.
///
/// Was a bare `static mut` with no lock: `add`/`lookup`/`clear` from two CPUs
/// raced on the same slots, so a route could appear half-written. Everything in
/// this crate already uses `SpinLock` for its globals, so this does too.
static ROUTE_TABLE: SpinLock<[Option<Route>; MAX_ROUTES]> =
    SpinLock::new([None; MAX_ROUTES]);

pub fn add(dest: [u8; 4], mask: [u8; 4], gateway: [u8; 4], iface: usize) -> bool {
    let mut table = ROUTE_TABLE.lock();
    for slot in table.iter_mut() {
        if slot.is_none() {
            *slot = Some(Route {
                dest,
                mask,
                gateway,
                iface,
            });
            return true;
        }
    }
    false
}

pub fn add_default(gateway: [u8; 4], iface: usize) -> bool {
    add([0; 4], [0; 4], gateway, iface)
}

pub fn add_direct(network: [u8; 4], mask: [u8; 4], iface: usize) -> bool {
    add(network, mask, [0; 4], iface)
}

fn prefix_len(mask: [u8; 4]) -> u32 {
    let mut n = 0u32;
    for i in 0..4 {
        n += (mask[i].count_ones()) as u32;
    }
    n
}

fn ip_masked(ip: [u8; 4], mask: [u8; 4]) -> [u8; 4] {
    [
        ip[0] & mask[0],
        ip[1] & mask[1],
        ip[2] & mask[2],
        ip[3] & mask[3],
    ]
}

pub fn lookup(ip: [u8; 4]) -> Option<(GatewayAction, usize)> {
    let table = ROUTE_TABLE.lock();
    let mut best: Option<(GatewayAction, usize, u32)> = None;
    for slot in table.iter() {
        if let Some(r) = slot {
            let plen = prefix_len(r.mask);
            if ip_masked(ip, r.mask) == r.dest && best.map(|b| plen > b.2).unwrap_or(true) {
                let gw = if r.gateway == [0; 4] {
                    GatewayAction::Direct
                } else {
                    GatewayAction::Via(r.gateway)
                };
                best = Some((gw, r.iface, plen));
            }
        }
    }
    best.map(|(gw, iface, _)| (gw, iface))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GatewayAction {
    Direct,
    Via([u8; 4]),
}

pub fn clear() {
    *ROUTE_TABLE.lock() = [None; MAX_ROUTES];
}
