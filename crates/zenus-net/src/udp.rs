use crate::ipv4;

pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

pub fn parse(packet: &[u8]) -> Option<(UdpHeader, &[u8])> {
    if packet.len() < 8 {
        return None;
    }
    let ptr = packet.as_ptr();

    let header = UdpHeader {
        src_port: u16::from_be(unsafe { core::ptr::read_unaligned(ptr as *const u16) }),
        dst_port: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(2) as *const u16) }),
        length: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(4) as *const u16) }),
        checksum: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(6) as *const u16) }),
    };

    let udp_payload = &packet[8..];
    Some((header, udp_payload))
}

pub fn send(iface_idx: usize, src_port: u16, dst_port: u16, src_ip: [u8; 4], dst_ip: [u8; 4], payload: &[u8]) -> bool {
    let total_len = 8 + payload.len();
    if total_len > 1500 {
        return false;
    }
    let mut buf = [0u8; 1500];
    buf[0..2].copy_from_slice(&src_port.to_be_bytes());
    buf[2..4].copy_from_slice(&dst_port.to_be_bytes());
    buf[4..6].copy_from_slice(&(total_len as u16).to_be_bytes());
    buf[8..total_len].copy_from_slice(payload);
    ipv4::send_raw(iface_idx, src_ip, dst_ip, ipv4::PROTO_UDP, &buf[..total_len])
}

pub fn handle_receive(
    iface_idx: usize,
    src_ip: [u8; 4], _dst_ip: [u8; 4],
    packet: &[u8],
) -> bool {
    if packet.len() < 8 {
        return false;
    }

    let (hdr, payload) = match parse(packet) {
        Some(h) => h,
        None => return false,
    };

    if hdr.dst_port == 7 {
        let total_len = 8 + payload.len();
        if total_len > 1500 {
            return false;
        }

        let mut resp = [0u8; 1500];
        resp[0..2].copy_from_slice(&hdr.dst_port.to_be_bytes());
        resp[2..4].copy_from_slice(&hdr.src_port.to_be_bytes());
        resp[4..6].copy_from_slice(&(total_len as u16).to_be_bytes());
        resp[8..total_len].copy_from_slice(payload);

        ipv4::send(iface_idx, src_ip, ipv4::PROTO_UDP, &resp[..total_len])
    } else if hdr.dst_port == 67 {
        crate::dhcp_server::handle_receive(iface_idx, src_ip, packet)
    } else if hdr.dst_port == 68 {
        crate::dhcp::handle_receive(iface_idx, src_ip, packet)
    } else if hdr.dst_port == crate::dns::active_port() || hdr.dst_port == 53 {
        crate::dns::handle_receive(iface_idx, src_ip, packet)
    } else if crate::socket::udp_enqueue(hdr.dst_port, src_ip, hdr.src_port, packet) {
        true
    } else {
        false
    }
}
