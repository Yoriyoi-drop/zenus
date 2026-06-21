pub const ETH_IPV4: u16 = 0x0800;
pub const ETH_ARP: u16 = 0x0806;
pub const ETH_IPV6: u16 = 0x86DD;

pub struct EthernetHeader {
    pub dst_mac: [u8; 6],
    pub src_mac: [u8; 6],
    pub ether_type: u16,
}

pub fn parse(packet: &[u8]) -> Option<(EthernetHeader, &[u8])> {
    if packet.len() < 14 {
        return None;
    }
    let ptr = packet.as_ptr();
    let dst_mac = unsafe { core::ptr::read_unaligned(ptr as *const [u8; 6]) };
    let src_mac = unsafe { core::ptr::read_unaligned(ptr.add(6) as *const [u8; 6]) };
    let ether_type = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(12) as *const u16) });

    let header = EthernetHeader {
        dst_mac,
        src_mac,
        ether_type,
    };
    let payload = &packet[14..];
    Some((header, payload))
}
