use crate::udp;
use crate::nic;

const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;

const OP_REQUEST: u8 = 1;
const OP_REPLY: u8 = 2;

const MAGIC_COOKIE: [u8; 4] = [0x63, 0x82, 0x53, 0x63];

const OPT_PAD: u8 = 0;
const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_DNS: u8 = 6;
const OPT_LEASE_TIME: u8 = 51;
const OPT_MSG_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_END: u8 = 255;
const OPT_REQ_IP: u8 = 50;

const MSG_DISCOVER: u8 = 1;
const MSG_OFFER: u8 = 2;
const MSG_REQUEST: u8 = 3;
const MSG_ACK: u8 = 5;
const MSG_NAK: u8 = 6;

const LEASE_TABLE_SIZE: usize = 16;
const POOL_START_OCTETS: [u8; 4] = [10, 0, 2, 100];
const POOL_SIZE: u8 = 16;
const DEFAULT_LEASE_SECS: u32 = 86400;

#[derive(Clone, Copy)]
struct Lease {
    ip: [u8; 4],
    mac: [u8; 6],
    expires: u32,
}

static mut LEASES: [Option<Lease>; LEASE_TABLE_SIZE] = [None; LEASE_TABLE_SIZE];

fn ip_to_pool_offset(ip: [u8; 4]) -> Option<u8> {
    let base = u32::from_be_bytes(POOL_START_OCTETS);
    let ip_u32 = u32::from_be_bytes(ip);
    if ip_u32 >= base && ip_u32 < base + (POOL_SIZE as u32) {
        Some((ip_u32 - base) as u8)
    } else {
        None
    }
}

fn pool_offset_to_ip(offset: u8) -> [u8; 4] {
    let base = u32::from_be_bytes(POOL_START_OCTETS);
    (base + offset as u32).to_be_bytes()
}

fn ip_to_u32(ip: [u8; 4]) -> u32 {
    u32::from_be_bytes(ip)
}

fn find_lease_by_mac(mac: &[u8; 6]) -> Option<usize> {
    unsafe {
        for i in 0..LEASE_TABLE_SIZE {
            if let Some(ref l) = LEASES[i] {
                if l.mac == *mac {
                    return Some(i);
                }
            }
        }
    }
    None
}

fn find_lease_by_ip(ip: [u8; 4]) -> Option<usize> {
    unsafe {
        for i in 0..LEASE_TABLE_SIZE {
            if let Some(ref l) = LEASES[i] {
                if l.ip == ip {
                    return Some(i);
                }
            }
        }
    }
    None
}

fn alloc_lease() -> Option<usize> {
    unsafe {
        for i in 0..LEASE_TABLE_SIZE {
            if LEASES[i].is_none() {
                return Some(i);
            }
        }
    }
    None
}

fn allocate_ip(mac: &[u8; 6]) -> Option<[u8; 4]> {
    if let Some(idx) = find_lease_by_mac(mac) {
        return unsafe { LEASES[idx].as_ref().map(|l| l.ip) };
    }
    let server_ip = get_server_ip();
    let server_u32 = ip_to_u32(server_ip);
    unsafe {
        for offset in 0..POOL_SIZE {
            let ip = pool_offset_to_ip(offset);
            if ip_to_u32(ip) == server_u32 {
                continue;
            }
            let mut taken = false;
            for i in 0..LEASE_TABLE_SIZE {
                if let Some(ref l) = LEASES[i] {
                    if l.ip == ip {
                        taken = true;
                        break;
                    }
                }
            }
            if !taken {
                return Some(ip);
            }
        }
    }
    None
}

fn get_server_ip() -> [u8; 4] {
    nic::get_iface(1).map(|iface| iface.ip).unwrap_or([0; 4])
}

fn get_server_subnet() -> [u8; 4] {
    nic::get_iface(1).map(|iface| iface.subnet_mask).unwrap_or([255, 255, 255, 0])
}

fn get_server_gateway() -> [u8; 4] {
    nic::get_iface(1).map(|iface| iface.gateway).unwrap_or([0; 4])
}

fn find_option(payload: &[u8], opt_type: u8) -> Option<&[u8]> {
    let mut off = 240usize;
    while off + 1 < payload.len() {
        let t = payload[off];
        if t == OPT_END {
            break;
        }
        if t == OPT_PAD {
            off += 1;
            continue;
        }
        let len = payload[off + 1] as usize;
        if off + 2 + len > payload.len() {
            break;
        }
        if t == opt_type {
            return Some(&payload[off + 2..off + 2 + len]);
        }
        off += 2 + len;
    }
    None
}

fn build_dhcp_reply(
    request: &[u8],
    msg_type: u8,
    yiaddr: [u8; 4],
    server_ip: [u8; 4],
    subnet_mask: [u8; 4],
    gateway: [u8; 4],
    lease_secs: u32,
) -> [u8; 548] {
    let mut buf = [0u8; 548];
    buf[0] = OP_REPLY;
    buf[1] = 1;
    buf[2] = 6;
    buf[3] = 0;
    buf[4..8].copy_from_slice(&request[4..8]);
    buf[8] = request[8];
    buf[9] = request[9];
    buf[10..12].copy_from_slice(&request[10..12]);
    buf[12..16].copy_from_slice(&request[12..16]);
    buf[16..20].copy_from_slice(&yiaddr);
    buf[20..24].copy_from_slice(&server_ip);
    buf[28..44].copy_from_slice(&request[28..44]);
    buf[44..108].copy_from_slice(&request[44..108]);
    buf[108..236].copy_from_slice(&request[108..236]);

    buf[236..240].copy_from_slice(&MAGIC_COOKIE);
    let mut off = 240usize;

    buf[off] = OPT_MSG_TYPE;
    buf[off + 1] = 1;
    buf[off + 2] = msg_type;
    off += 3;

    buf[off] = OPT_SERVER_ID;
    buf[off + 1] = 4;
    buf[off + 2..off + 6].copy_from_slice(&server_ip);
    off += 6;

    buf[off] = OPT_LEASE_TIME;
    buf[off + 1] = 4;
    buf[off + 2..off + 6].copy_from_slice(&lease_secs.to_be_bytes());
    off += 6;

    buf[off] = OPT_SUBNET_MASK;
    buf[off + 1] = 4;
    buf[off + 2..off + 6].copy_from_slice(&subnet_mask);
    off += 6;

    buf[off] = OPT_ROUTER;
    buf[off + 1] = 4;
    buf[off + 2..off + 6].copy_from_slice(&gateway);
    off += 6;

    let dns = [10, 0, 2, 3];
    buf[off] = OPT_DNS;
    buf[off + 1] = 4;
    buf[off + 2..off + 6].copy_from_slice(&dns);
    off += 6;

    buf[off] = OPT_END;
    off += 1;

    for i in off..548 {
        buf[i] = OPT_PAD;
    }

    buf
}

fn get_lease_secs(opt_lease_time: Option<&[u8]>) -> u32 {
    if let Some(val) = opt_lease_time {
        if val.len() >= 4 {
            let requested = u32::from_be_bytes([val[0], val[1], val[2], val[3]]);
            if requested > 0 && requested <= DEFAULT_LEASE_SECS {
                return requested;
            }
        }
    }
    DEFAULT_LEASE_SECS
}

pub fn handle_receive(iface_idx: usize, _src_ip: [u8; 4], packet: &[u8]) -> bool {
    if packet.len() < 8 {
        return false;
    }
    let (hdr, payload) = match udp::parse(packet) {
        Some(h) => h,
        None => return false,
    };

    if hdr.dst_port != DHCP_SERVER_PORT || hdr.src_port != DHCP_CLIENT_PORT {
        return false;
    }

    if payload.len() < 240 {
        return false;
    }

    let magic = [payload[236], payload[237], payload[238], payload[239]];
    if magic != MAGIC_COOKIE {
        return false;
    }

    let op = payload[0];
    if op != OP_REQUEST {
        return false;
    }

    if payload[1] != 1 || payload[2] != 6 {
        return false;
    }

    let client_mac = &payload[28..34];
    let mut mac = [0u8; 6];
    mac.copy_from_slice(client_mac);

    let msg_type = match find_option(payload, OPT_MSG_TYPE) {
        Some(val) if val.len() >= 1 => val[0],
        _ => return false,
    };

    let server_ip = get_server_ip();
    if server_ip == [0; 4] {
        return false;
    }

    match msg_type {
        MSG_DISCOVER => {
            let ip = match allocate_ip(&mac) {
                Some(ip) => ip,
                None => return false,
            };

            let lease_idx = match find_lease_by_mac(&mac) {
                Some(idx) => idx,
                None => match alloc_lease() {
                    Some(idx) => idx,
                    None => return false,
                },
            };

            let lease_secs = get_lease_secs(find_option(payload, OPT_LEASE_TIME));
            let subnet = get_server_subnet();
            let gateway = get_server_gateway();

            let reply = build_dhcp_reply(
                payload, MSG_OFFER, ip, server_ip, subnet, gateway, lease_secs,
            );

            unsafe {
                LEASES[lease_idx] = Some(Lease {
                    ip,
                    mac,
                    expires: 0,
                });
            }

            send_dhcp_udp(iface_idx, &reply, server_ip, [255; 4]);
            true
        }
        MSG_REQUEST => {
            let req_ip = match find_option(payload, OPT_REQ_IP) {
                Some(val) if val.len() >= 4 => [val[0], val[1], val[2], val[3]],
                Some(_) => return false,
                None => [payload[12], payload[13], payload[14], payload[15]],
            };

            if req_ip == [0; 4] {
                return false;
            }

            let valid = match find_lease_by_ip(req_ip) {
                Some(idx) => {
                    unsafe {
                        if let Some(ref l) = LEASES[idx] {
                            l.mac == mac
                        } else {
                            false
                        }
                    }
                }
                None => {
                    ip_to_pool_offset(req_ip).is_some() && req_ip != server_ip
                }
            };

            if valid {
                let lease_idx = match find_lease_by_mac(&mac) {
                    Some(idx) => idx,
                    None => match alloc_lease() {
                        Some(idx) => idx,
                        None => return false,
                    },
                };

                let lease_secs = get_lease_secs(find_option(payload, OPT_LEASE_TIME));
                let subnet = get_server_subnet();
                let gateway = get_server_gateway();

                let reply = build_dhcp_reply(
                    payload, MSG_ACK, req_ip, server_ip, subnet, gateway, lease_secs,
                );

                unsafe {
                    LEASES[lease_idx] = Some(Lease {
                        ip: req_ip,
                        mac,
                        expires: 0,
                    });
                }

                send_dhcp_udp(iface_idx, &reply, server_ip, [255; 4]);
                true
            } else {
                let reply = build_dhcp_reply(
                    payload, MSG_NAK, [0; 4], server_ip, [0; 4], [0; 4], 0,
                );
                send_dhcp_udp(iface_idx, &reply, server_ip, [255; 4]);
                true
            }
        }
        _ => false,
    }
}

fn send_dhcp_udp(iface_idx: usize, dhcp_msg: &[u8; 548], src_ip: [u8; 4], dst_ip: [u8; 4]) {
    let udp_len = 8 + 548;
    let mut udp_buf = [0u8; 1500];
    udp_buf[0..2].copy_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
    udp_buf[2..4].copy_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
    udp_buf[4..6].copy_from_slice(&(udp_len as u16).to_be_bytes());
    udp_buf[8..udp_len].copy_from_slice(dhcp_msg);

    let total_len = 20 + udp_len;
    let mut ip_pkt = [0u8; 1500];
    ip_pkt[0] = 0x45;
    ip_pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    ip_pkt[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip_pkt[8] = 64;
    ip_pkt[9] = crate::ipv4::PROTO_UDP;
    ip_pkt[12..16].copy_from_slice(&src_ip);
    ip_pkt[16..20].copy_from_slice(&dst_ip);
    ip_pkt[20..total_len].copy_from_slice(&udp_buf[..udp_len]);

    let csum = crate::ipv4::internet_checksum(&ip_pkt[..20]);
    ip_pkt[10] = (csum >> 8) as u8;
    ip_pkt[11] = (csum & 0xFF) as u8;

    nic::send_broadcast_packet(iface_idx, &ip_pkt[..total_len]);
}

pub fn lease_count() -> usize {
    unsafe {
        let mut count = 0;
        for i in 0..LEASE_TABLE_SIZE {
            if LEASES[i].is_some() {
                count += 1;
            }
        }
        count
    }
}

pub fn print_leases(write_str: &mut dyn FnMut(&str)) {
    unsafe {
        for i in 0..LEASE_TABLE_SIZE {
            if let Some(ref l) = LEASES[i] {
                write_str("  ");
                print_ip(write_str, l.ip);
                write_str("  ");
                print_mac(write_str, &l.mac);
                write_str("\r\n");
            }
        }
    }
}

fn print_ip(write_str: &mut dyn FnMut(&str), ip: [u8; 4]) {
    let mut buf = [0u8; 16];
    let mut pos = 0;
    for j in 0..4 {
        if j > 0 { buf[pos] = b'.'; pos += 1; }
        let n = ip[j] as u32;
        if n >= 100 { buf[pos] = b'0' + (n / 100) as u8; pos += 1; }
        if n >= 10 { buf[pos] = b'0' + ((n / 10) % 10) as u8; pos += 1; }
        buf[pos] = b'0' + (n % 10) as u8; pos += 1;
    }
    if let Ok(s) = core::str::from_utf8(&buf[..pos]) {
        write_str(s);
    }
}

fn print_mac(write_str: &mut dyn FnMut(&str), mac: &[u8; 6]) {
    let hex = b"0123456789abcdef";
    let mut buf = [0u8; 17];
    for j in 0..6 {
        if j > 0 { buf[j * 3 - 1] = b':'; }
        buf[j * 3] = hex[(mac[j] >> 4) as usize];
        buf[j * 3 + 1] = hex[(mac[j] & 0xF) as usize];
    }
    if let Ok(s) = core::str::from_utf8(&buf) {
        write_str(s);
    }
}
