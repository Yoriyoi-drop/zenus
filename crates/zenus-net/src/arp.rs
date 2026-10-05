use crate::ethernet;
use zenus_sync::spinlock::SpinLock;

const HARDWARE_TYPE_ETH: u16 = 0x0001;
const PROTOCOL_TYPE_IPV4: u16 = 0x0800;
const ARP_REQUEST: u16 = 0x0001;
const ARP_REPLY: u16 = 0x0002;

const ARP_CACHE_SIZE: usize = 16;

struct ArpState {
    gateway: [u8; 4],
    cache: [ArpEntry; ARP_CACHE_SIZE],
}

static ARP_STATE: SpinLock<ArpState> = SpinLock::new(ArpState {
    gateway: [10, 0, 2, 2],
    cache: [ArpEntry {
        ip: [0; 4],
        mac: [0; 6],
        valid: false,
    }; ARP_CACHE_SIZE],
});

#[derive(Clone, Copy)]
struct ArpEntry {
    ip: [u8; 4],
    mac: [u8; 6],
    valid: bool,
}

pub fn set_gateway(gw: [u8; 4]) {
    ARP_STATE.lock().gateway = gw;
}

fn arp_lookup(target_ip: [u8; 4]) -> Option<[u8; 6]> {
    let state = ARP_STATE.lock();
    for i in 0..ARP_CACHE_SIZE {
        if state.cache[i].valid && state.cache[i].ip == target_ip {
            return Some(state.cache[i].mac);
        }
    }
    None
}

pub fn add_static(ip: [u8; 4], mac: [u8; 6]) {
    // A static entry is authoritative, so it bypasses the "never replace a
    // cached MAC" rule: the whole point is that it is known ahead of time.
    let mut state = ARP_STATE.lock();
    for i in 0..ARP_CACHE_SIZE {
        if !state.cache[i].valid || state.cache[i].ip == ip {
            state.cache[i] = ArpEntry {
                ip,
                mac,
                valid: true,
            };
            return;
        }
    }
}

/// Record `mac` for `ip`, unless the cache already holds a different MAC.
///
/// Returns what happened, so a caller can report a poisoning attempt. The old
/// version returned nothing and silently kept the first answer, which made a
/// poisoned entry permanent: nothing could replace it and nothing could
/// observe that anything had tried.
fn arp_insert(ip: [u8; 4], mac: [u8; 6]) -> ArpInsert {
    let existing = arp_lookup(ip);
    let verdict = classify_insert(existing, ip, mac);
    if verdict != ArpInsert::Learned {
        return verdict;
    }
    let mut state = ARP_STATE.lock();
    if ip == state.gateway {
        return ArpInsert::Refused;
    }
    for i in 0..ARP_CACHE_SIZE {
        if !state.cache[i].valid {
            state.cache[i] = ArpEntry {
                ip,
                mac,
                valid: true,
            };
            return ArpInsert::Learned;
        }
    }
    for i in 1..ARP_CACHE_SIZE {
        if state.cache[i].ip != state.gateway {
            state.cache[i] = ArpEntry {
                ip,
                mac,
                valid: true,
            };
            return ArpInsert::Learned;
        }
    }
    // Nowhere to put it. Reported as a conflict so the caller can see the cache
    // is full rather than silently dropping the entry.
    ArpInsert::Conflict
}

/// Learned entries for one IP, and whether a second, different MAC ever showed
/// up for it.
///
/// `arp_insert` refuses to *change* a MAC that is already cached. That is the
/// right default, but it means the first answer wins forever — so a poisoned
/// entry cannot be corrected later, and it cannot even be noticed. This is the
/// read side of that decision: what `arp_handle` inserts and what it is about
/// to tell the world.
///
/// Pure, so the reply-versus-cache decision is testable without a NIC.
pub fn classify_insert(
    cache_ip: Option<[u8; 6]>,
    incoming_ip: [u8; 4],
    incoming_mac: [u8; 6],
) -> ArpInsert {
    match cache_ip {
        Some(existing) if existing == incoming_mac => ArpInsert::Known,
        Some(_) => ArpInsert::Conflict,
        None if incoming_ip == [0, 0, 0, 0] => ArpInsert::Refused,
        None => ArpInsert::Learned,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpInsert {
    /// Already cached with this MAC. Nothing changes.
    Known,
    /// Not cached; the sender is not a zero address.
    Learned,
    /// Not cached, but the sender claims 0.0.0.0 — an ARP request that has not
    /// been answered yet. Learning it would let a single unanswered probe
    /// redirect traffic.
    Refused,
    /// Already cached under a *different* MAC. The existing entry is kept, so
    /// this is a poisoning attempt and is worth reporting.
    Conflict,
}

/// Which interface answers an ARP request for `target_ip`.
///
/// The caller used to pass a hardcoded `10.0.2.15` and the *requester's* MAC
/// as its own, so the reply that would have been sent claimed to come from the
/// sender's own address — and, being wrong about the MAC, from the wrong NIC
/// too. It was then thrown away anyway; had it been sent, it would have been
/// sticky, because `arp_insert` never replaces a MAC. That is ARP poisoning the
/// kernel against itself.
///
/// Pure: takes the candidate identities rather than reading the interface
/// table, which is process-global and cannot be constructed on the host.
pub fn answering_identity<'a>(
    target_ip: [u8; 4],
    ifaces: &'a [([u8; 6], [u8; 4])],
) -> Option<&'a ([u8; 6], [u8; 4])> {
    ifaces.iter().find(|(_, ip)| *ip == target_ip)
}

/// The reply frame for an ARP request, or `None` if the request is not ours.
///
/// Kept separate from `arp::handle` so the shape of the answer — which MAC
/// goes in which field — can be asserted directly.
pub fn build_reply(
    our_mac: [u8; 6],
    our_ip: [u8; 4],
    requester_mac: [u8; 6],
    requester_ip: [u8; 4],
) -> [u8; 42] {
    arp_packet(&our_mac, &our_ip, &requester_mac, &requester_ip, ARP_REPLY)
}

fn arp_packet(
    src_mac: &[u8; 6],
    src_ip: &[u8; 4],
    dst_mac: &[u8; 6],
    dst_ip: &[u8; 4],
    opcode: u16,
) -> [u8; 42] {
    let mut buf = [0u8; 42];
    buf[0..6].copy_from_slice(dst_mac);
    buf[6..12].copy_from_slice(src_mac);
    buf[12..14].copy_from_slice(&crate::ethernet::ETH_ARP.to_be_bytes());
    let mut off = 14;
    buf[off..off + 2].copy_from_slice(&HARDWARE_TYPE_ETH.to_be_bytes());
    off += 2;
    buf[off..off + 2].copy_from_slice(&PROTOCOL_TYPE_IPV4.to_be_bytes());
    off += 2;
    buf[off] = 6;
    off += 1;
    buf[off] = 4;
    off += 1;
    buf[off..off + 2].copy_from_slice(&opcode.to_be_bytes());
    off += 2;
    buf[off..off + 6].copy_from_slice(src_mac);
    off += 6;
    buf[off..off + 4].copy_from_slice(src_ip);
    off += 4;
    buf[off..off + 6].copy_from_slice(dst_mac);
    off += 6;
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
    let pkt = arp_packet(&iface.mac, &iface.ip, &broadcast, &target_ip, ARP_REQUEST);
    crate::nic::send_frame(iface_idx, &pkt)
}

/// The target protocol address of an ARP payload, if the payload is long
/// enough to have one. Pure.
fn target_address(payload: &[u8]) -> Option<[u8; 4]> {
    if payload.len() < 28 {
        return None;
    }
    let ptr = payload.as_ptr();
    // SAFETY: the length is checked above and `read_unaligned` does not require
    // alignment; `payload` is a live slice for the whole read.
    Some(unsafe { core::ptr::read_unaligned(ptr.add(24) as *const [u8; 4]) })
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

    if hw_type != HARDWARE_TYPE_ETH {
        return None;
    }
    if proto_type != PROTOCOL_TYPE_IPV4 {
        return None;
    }
    if hw_addr_len != 6 || proto_addr_len != 4 {
        return None;
    }

    let sender_mac = unsafe { core::ptr::read_unaligned(ptr.add(8) as *const [u8; 6]) };
    let sender_ip = unsafe { core::ptr::read_unaligned(ptr.add(14) as *const [u8; 4]) };

    match arp_insert(sender_ip, sender_mac) {
        ArpInsert::Conflict => zenus_console::kwarn!(
            "ARP: {}.{}.{}.{} already maps to another MAC; keeping it",
            sender_ip[0],
            sender_ip[1],
            sender_ip[2],
            sender_ip[3]
        ),
        _ => {}
    }

    if opcode == ARP_REQUEST {
        let target_ip = match target_address(payload) {
            Some(t) => t,
            None => return None,
        };
        if target_ip != *our_ip {
            return None;
        }
        Some(build_reply(*our_mac, *our_ip, sender_mac, sender_ip))
    } else {
        None
    }
}

#[cfg(test)]
mod host_tests {
    use super::{
        answering_identity, build_reply, classify_insert, target_address, ArpInsert, ARP_REPLY,
    };

    const OURS: [u8; 6] = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
    const THEIRS: [u8; 6] = [0x02, 0x00, 0x00, 0x00, 0x00, 0x01];
    const OUR_IP: [u8; 4] = [10, 0, 2, 15];
    const THEIR_IP: [u8; 4] = [10, 0, 2, 2];

    /// A 28-byte ARP payload for a request from `THEIRS` about `target`.
    fn request(target: [u8; 4]) -> [u8; 28] {
        let mut p = [0u8; 28];
        p[0..2].copy_from_slice(&1u16.to_be_bytes()); // hw type: ethernet
        p[2..4].copy_from_slice(&0x0800u16.to_be_bytes()); // proto type: IPv4
        p[4] = 6;
        p[5] = 4;
        p[6..8].copy_from_slice(&1u16.to_be_bytes()); // opcode: request
        p[8..14].copy_from_slice(&THEIRS);
        p[14..18].copy_from_slice(&THEIR_IP);
        p[18..24].copy_from_slice(&[0; 6]);
        p[24..28].copy_from_slice(&target);
        p
    }

    /// Regression: `nic::poll_packet` computed the ARP reply and dropped it —
    /// the return value of `arp::handle` was ignored — so a virtio NIC never
    /// answered ARP and no outbound IPv4 traffic could leave the machine.
    ///
    /// The frame is what gets put on the wire, so asserting on its fields is
    /// what actually pins the bug: the old code would have produced a frame
    /// whose source MAC is the *requester's*.
    #[test]
    fn the_reply_carries_our_identity_and_their_addresses() {
        let frame = build_reply(OURS, OUR_IP, THEIRS, THEIR_IP);
        assert_eq!(frame.len(), 14 + 28);

        // Ethernet: to the requester, from us.
        assert_eq!(&frame[0..6], &THEIRS, "destination is the requester");
        assert_eq!(&frame[6..12], &OURS, "source is us, not the requester");
        assert_eq!(&frame[12..14], &0x0806u16.to_be_bytes(), "ARP");

        // ARP: sender = us, target = them.
        assert_eq!(&frame[14..16], &1u16.to_be_bytes(), "hw type ethernet");
        assert_eq!(&frame[16..18], &0x0800u16.to_be_bytes(), "proto IPv4");
        assert_eq!(frame[18], 6);
        assert_eq!(frame[19], 4);
        assert_eq!(&frame[20..22], &ARP_REPLY.to_be_bytes(), "opcode is reply");
        assert_eq!(&frame[22..28], &OURS, "sender hardware address");
        assert_eq!(&frame[28..32], &OUR_IP, "sender protocol address");
        assert_eq!(&frame[32..38], &THEIRS, "target hardware address");
        assert_eq!(&frame[38..42], &THEIR_IP, "target protocol address");

        // The specific failure the old call site would have produced: the
        // requester's MAC echoed back as our own.
        let poisoned = build_reply(THEIRS, OUR_IP, THEIRS, THEIR_IP);
        assert_eq!(&poisoned[6..12], &THEIRS, "what the old code would send");
        assert_ne!(
            &frame[6..12],
            &poisoned[6..12],
            "our reply must not be sourced from the requester's MAC"
        );
    }

    /// Which interface answers a request for a given address.
    ///
    /// The old call site passed a hardcoded `10.0.2.15` and the requester's MAC
    /// as its own identity, so it answered on behalf of an address that might
    /// not be ours, from a MAC that was never ours.
    #[test]
    fn the_answering_interface_is_chosen_by_address_not_hardcoded() {
        let ifaces = [
            ([0, 0, 0, 0, 0, 0], [127, 0, 0, 1]),
            (OURS, OUR_IP),
            ([0xAA; 6], [192, 168, 1, 5]),
        ];

        assert_eq!(
            answering_identity(OUR_IP, &ifaces),
            Some(&(OURS, OUR_IP)),
            "a request for our address must resolve to our interface"
        );
        assert_eq!(
            answering_identity([192, 168, 1, 5], &ifaces),
            Some(&([0xAA; 6], [192, 168, 1, 5]))
        );
        assert_eq!(
            answering_identity([10, 0, 2, 99], &ifaces),
            None,
            "nobody answers for an address nobody holds"
        );
        // A hardcoded answer would have matched regardless of the table.
        assert_ne!(
            answering_identity([10, 0, 2, 99], &ifaces).map(|(_, ip)| *ip),
            Some(OUR_IP)
        );
    }

    /// The cache keeps the first MAC it learns and refuses to change it. That
    /// is the right default, but it means a poisoned entry is permanent — so
    /// the attempt has to be observable.
    #[test]
    fn a_conflicting_mac_is_reported_rather_than_silently_ignored() {
        assert_eq!(classify_insert(None, THEIR_IP, THEIRS), ArpInsert::Learned);
        assert_eq!(
            classify_insert(Some(THEIRS), THEIR_IP, THEIRS),
            ArpInsert::Known,
            "the same MAC again is not a conflict"
        );
        assert_eq!(
            classify_insert(Some(THEIRS), THEIR_IP, [0xDE; 6]),
            ArpInsert::Conflict,
            "a different MAC for a cached IP must be reported"
        );
        assert_eq!(
            classify_insert(None, [0, 0, 0, 0], THEIRS),
            ArpInsert::Refused,
            "an unanswered probe claiming 0.0.0.0 must not be learned"
        );
    }

    /// The target address is read from a bounded offset; a short payload is
    /// refused rather than read past.
    #[test]
    fn the_target_address_needs_a_long_enough_payload() {
        let full = request(OUR_IP);
        assert_eq!(target_address(&full), Some(OUR_IP));
        for len in 0..28 {
            assert_eq!(
                target_address(&full[..len]),
                None,
                "{len} bytes must not yield a target address"
            );
        }
    }
}
