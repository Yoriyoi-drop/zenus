use crate::ethernet;

const HARDWARE_TYPE_ETH: u16 = 0x0001;
const PROTOCOL_TYPE_IPV4: u16 = 0x0800;
const ARP_REQUEST: u16 = 0x0001;
const ARP_REPLY: u16 = 0x0002;

const ARP_CACHE_SIZE: usize = 16;

static mut ARP_GATEWAY: [u8; 4] = [10, 0, 2, 2];

#[derive(Clone, Copy)]
struct ArpEntry {
    ip: [u8; 4],
    mac: [u8; 6],
    valid: bool,
}

static mut ARP_CACHE: [ArpEntry; ARP_CACHE_SIZE] = [ArpEntry { ip: [0; 4], mac: [0; 6], valid: false }; ARP_CACHE_SIZE];

pub fn set_gateway(gw: [u8; 4]) {
    unsafe { ARP_GATEWAY = gw; }
}

fn arp_lookup(target_ip: [u8; 4]) -> Option<[u8; 6]> {
    unsafe {
        for i in 0..ARP_CACHE_SIZE {
            if ARP_CACHE[i].valid && ARP_CACHE[i].ip == target_ip {
                return Some(ARP_CACHE[i].mac);
            }
        }
    }
    None
}

pub fn add_static(ip: [u8; 4], mac: [u8; 6]) {
    arp_insert(ip, mac);
}
fn arp_insert(ip: [u8; 4], mac: [u8; 6]) {
    unsafe {
        if ip == ARP_GATEWAY {
            return;
        }
        for i in 0..ARP_CACHE_SIZE {
            if !ARP_CACHE[i].valid {
                ARP_CACHE[i] = ArpEntry { ip, mac, valid: true };
                return;
            }
            if ARP_CACHE[i].ip == ip {
                if ARP_CACHE[i].mac != mac {
                    return;
                }
                return;
            }
        }
        // All slots full: evict oldest non-gateway entry
        for i in 1..ARP_CACHE_SIZE {
            if ARP_CACHE[i].ip != ARP_GATEWAY {
                ARP_CACHE[i] = ArpEntry { ip, mac, valid: true };
                return;
            }
        }
    }
}

fn arp_packet(src_mac: &[u8; 6], src_ip: &[u8; 4], dst_mac: &[u8; 6], dst_ip: &[u8; 4], opcode: u16) -> [u8; 42] {
    let mut buf = [0u8; 42];
    buf[0..6].copy_from_slice(dst_mac);
    buf[6..12].copy_from_slice(src_mac);
    buf[12..14].copy_from_slice(&crate::ethernet::ETH_ARP.to_be_bytes());
    let mut off = 14;
    buf[off..off + 2].copy_from_slice(&HARDWARE_TYPE_ETH.to_be_bytes()); off += 2;
    buf[off..off + 2].copy_from_slice(&PROTOCOL_TYPE_IPV4.to_be_bytes()); off += 2;
    buf[off] = 6; off += 1;
    buf[off] = 4; off += 1;
    buf[off..off + 2].copy_from_slice(&opcode.to_be_bytes()); off += 2;
    buf[off..off + 6].copy_from_slice(src_mac); off += 6;
    buf[off..off + 4].copy_from_slice(src_ip); off += 4;
    buf[off..off + 6].copy_from_slice(dst_mac); off += 6;
    buf[off..off + 4].copy_from_slice(dst_ip);
    buf
}

pub fn resolve(iface_idx: usize, target_ip: [u8; 4]) -> Option<[u8; 6]> {
    if let Some(mac) = arp_lookup(target_ip) {
        return Some(mac);
    }
    send_request(iface_idx, target_ip);
    None
}

pub fn send_request(iface_idx: usize, target_ip: [u8; 4]) -> bool {
    let iface = match crate::nic::get_iface(iface_idx) {
        Some(iface) => iface,
        None => return false,
    };
    let broadcast = [0xFF; 6];
    let pkt = arp_packet(
        &iface.mac, &iface.ip,
        &broadcast, &target_ip,
        ARP_REQUEST,
    );
    crate::nic::send_frame(iface_idx, &pkt)
}

pub fn handle(
    _eth_hdr: &ethernet::EthernetHeader,
    payload: &[u8],
    our_ip: &[u8; 4],
    our_mac: &[u8; 6],
) -> Option<[u8; 42]> {
    if payload.len() < 28 {
        return None;
    }
    let ptr = payload.as_ptr();
    let hw_type = u16::from_be(unsafe { core::ptr::read_unaligned(ptr as *const u16) });
    let proto_type = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(2) as *const u16) });
    let hw_addr_len = unsafe { *ptr.add(4) };
    let proto_addr_len = unsafe { *ptr.add(5) };
    let opcode = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(6) as *const u16) });

    if hw_type != HARDWARE_TYPE_ETH { return None; }
    if proto_type != PROTOCOL_TYPE_IPV4 { return None; }
    if hw_addr_len != 6 || proto_addr_len != 4 { return None; }

    let sender_mac = unsafe { core::ptr::read_unaligned(ptr.add(8) as *const [u8; 6]) };
    let sender_ip = unsafe { core::ptr::read_unaligned(ptr.add(14) as *const [u8; 4]) };

    arp_insert(sender_ip, sender_mac);

    if opcode == ARP_REQUEST {
        let target_ip = unsafe { core::ptr::read_unaligned(ptr.add(24) as *const [u8; 4]) };
        if target_ip != *our_ip {
            return None;
        }
        Some(arp_packet(our_mac, our_ip, &sender_mac, &sender_ip, ARP_REPLY))
    } else {
        None
    }
}
