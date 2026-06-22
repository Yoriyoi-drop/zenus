pub const PROTO_TCP: u8 = 6;
pub const PROTO_UDP: u8 = 17;
pub const PROTO_ICMP: u8 = 1;

pub struct Ipv4Header {
    pub version_ihl: u8,
    pub dscp_ecn: u8,
    pub total_length: u16,
    pub identification: u16,
    pub flags_fragment: u16,
    pub ttl: u8,
    pub protocol: u8,
    pub checksum: u16,
    pub src_ip: [u8; 4],
    pub dst_ip: [u8; 4],
}

pub fn parse(packet: &[u8]) -> Option<(Ipv4Header, &[u8])> {
    if packet.len() < 20 {
        return None;
    }
    let ptr = packet.as_ptr();

    let version_ihl = unsafe { *ptr };
    let ihl = ((version_ihl & 0x0F) * 4) as usize;

    if packet.len() < ihl {
        return None;
    }

    let total_length = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(2) as *const u16) }) as usize;

    if total_length < ihl || total_length > packet.len() {
        return None;
    }

    let stored_csum = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(10) as *const u16) });
    if stored_csum != 0 {
        let mut csum_buf = [0u8; 20];
        csum_buf.copy_from_slice(&packet[..ihl.min(20)]);
        csum_buf[10] = 0;
        csum_buf[11] = 0;
        let calc_csum = internet_checksum(&csum_buf[..ihl.min(20)]);
        if calc_csum != stored_csum {
            return None;
        }
    }

    let header = Ipv4Header {
        version_ihl,
        dscp_ecn: unsafe { *ptr.add(1) },
        total_length: total_length as u16,
        identification: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(4) as *const u16) }),
        flags_fragment: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(6) as *const u16) }),
        ttl: unsafe { *ptr.add(8) },
        protocol: unsafe { *ptr.add(9) },
        checksum: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(10) as *const u16) }),
        src_ip: unsafe { core::ptr::read_unaligned(ptr.add(12) as *const [u8; 4]) },
        dst_ip: unsafe { core::ptr::read_unaligned(ptr.add(16) as *const [u8; 4]) },
    };

    let payload = &packet[ihl..total_length];
    Some((header, payload))
}

pub fn internet_checksum(data: &[u8]) -> u16 {
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

pub fn send_raw(iface_idx: usize, src_ip: [u8; 4], dst_ip: [u8; 4], protocol: u8, payload: &[u8]) -> bool {
    let iface = match crate::nic::get_iface(iface_idx) {
        Some(iface) => iface,
        None => return false,
    };
    if iface.nic_type == crate::nic::NicType::Loopback {
        return false;
    }

    let total_len = 20 + payload.len();
    if total_len > 1500 {
        return false;
    }

    let mut ip_pkt = [0u8; 1500];
    ip_pkt[0] = 0x45;
    ip_pkt[1] = 0;
    ip_pkt[2..4].copy_from_slice(&(total_len as u16).to_be_bytes());
    ip_pkt[6..8].copy_from_slice(&0x4000u16.to_be_bytes());
    ip_pkt[8] = 64;
    ip_pkt[9] = protocol;
    ip_pkt[12..16].copy_from_slice(&src_ip);
    ip_pkt[16..20].copy_from_slice(&dst_ip);
    ip_pkt[20..20 + payload.len()].copy_from_slice(payload);

    let csum = internet_checksum(&ip_pkt[..20]);
    ip_pkt[10] = (csum >> 8) as u8;
    ip_pkt[11] = (csum & 0xFF) as u8;

    crate::nic::send_packet(iface_idx, &ip_pkt[..total_len])
}

pub fn send(iface_idx: usize, dst_ip: [u8; 4], protocol: u8, payload: &[u8]) -> bool {
    let iface = match crate::nic::get_iface(iface_idx) {
        Some(iface) => iface,
        None => return false,
    };
    send_raw(iface_idx, iface.ip, dst_ip, protocol, payload)
}
