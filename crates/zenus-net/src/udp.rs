use crate::ipv4;

pub struct UdpHeader {
    pub src_port: u16,
    pub dst_port: u16,
    pub length: u16,
    pub checksum: u16,
}

fn checksum(src_ip: [u8; 4], dst_ip: [u8; 4], segment: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    sum += u32::from(u16::from_be_bytes([src_ip[0], src_ip[1]]));
    sum += u32::from(u16::from_be_bytes([src_ip[2], src_ip[3]]));
    sum += u32::from(u16::from_be_bytes([dst_ip[0], dst_ip[1]]));
    sum += u32::from(u16::from_be_bytes([dst_ip[2], dst_ip[3]]));
    sum += 0x0011;
    let udp_len = segment.len() as u16;
    sum += u32::from(udp_len);
    let mut i = 0;
    while i + 1 < segment.len() {
        sum += u32::from(u16::from_be_bytes([segment[i], segment[i + 1]]));
        i += 2;
    }
    if i < segment.len() {
        sum += u32::from(segment[i]) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xFFFF) + (sum >> 16);
    }
    !(sum as u16)
}

pub fn parse(packet: &[u8]) -> Option<(UdpHeader, &[u8])> {
    if packet.len() < 8 {
        return None;
    }
    let ptr = packet.as_ptr();

    let length = u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(4) as *const u16) });
    // The header's own length field is the authority on where the datagram
    // ends. It used to be parsed into `UdpHeader::length` and then never read
    // again: the payload was `&packet[8..]`, i.e. everything IPv4 handed us,
    // and the checksum was verified over that same over-long span. Two
    // consequences, both from one packet: bytes past the datagram were treated
    // as payload (so a DHCP or DNS handler saw trailing garbage), and the
    // checksum was computed over the wrong length, so a *corrupt* datagram
    // verified clean exactly when there was junk after it.
    if (length as usize) < 8 || (length as usize) > packet.len() {
        return None;
    }
    let datagram = &packet[..length as usize];

    let header = UdpHeader {
        src_port: u16::from_be(unsafe { core::ptr::read_unaligned(ptr as *const u16) }),
        dst_port: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(2) as *const u16) }),
        length,
        checksum: u16::from_be(unsafe { core::ptr::read_unaligned(ptr.add(6) as *const u16) }),
    };

    let udp_payload = &datagram[8..];
    Some((header, udp_payload))
}

pub fn send(
    iface_idx: usize,
    src_port: u16,
    dst_port: u16,
    src_ip: [u8; 4],
    dst_ip: [u8; 4],
    payload: &[u8],
) -> bool {
    let total_len = 8 + payload.len();
    if total_len > 1500 {
        return false;
    }
    let mut buf = [0u8; 1500];
    buf[0..2].copy_from_slice(&src_port.to_be_bytes());
    buf[2..4].copy_from_slice(&dst_port.to_be_bytes());
    buf[4..6].copy_from_slice(&(total_len as u16).to_be_bytes());
    buf[8..total_len].copy_from_slice(payload);
    ipv4::send_raw(
        iface_idx,
        src_ip,
        dst_ip,
        ipv4::PROTO_UDP,
        &buf[..total_len],
    )
}

pub fn handle_receive(iface_idx: usize, src_ip: [u8; 4], dst_ip: [u8; 4], packet: &[u8]) -> bool {
    if packet.len() < 8 {
        return false;
    }

    let (hdr, payload) = match parse(packet) {
        Some(h) => h,
        None => return false,
    };

    // Over the datagram only — `&packet[..hdr.length as usize]`, not whatever
    // IPv4 passed in. The pseudo-header's length field has to agree with the
    // bytes actually summed, and the two used to disagree whenever a datagram
    // was followed by padding.
    if hdr.checksum != 0 && checksum(src_ip, dst_ip, &packet[..hdr.length as usize]) != 0 {
        return false;
    }

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

#[cfg(test)]
mod host_tests {
    use super::parse;
    use alloc::vec;
    use alloc::vec::Vec;

    /// Build a UDP datagram with a correct checksum over `payload`.
    fn datagram(src_port: u16, dst_port: u16, payload: &[u8]) -> Vec<u8> {
        let mut pkt = vec![0u8; 8 + payload.len()];
        pkt[0..2].copy_from_slice(&src_port.to_be_bytes());
        pkt[2..4].copy_from_slice(&dst_port.to_be_bytes());
        pkt[4..6].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
        pkt[8..].copy_from_slice(payload);
        pkt
    }

    /// Regression: `UdpHeader::length` was parsed and then never read. The
    /// payload was `&packet[8..]` — everything IPv4 passed up, including any
    /// padding — so a DHCP or DNS handler saw bytes that were not part of the
    /// datagram, and the checksum was verified over that same over-long span.
    #[test]
    fn the_length_field_bounds_the_payload() {
        let payload = b"hello";
        let mut pkt = datagram(1234, 53, payload);
        assert_eq!(pkt.len(), 13);

        // Append 8 bytes of trailing junk, as IPv4 padding or a sloppy sender
        // would. The datagram still says 13 bytes, so the payload must still be
        // 5 bytes and the junk must not be visible.
        pkt.extend_from_slice(&[0xAA; 8]);
        let (hdr, got) = parse(&pkt).expect("the datagram itself is well formed");
        assert_eq!(hdr.length as usize, pkt.len() - 8, "the field is not ignored");
        assert_eq!(got, b"hello", "payload must stop at the length field");
    }

    /// A length field that disagrees with the buffer is a malformed packet, not
    /// something to guess about. Both directions are refused.
    #[test]
    fn an_impossible_length_field_is_refused() {
        let payload = b"hello";
        let good = datagram(1234, 53, payload);

        // Longer than the buffer: a truncated capture.
        let mut too_long = good.clone();
        too_long[4..6].copy_from_slice(&4096u16.to_be_bytes());
        assert!(parse(&too_long).is_none(), "length past the buffer");

        // Shorter than the fixed header: the payload cannot be negative.
        let mut too_short = good.clone();
        too_short[4..6].copy_from_slice(&4u16.to_be_bytes());
        assert!(parse(&too_short).is_none(), "length below the header size");

        let mut zero = good.clone();
        zero[4..6].copy_from_slice(&0u16.to_be_bytes());
        assert!(parse(&zero).is_none(), "length 0");

        // Exactly the header, with no payload at all: legal, empty datagram.
        let mut empty = good.clone();
        empty[4..6].copy_from_slice(&8u16.to_be_bytes());
        empty.truncate(8);
        let (hdr, got) = parse(&empty).expect("an empty datagram is valid");
        assert_eq!(hdr.length, 8);
        assert!(got.is_empty());

        // The honest one still parses.
        assert_eq!(parse(&good).expect("valid").1, payload);
    }

    /// Truncation must be caught rather than read past. A datagram whose length
    /// field promises more than arrived is the shape of a truncated capture.
    #[test]
    fn short_buffers_are_refused() {
        assert!(parse(&[]).is_none());
        assert!(parse(&[0u8; 7]).is_none());
        let full = datagram(1, 2, b"abcd");
        for len in 0..full.len() {
            assert!(
                parse(&full[..len]).is_none(),
                "{len} bytes must not parse"
            );
        }
    }
}
