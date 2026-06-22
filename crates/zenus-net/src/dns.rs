use crate::udp;
use crate::nic;

const DNS_PORT: u16 = 53;
const QR_RESPONSE: u16 = 0x8000;
const RD_FLAG: u16 = 0x0100;
const RCODE_NXDOMAIN: u16 = 0x0003;
const QTYPE_A: u16 = 1;
const QCLASS_IN: u16 = 1;

static mut RESP_BUF: [u8; 1500] = [0; 1500];
static mut RESP_LEN: usize = 0;
static mut RESP_READY: bool = false;
static mut EXPECTED_ID: u16 = 0;
static mut ACTIVE_PORT: u16 = 0;

pub fn active_port() -> u16 {
    unsafe { ACTIVE_PORT }
}

fn dns_id() -> u16 {
    unsafe {
        let id = core::ptr::read_volatile(&EXPECTED_ID);
        let next = id.wrapping_add(1);
        core::ptr::write_volatile(&mut EXPECTED_ID, next);
        id
    }
}

fn encode_name(name: &str, buf: &mut [u8]) -> Option<usize> {
    let bytes = name.as_bytes();
    let mut pos = 0;
    let mut i = 0;
    while i <= bytes.len() {
        let end = if i == bytes.len() {
            bytes.len()
        } else {
            match bytes[i..].iter().position(|&b| b == b'.') {
                Some(len) => i + len,
                None => bytes.len(),
            }
        };
        let label_len = end - i;
        if label_len > 63 || pos + label_len + 1 > buf.len() {
            return None;
        }
        buf[pos] = label_len as u8;
        pos += 1;
        buf[pos..pos + label_len].copy_from_slice(&bytes[i..end]);
        pos += label_len;
        if end >= bytes.len() || bytes[end] == b'.' && end + 1 >= bytes.len() {
            break;
        }
        i = end + 1;
    }
    if pos + 1 > buf.len() {
        return None;
    }
    buf[pos] = 0;
    Some(pos + 1)
}

fn build_query(id: u16, name: &str, buf: &mut [u8]) -> Option<usize> {
    let name_len = encode_name(name, &mut buf[12..])?;
    let qlen = 12 + name_len;
    if qlen + 4 > buf.len() {
        return None;
    }
    buf[0..2].copy_from_slice(&id.to_be_bytes());
    buf[2..4].copy_from_slice(&RD_FLAG.to_be_bytes());
    buf[4..6].copy_from_slice(&1u16.to_be_bytes());
    buf[6..8].copy_from_slice(&0u16.to_be_bytes());
    buf[8..10].copy_from_slice(&0u16.to_be_bytes());
    buf[10..12].copy_from_slice(&0u16.to_be_bytes());
    let off = 12 + name_len;
    buf[off..off + 2].copy_from_slice(&QTYPE_A.to_be_bytes());
    buf[off + 2..off + 4].copy_from_slice(&QCLASS_IN.to_be_bytes());
    Some(off + 4)
}

pub fn handle_receive(_iface_idx: usize, _src_ip: [u8; 4], packet: &[u8]) -> bool {
    if packet.len() < 8 {
        return false;
    }
    let (_hdr, payload) = match udp::parse(packet) {
        Some(h) => h,
        None => return false,
    };

    if payload.len() < 12 {
        return false;
    }

    let resp_id = u16::from_be_bytes([payload[0], payload[1]]);
    let resp_len = core::cmp::min(payload.len(), unsafe { RESP_BUF.len() });
    unsafe {
        if resp_id == EXPECTED_ID {
            RESP_BUF[..resp_len].copy_from_slice(&payload[..resp_len]);
            RESP_LEN = resp_len;
            RESP_READY = true;
        }
    }
    true
}

fn parse_response(buf: &[u8]) -> Option<[u8; 4]> {
    if buf.len() < 12 {
        return None;
    }
    let flags = u16::from_be_bytes([buf[2], buf[3]]);
    if flags & QR_RESPONSE == 0 {
        return None;
    }
    let rcode = flags & 0x000F;
    if rcode == RCODE_NXDOMAIN {
        return None;
    }
    if rcode != 0 {
        return None;
    }

    let qdcount = u16::from_be_bytes([buf[4], buf[5]]);
    let ancount = u16::from_be_bytes([buf[6], buf[7]]);
    if ancount == 0 {
        return None;
    }

    let mut off = 12usize;

    for _ in 0..qdcount {
        while off < buf.len() {
            let b = buf[off];
            if b == 0 {
                off += 1;
                break;
            }
            if b & 0xC0 == 0xC0 {
                off += 2;
                break;
            }
            off += 1 + b as usize;
        }
        off += 4;
    }

    for _ in 0..ancount {
        let mut depth = 0usize;
        while off < buf.len() && depth < 16 {
            let b = buf[off];
            if b == 0 {
                off += 1;
                break;
            }
            if b & 0xC0 == 0xC0 {
                if depth > 0 {
                    return None;
                }
                let ptr = (u16::from_be_bytes([buf[off] & 0x3F, buf[off + 1]])) as usize;
                if ptr >= buf.len() {
                    return None;
                }
                off = ptr;
                depth += 1;
                continue;
            }
            off += 1 + b as usize;
            if off >= buf.len() {
                return None;
            }
        }
        if off + 10 > buf.len() {
            return None;
        }
        let rtype = u16::from_be_bytes([buf[off], buf[off + 1]]);
        let rdlength = u16::from_be_bytes([buf[off + 8], buf[off + 9]]) as usize;
        off += 10;
        if rtype == QTYPE_A && rdlength >= 4 && off + 4 <= buf.len() {
            return Some([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]]);
        }
        off += rdlength;
    }
    None
}

pub fn resolve(iface_idx: usize, dns_server: [u8; 4], domain: &str) -> Option<[u8; 4]> {
    unsafe { RESP_READY = false; }

    let iface = nic::get_iface(iface_idx)?;
    let src_ip = iface.ip;

    let id = dns_id();
    let src_port = 12345;
    unsafe {
        EXPECTED_ID = id;
        ACTIVE_PORT = src_port;
    }

    let mut query = [0u8; 512];
    let qlen = build_query(id, domain, &mut query)?;

    udp::send(iface_idx, src_port, DNS_PORT, src_ip, dns_server, &query[..qlen]);

    for tick in 0..50000 {
        if tick > 0 && tick % 5000 == 0 {
            udp::send(iface_idx, src_port, DNS_PORT, src_ip, dns_server, &query[..qlen]);
        }
        nic::net_poll();
        unsafe {
            if RESP_READY {
                if let Some(ip) = parse_response(&RESP_BUF[..RESP_LEN]) {
                    ACTIVE_PORT = 0;
                    return Some(ip);
                }
            }
        }
    }

    unsafe { ACTIVE_PORT = 0; }
    None
}
