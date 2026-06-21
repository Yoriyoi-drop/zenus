use crate::ipv4;

const ICMP_ECHO_REQUEST: u8 = 8;
const ICMP_ECHO_REPLY: u8 = 0;

fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    let mut i = 0;

    while i + 1 < data.len() {
        sum += u32::from(u16::from_be_bytes([data[i], data[i + 1]]));
        i += 2;
    }

    if i < data.len() {
        sum += u32::from(data[i]) << 8;
    }

    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }

    !(sum as u16)
}

fn ip_checksum(header: &[u8]) -> u16 {
    checksum(header)
}

pub fn handle_echo(
    ip_hdr: &ipv4::Ipv4Header,
    payload: &[u8],
    our_mac: &[u8; 6],
    dst_mac: &[u8; 6],
    our_ip: &[u8; 4],
) -> Option<[u8; 1024]> {
    if payload.len() < 8 {
        return None;
    }

    let ptr = payload.as_ptr();
    let icmp_type = unsafe { *ptr };
    let _code = unsafe { *ptr.add(1) };
    let ident = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(4) as *const u16) });
    let seq = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(6) as *const u16) });

    if icmp_type != ICMP_ECHO_REQUEST {
        return None;
    }

    let echo_data = &payload[8..];
    let data_len = echo_data.len();

    let total_reply_len = 14 + 20 + 8 + data_len;
    if total_reply_len > 1024 {
        return None;
    }

    let mut buf = [0u8; 1024];

    buf[0..6].copy_from_slice(dst_mac);
    buf[6..12].copy_from_slice(our_mac);
    buf[12..14].copy_from_slice(&crate::ethernet::ETH_IPV4.to_be_bytes());

    let ip_total_len = (20 + 8 + data_len) as u16;

    let mut off = 14;
    buf[off] = 0x45; off += 1;
    buf[off] = 0; off += 1;
    buf[off..off + 2].copy_from_slice(&ip_total_len.to_be_bytes()); off += 2;
    buf[off..off + 2].copy_from_slice(&[0, 0]); off += 2;
    buf[off..off + 2].copy_from_slice(&[0, 0]); off += 2;
    buf[off] = 64; off += 1;
    buf[off] = ipv4::PROTO_ICMP; off += 1;
    buf[off..off + 2].copy_from_slice(&[0, 0]); off += 2;
    buf[off..off + 4].copy_from_slice(our_ip); off += 4;
    buf[off..off + 4].copy_from_slice(&ip_hdr.src_ip);

    let ip_csum = ip_checksum(&buf[14..34]);
    buf[24] = (ip_csum >> 8) as u8;
    buf[25] = (ip_csum & 0xFF) as u8;

    let icmp_off = 34;
    buf[icmp_off] = ICMP_ECHO_REPLY;
    buf[icmp_off + 1] = 0;
    buf[icmp_off + 2] = 0;
    buf[icmp_off + 3] = 0;
    buf[icmp_off + 4..icmp_off + 6].copy_from_slice(&ident.to_be_bytes());
    buf[icmp_off + 6..icmp_off + 8].copy_from_slice(&seq.to_be_bytes());
    buf[icmp_off + 8..icmp_off + 8 + data_len].copy_from_slice(echo_data);

    let icmp_csum = checksum(&buf[icmp_off..icmp_off + 8 + data_len]);
    buf[icmp_off + 2] = (icmp_csum >> 8) as u8;
    buf[icmp_off + 3] = (icmp_csum & 0xFF) as u8;

    Some(buf)
}
