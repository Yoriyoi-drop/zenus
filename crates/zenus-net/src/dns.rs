use crate::nic;
use crate::udp;
use zenus_sync::spinlock::SpinLock;

const DNS_PORT: u16 = 53;
const QR_RESPONSE: u16 = 0x8000;
const RD_FLAG: u16 = 0x0100;
const RCODE_NXDOMAIN: u16 = 0x0003;
const QTYPE_A: u16 = 1;
const QCLASS_IN: u16 = 1;

struct DnsState {
    resp_buf: [u8; 1500],
    resp_len: usize,
    resp_ready: bool,
    expected_id: u16,
    active_port: u16,
}

static DNS_STATE: SpinLock<DnsState> = SpinLock::new(DnsState {
    resp_buf: [0; 1500],
    resp_len: 0,
    resp_ready: false,
    expected_id: 0,
    active_port: 0,
});

pub fn active_port() -> u16 {
    DNS_STATE.lock().active_port
}

fn dns_id() -> u16 {
    let mut state = DNS_STATE.lock();
    let id = state.expected_id;
    state.expected_id = state.expected_id.wrapping_add(1);
    id
}

fn encode_name(name: &str, buf: &mut [u8]) -> Option<usize> {
    let bytes = name.as_bytes();
    // An empty name is the root label: a single zero byte. Without this early
    // return the loop below emits *two* zero bytes, i.e. a stray empty label
    // after the root, which is a malformed question section.
    if bytes.is_empty() {
        if buf.is_empty() {
            return None;
        }
        buf[0] = 0;
        return Some(1);
    }
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
        if label_len == 0 {
            // An empty interior label ("a..b", "a." at the end) is not a legal
            // domain name and used to be encoded as a zero-length label, which
            // decodes as the root and silently truncates the name.
            return None;
        }
        if label_len > 63 {
            return None;
        }
        // RFC 1035: a name is at most 255 octets including the length bytes.
        if pos + label_len + 1 > buf.len() || pos + label_len + 1 > 255 {
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

/// Host-side unit tests for DNS name encoding and query assembly.
///
/// `resolve`/`handle_receive` need the NIC, so only the wire-format helpers are
/// covered here.
#[cfg(test)]
mod host_tests {
    use super::{build_query, encode_name, QCLASS_IN, QTYPE_A, RD_FLAG};
    use alloc::format;
    use alloc::string::String;
    use alloc::vec;
    use alloc::vec::Vec;

    #[test]
    fn encode_name_produces_length_prefixed_labels() {
        let mut buf = [0u8; 64];
        let len = encode_name("zenus.local", &mut buf).expect("encodes");
        // 1+5 (zenus) + 1+5 (local) + 1 (root) = 13
        assert_eq!(len, 13);
        assert_eq!(
            &buf[..len],
            &[5, b'z', b'e', b'n', b'u', b's', 5, b'l', b'o', b'c', b'a', b'l', 0]
        );
    }

    #[test]
    fn encode_name_handles_single_label_and_empty() {
        let mut buf = [0u8; 32];
        assert_eq!(encode_name("root", &mut buf), Some(6));
        assert_eq!(&buf[..6], &[4, b'r', b'o', b'o', b't', 0]);

        assert_eq!(encode_name("", &mut buf), Some(1));
        assert_eq!(buf[0], 0, "empty name is just the root label");
    }

    #[test]
    fn encode_name_rejects_oversized_labels() {
        let mut buf = [0u8; 512];
        let long_label = "x".repeat(64);
        assert_eq!(encode_name(&long_label, &mut buf), None, "label > 63 octets");

        // 63 is the limit and must still fit.
        let ok_label = "y".repeat(63);
        assert!(encode_name(&ok_label, &mut buf).is_some());

        // Many labels that do not fit the output buffer.
        let mut too_long = String::new();
        for _ in 0..100 {
            too_long.push_str("abcdefgh.");
        }
        assert_eq!(encode_name(&too_long, &mut buf), None);
    }

    #[test]
    fn build_query_lays_out_header_and_question() {
        let mut buf = [0u8; 512];
        let len = build_query(0xBEEF, "zenus", &mut buf).expect("query builds");

        assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), 0xBEEF);
        assert_eq!(u16::from_be_bytes([buf[2], buf[3]]), RD_FLAG);
        assert_eq!(u16::from_be_bytes([buf[4], buf[5]]), 1, "QDCOUNT = 1");
        assert_eq!(u16::from_be_bytes([buf[6], buf[7]]), 0, "ANCOUNT = 0");
        assert_eq!(u16::from_be_bytes([buf[8], buf[9]]), 0, "NSCOUNT = 0");
        assert_eq!(u16::from_be_bytes([buf[10], buf[11]]), 0, "ARCOUNT = 0");

        // Question section starts at offset 12: name, then QTYPE/QCLASS.
        assert_eq!(&buf[12..18], &[5, b'z', b'e', b'n', b'u', b's']);
        assert_eq!(buf[18], 0, "root label terminator");
        assert_eq!(u16::from_be_bytes([buf[19], buf[20]]), QTYPE_A);
        assert_eq!(u16::from_be_bytes([buf[21], buf[22]]), QCLASS_IN);
        assert_eq!(len, 23);
    }

    /// Regression: `encode_name` accepted empty interior labels ("a..b") and
    /// encoded them as a zero-length label, which decodes as the root and
    /// silently truncates the name being queried.
    #[test]
    fn encode_name_rejects_empty_interior_labels() {
        let mut buf = [0u8; 256];
        assert!(encode_name("a..b", &mut buf).is_none());
        assert!(encode_name("..", &mut buf).is_none());
        assert!(
            encode_name("a.", &mut buf).is_some(),
            "a single trailing dot is legal"
        );
    }

    /// Regression: nothing bounded the total encoded length, so a caller could
    /// build a name far past the 255-octet limit of RFC 1035.
    #[test]
    fn encode_name_respects_the_255_octet_limit() {
        let mut buf = [0u8; 512];
        // 4 labels of 63 octets = 4 * (1 + 63) + 1 = 257 > 255.
        let long = format!("{}.{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63), "d".repeat(63));
        assert!(encode_name(&long, &mut buf).is_none(), "257 octets must be refused");

        // Three labels of 63 = 3 * 64 + 1 = 193, which fits.
        let ok = format!("{}.{}.{}", "a".repeat(63), "b".repeat(63), "c".repeat(63));
        assert!(encode_name(&ok, &mut buf).is_some());
    }

    /// A minimal, valid A-record response: header, one question, one answer.
    fn a_record_response() -> Vec<u8> {
        let mut buf = vec![
            0x12, 0x34, // id
            0x81, 0x80, // QR + RD + RA, rcode 0
            0x00, 0x01, // QDCOUNT
            0x00, 0x01, // ANCOUNT
            0x00, 0x00, // NSCOUNT
            0x00, 0x00, // ARCOUNT
        ];
        // QNAME "a" + root (3), QTYPE A (2), QCLASS IN (2) -> 7 bytes,
        // so the answer section starts at 12 + 7 = 19.
        buf.extend_from_slice(&[1, b'a', 0, 0x00, 0x01, 0x00, 0x01]);
        // ANAME (inline "b"), then TYPE A, CLASS IN, TTL 60, RDLENGTH 4.
        buf.extend_from_slice(&[1, b'b', 0]);
        buf.extend_from_slice(&[0x00, 0x01, 0x00, 0x01, 0, 0, 0, 60, 0x00, 0x04]);
        buf.extend_from_slice(&[10, 0, 0, 7]);
        buf
    }

    #[test]
    fn parse_response_extracts_the_a_record() {
        assert_eq!(super::parse_response(&a_record_response()), Some([10, 0, 0, 7]));
    }

    /// Regression: `buf[off + 1]` for a compression pointer was read after only
    /// checking `off < buf.len()`, so a response ending in `0xC0` read one byte
    /// past the buffer and panicked the resolver.
    #[test]
    fn compression_pointer_at_end_of_buffer_is_rejected() {
        // Answer name would start at 19; leave exactly one byte there so the
        // pointer's *second* octet is missing — the exact input that made the
        // old `buf[off + 1]` read one byte past the slice.
        let mut resp = a_record_response();
        resp.truncate(19);
        resp.push(0xC0);
        assert_eq!(resp.len(), 20);
        assert_eq!(super::parse_response(&resp), None);
    }

    /// Every truncation of a valid response must be rejected, never panic and
    /// never invent an address.
    #[test]
    fn truncated_responses_are_rejected_at_every_length() {
        let full = a_record_response();
        for len in 0..full.len() {
            let prefix = &full[..len];
            assert_eq!(
                super::parse_response(prefix),
                None,
                "prefix of {len} bytes was accepted"
            );
        }
    }

    #[test]
    fn parse_response_rejects_wrong_header_shape() {
        let mut resp = a_record_response();
        resp[2] &= 0x7F; // clear QR -> this is a query, not a response
        assert_eq!(super::parse_response(&resp), None);

        let mut resp = a_record_response();
        resp[3] = 0x83; // rcode 3 = NXDOMAIN
        assert_eq!(super::parse_response(&resp), None);

        let mut resp = a_record_response();
        resp[7] = 0x00; // ANCOUNT = 0
        assert_eq!(super::parse_response(&resp), None);

        assert_eq!(super::parse_response(&[]), None);
        assert_eq!(super::parse_response(&[0u8; 11]), None);
    }

    #[test]
    fn self_referencing_compression_pointer_is_rejected() {
        // Point the answer name at itself: the parser must give up through the
        // pointer-depth guard instead of looping.
        let mut resp = a_record_response();
        resp[19] = 0xC0;
        resp[20] = 0x13; // offset 19
        assert_eq!(super::parse_response(&resp), None);
    }

    #[test]
    fn out_of_bounds_compression_pointer_is_rejected() {
        let mut resp = a_record_response();
        resp[19] = 0xC0;
        resp[20] = 0xF0; // offset 240, past the end
        assert_eq!(super::parse_response(&resp), None);
    }

    #[test]
    fn build_query_fails_when_buffer_is_too_small() {
        let mut small = [0u8; 16];
        assert!(build_query(1, "zenus.local", &mut small).is_none());

        // Exactly enough: 12-byte header + 6-byte name ("abcd" -> 1+4+1) + 4.
        let mut exact = [0u8; 22];
        assert_eq!(build_query(1, "abcd", &mut exact), Some(22));

        let mut one_short = [0u8; 21];
        assert!(build_query(1, "abcd", &mut one_short).is_none());
    }
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
    let mut state = DNS_STATE.lock();
    if resp_id == state.expected_id {
        let resp_len = core::cmp::min(payload.len(), state.resp_buf.len());
        state.resp_buf[..resp_len].copy_from_slice(&payload[..resp_len]);
        state.resp_len = resp_len;
        state.resp_ready = true;
    }
    true
}

/// Read a big-endian u16 at `off`, or `None` if the slice ends first.
///
/// Every field read in `parse_response` goes through this. The parser walks a
/// network-supplied buffer, and the old code indexed `buf[off + 1]` after only
/// checking `off < buf.len()` — a response whose last byte is `0xC0` read one
/// byte past the end and panicked the resolver in the middle of packet
/// processing.
fn be16_at(buf: &[u8], off: usize) -> Option<u16> {
    let bytes = buf.get(off..off.checked_add(2)?)?;
    Some(u16::from_be_bytes([bytes[0], bytes[1]]))
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
        loop {
            let b = *buf.get(off)?;
            off += 1;
            if b == 0 {
                break;
            }
            if b & 0xC0 == 0xC0 {
                // A compression pointer is two octets; the second one must
                // exist.
                off = off.checked_add(1)?;
                if off > buf.len() {
                    return None;
                }
                break;
            }
            off = off.checked_add(b as usize)?;
            if off > buf.len() {
                return None;
            }
        }
        off = off.checked_add(4)?; // QTYPE + QCLASS
        if off > buf.len() {
            return None;
        }
    }

    for _ in 0..ancount {
        let mut depth = 0usize;
        loop {
            if depth >= 16 {
                // Pointer loop guard.
                return None;
            }
            let b = *buf.get(off)?;
            if b == 0 {
                off += 1;
                break;
            }
            if b & 0xC0 == 0xC0 {
                if depth > 0 {
                    return None;
                }
                // Two octets, both of which must be inside the buffer.
                let ptr = ((be16_at(buf, off)? & 0x3FFF) as usize).min(buf.len());
                if ptr >= buf.len() {
                    return None;
                }
                off = ptr;
                depth += 1;
                continue;
            }
            off = off.checked_add(1)?.checked_add(b as usize)?;
            if off >= buf.len() {
                return None;
            }
        }
        // TYPE(2) CLASS(2) TTL(4) RDLENGTH(2)
        if off.checked_add(10)? > buf.len() {
            return None;
        }
        let rtype = be16_at(buf, off)?;
        let rdlength = be16_at(buf, off + 8)? as usize;
        off += 10;
        if rtype == QTYPE_A && rdlength >= 4 {
            let rd = buf.get(off..off.checked_add(4)?)?;
            return Some([rd[0], rd[1], rd[2], rd[3]]);
        }
        off = off.checked_add(rdlength)?;
        if off > buf.len() {
            return None;
        }
    }
    None
}

pub fn resolve(iface_idx: usize, dns_server: [u8; 4], domain: &str) -> Option<[u8; 4]> {
    {
        let mut state = DNS_STATE.lock();
        state.resp_ready = false;
    }

    let iface = nic::get_iface(iface_idx)?;
    let src_ip = iface.ip;

    let id = dns_id();
    let src_port = 12345;
    {
        let mut state = DNS_STATE.lock();
        state.expected_id = id;
        state.active_port = src_port;
    }

    let mut query = [0u8; 512];
    let qlen = build_query(id, domain, &mut query)?;

    udp::send(
        iface_idx,
        src_port,
        DNS_PORT,
        src_ip,
        dns_server,
        &query[..qlen],
    );

    for tick in 0..50000 {
        if tick > 0 && tick % 5000 == 0 {
            udp::send(
                iface_idx,
                src_port,
                DNS_PORT,
                src_ip,
                dns_server,
                &query[..qlen],
            );
        }
        nic::net_poll();
        let mut state = DNS_STATE.lock();
        if state.resp_ready {
            if let Some(ip) = parse_response(&state.resp_buf[..state.resp_len]) {
                state.active_port = 0;
                return Some(ip);
            }
        }
    }

    DNS_STATE.lock().active_port = 0;
    None
}
