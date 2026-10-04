#![no_std]
#![allow(static_mut_refs)]

extern crate alloc;

// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub mod arp;
pub mod dhcp;
pub mod dhcp_server;
pub mod dns;
pub mod ethernet;
pub mod firewall;
pub mod icmp;
pub mod ipv4;
pub mod nic;
pub mod route;
pub mod rtl8139;
pub mod socket;
pub mod ssh;
pub mod tcp;
pub mod udp;

/// Host-side unit tests (`cargo test --target x86_64-unknown-linux-gnu`).
///
/// Only pure-logic paths are covered here: nothing in this module may touch
/// the NIC, the PIT or the UART (`nic::*`, `rtl8139::*`, `flush_output`).
/// Modules with private helpers carry their own `host_tests` submodule.
#[cfg(test)]
mod host_tests {
    use crate::ethernet::{self, ETH_ARP, ETH_IPV4};
    use crate::firewall::{
        self, ConnState, ConnTrack, FirewallAction, FirewallProto, FirewallRule, PacketInfo,
        MAX_CONNTRACK, MAX_RULES, RULE_NAME_LEN,
    };
    use crate::ipv4::{self, PROTO_TCP, PROTO_UDP};
    use crate::route::{self, GatewayAction};
    // The only internet-checksum implementation in the tree (the duplicate
    // that used to sit in `checksum.rs` was never declared as a module and has
    // been removed).
    use crate::ipv4::internet_checksum;
    use zenus_sync::spinlock::{SpinLock, SpinLockGuard};

    /// Serialises tests that share a module-global table (route, firewall).
    static SERIAL: SpinLock<()> = SpinLock::new(());

    fn serial() -> SpinLockGuard<'static, ()> {
        SERIAL.lock()
    }

    fn name(text: &str) -> [u8; RULE_NAME_LEN] {
        let mut out = [0u8; RULE_NAME_LEN];
        let bytes = text.as_bytes();
        let len = bytes.len().min(RULE_NAME_LEN);
        out[..len].copy_from_slice(&bytes[..len]);
        out
    }

    fn default_rule(rule_name: &str) -> FirewallRule {
        FirewallRule {
            name: name(rule_name),
            enabled: true,
            action: FirewallAction::Accept,
            proto: FirewallProto::Any,
            src_ip: [0; 4],
            src_mask: [0; 4],
            dst_ip: [0; 4],
            dst_mask: [0; 4],
            src_port: 0,
            dst_port: 0,
            established: false,
            packets_matched: 0,
        }
    }

    fn tcp_packet(src: [u8; 4], dst: [u8; 4], sport: u16, dport: u16) -> PacketInfo {
        PacketInfo {
            src_ip: src,
            dst_ip: dst,
            src_port: sport,
            dst_port: dport,
            proto: FirewallProto::Tcp,
        }
    }

    // ── checksum ──────────────────────────────────────────────────────────

    #[test]
    fn checksum_of_all_zeroes_is_ffff() {
        // The one's-complement sum of nothing is 0, and the checksum is the
        // complement — 0xFFFF is what a correct receiver expects to see.
        assert_eq!(internet_checksum(&[0u8; 20]), 0xFFFF);
    }

    #[test]
    fn checksum_verifies_a_known_header() {
        // RFC 1071: summing a buffer that already contains its checksum yields
        // 0 — that is exactly what a receiver checks (and what `ipv4::parse`
        // relies on when it recomputes the header checksum).
        let mut header = [0u8; 20];
        header[0] = 0x45;
        header[2..4].copy_from_slice(&20u16.to_be_bytes());
        header[8] = 64;
        header[9] = PROTO_TCP;
        header[12..16].copy_from_slice(&[192, 168, 1, 1]);
        header[16..20].copy_from_slice(&[192, 168, 1, 2]);
        let sum = internet_checksum(&header);
        header[10..12].copy_from_slice(&sum.to_be_bytes());
        assert_eq!(internet_checksum(&header), 0);
    }

    #[test]
    fn checksum_handles_odd_length_buffers() {
        // A trailing odd byte is padded on the right (network byte order).
        assert_ne!(internet_checksum(&[1, 2, 3]), internet_checksum(&[1, 2]));
        assert_eq!(internet_checksum(&[]), 0xFFFF);
        assert_eq!(internet_checksum(&[0xFF]), 0x00FF);
    }

    // ── ethernet ──────────────────────────────────────────────────────────

    #[test]
    fn ethernet_parse_splits_header_and_payload() {
        let frame = [
            0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, // dst
            0x02, 0x00, 0x00, 0x00, 0x00, 0x01, // src
            0x08, 0x06, // ARP, big endian
            0xDE, 0xAD, 0xBE, 0xEF, // payload
        ];
        let (header, payload) = ethernet::parse(&frame).expect("frame parses");
        assert_eq!(header.dst_mac, [0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);
        assert_eq!(header.src_mac, [0x02, 0, 0, 0, 0, 1]);
        assert_eq!(header.ether_type, ETH_ARP);
        assert_eq!(payload, &[0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn ethernet_parse_rejects_runt_frames() {
        assert!(ethernet::parse(&[0u8; 13]).is_none());
        assert!(ethernet::parse(&[]).is_none());
        assert_eq!(ETH_IPV4, 0x0800);
    }

    // ── ipv4 ──────────────────────────────────────────────────────────────

    /// Build a valid IPv4 packet (IHL 5, no options) with a correct header
    /// checksum. Fixed 24-byte array keeps the test free of `alloc`.
    fn ipv4_packet(protocol: u8, src: [u8; 4], dst: [u8; 4], payload: [u8; 4]) -> [u8; 24] {
        let mut packet = [0u8; 24];
        packet[0] = 0x45;
        packet[2..4].copy_from_slice(&24u16.to_be_bytes());
        packet[4..6].copy_from_slice(&0x1234u16.to_be_bytes());
        packet[6..8].copy_from_slice(&0x4000u16.to_be_bytes()); // don't fragment
        packet[8] = 64;
        packet[9] = protocol;
        packet[12..16].copy_from_slice(&src);
        packet[16..20].copy_from_slice(&dst);
        let sum = ipv4::internet_checksum(&packet[..20]);
        packet[10..12].copy_from_slice(&sum.to_be_bytes());
        packet[20..24].copy_from_slice(&payload);
        packet
    }

    #[test]
    fn ipv4_parse_reads_fields_and_splits_payload() {
        let packet = ipv4_packet(
            PROTO_UDP,
            [10, 0, 0, 2],
            [10, 0, 0, 1],
            [0x11, 0x22, 0x33, 0x44],
        );
        let (header, payload) = ipv4::parse(&packet).expect("packet parses");
        assert_eq!(header.version_ihl & 0xF0, 0x40, "version 4");
        assert_eq!((header.version_ihl & 0x0F) * 4, 20, "IHL 5 -> 20 bytes");
        assert_eq!(header.total_length, 24);
        assert_eq!(header.protocol, PROTO_UDP);
        assert_eq!(header.ttl, 64);
        assert_eq!(header.src_ip, [10, 0, 0, 2]);
        assert_eq!(header.dst_ip, [10, 0, 0, 1]);
        assert_eq!(payload, &[0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn ipv4_parse_rejects_bad_checksum() {
        let mut packet =
            ipv4_packet(PROTO_TCP, [192, 168, 0, 2], [192, 168, 0, 1], [1, 2, 3, 4]);
        packet[12] = 10; // corrupt the source address under the checksum
        assert!(ipv4::parse(&packet).is_none(), "corrupt header must be dropped");
    }

    #[test]
    fn ipv4_parse_rejects_impossible_lengths() {
        // Shorter than the fixed header.
        assert!(ipv4::parse(&[0x45; 19]).is_none());

        // total_length larger than the buffer (truncated capture).
        let mut packet = ipv4_packet(PROTO_TCP, [1, 2, 3, 4], [5, 6, 7, 8], [0; 4]);
        packet[2..4].copy_from_slice(&999u16.to_be_bytes());
        assert!(ipv4::parse(&packet).is_none());

        // total_length smaller than the IHL.
        let mut packet = ipv4_packet(PROTO_TCP, [1, 2, 3, 4], [5, 6, 7, 8], [0; 4]);
        packet[2..4].copy_from_slice(&10u16.to_be_bytes());
        assert!(ipv4::parse(&packet).is_none());
    }

    #[test]
    fn ipv4_parse_accepts_zero_checksum() {
        // A non-zero checksum is validated; zero means "not computed" and must
        // be accepted (that is how some senders and all tunnels behave).
        let mut packet = ipv4_packet(PROTO_UDP, [1, 2, 3, 4], [5, 6, 7, 8], [0; 4]);
        packet[10..12].copy_from_slice(&[0, 0]);
        assert!(ipv4::parse(&packet).is_some());
    }

    // ── ipv4 hardening ────────────────────────────────────────────────────

    /// Regression: `parse` accepted any IP version and interpreted the header
    /// with IPv4 offsets, so a version-6 packet parsed as a valid IPv4 one.
    #[test]
    fn ipv4_parse_rejects_other_versions_and_bad_ihl() {
        let mut packet = ipv4_packet(PROTO_TCP, [1, 2, 3, 4], [5, 6, 7, 8], [0; 4]);

        packet[0] = 0x65; // version 6, IHL 5
        assert!(ipv4::parse(&packet).is_none(), "version 6 must be rejected");

        packet[0] = 0x40; // version 4, IHL 0
        assert!(ipv4::parse(&packet).is_none(), "IHL 0 must be rejected");

        packet[0] = 0x44; // IHL 4 < 5 words
        assert!(ipv4::parse(&packet).is_none(), "IHL < 5 must be rejected");
    }

    /// Regression: a header with options (IHL > 5) was checksummed over the
    /// first 20 bytes only, so its attacker-controlled option bytes were never
    /// verified.
    #[test]
    fn ipv4_options_are_covered_by_the_checksum() {
        // 24-byte header (IHL 6): 20 fixed bytes + 4 option bytes, 4-byte
        // aligned so the payload starts on a 4-octet boundary as the RFC wants.
        let mut packet = [0u8; 28];
        packet[0] = 0x46;
        packet[2..4].copy_from_slice(&28u16.to_be_bytes());
        packet[8] = 64;
        packet[9] = PROTO_TCP;
        packet[12..16].copy_from_slice(&[192, 168, 0, 1]);
        packet[16..20].copy_from_slice(&[192, 168, 0, 2]);
        packet[20..24].copy_from_slice(&[0x01, 0x01, 0x01, 0x00]); // NOP-ish options
        let mut header = [0u8; 24];
        header.copy_from_slice(&packet[..24]);
        let sum = ipv4::internet_checksum(&header);
        packet[10..12].copy_from_slice(&sum.to_be_bytes());

        let (parsed, payload) = ipv4::parse(&packet).expect("valid header with options");
        assert_eq!((parsed.version_ihl & 0x0F) * 4, 24, "IHL 6 -> 24 bytes");
        assert_eq!(payload, &packet[24..28]);

        // Corrupting an option byte must now be detected.
        let mut tampered = packet;
        tampered[21] ^= 0xFF;
        assert!(ipv4::parse(&tampered).is_none(), "option bytes must be covered");
    }

    // ── dns name limits ───────────────────────────────────────────────────

    // ── route ─────────────────────────────────────────────────────────────

    #[test]
    fn route_lookup_prefers_longest_prefix() {
        let _serial = serial();
        route::clear();

        assert!(route::add_direct([10, 0, 0, 0], [255, 0, 0, 0], 0)); // /8
        assert!(route::add_direct([10, 1, 0, 0], [255, 255, 0, 0], 1)); // /16
        assert!(route::add([192, 168, 5, 0], [255, 255, 255, 0], [10, 0, 0, 1], 2)); // /24

        // 10.1.2.3 hits both the /8 and the /16 — the /16 must win.
        assert_eq!(
            route::lookup([10, 1, 2, 3]),
            Some((GatewayAction::Direct, 1))
        );
        // 10.9.9.9 only hits the /8.
        assert_eq!(
            route::lookup([10, 9, 9, 9]),
            Some((GatewayAction::Direct, 0))
        );
        // 192.168.5.7 matches the /24 with a gateway.
        assert_eq!(
            route::lookup([192, 168, 5, 7]),
            Some((GatewayAction::Via([10, 0, 0, 1]), 2))
        );
        assert_eq!(route::lookup([8, 8, 8, 8]), None, "unroutable without default");

        route::clear();
    }

    #[test]
    fn route_default_route_is_the_zero_prefix() {
        let _serial = serial();
        route::clear();

        assert!(route::add_default([10, 0, 0, 1], 3));
        assert_eq!(
            route::lookup([8, 8, 8, 8]),
            Some((GatewayAction::Via([10, 0, 0, 1]), 3))
        );

        // A more specific route still beats the default.
        assert!(route::add_direct([172, 16, 0, 0], [255, 240, 0, 0], 1));
        assert_eq!(
            route::lookup([172, 16, 5, 5]),
            Some((GatewayAction::Direct, 1))
        );

        route::clear();
        assert_eq!(route::lookup([8, 8, 8, 8]), None);
    }

    #[test]
    fn route_table_reports_full() {
        let _serial = serial();
        route::clear();

        for i in 0..8 {
            assert!(route::add_direct([10, 0, 0, i], [255, 255, 255, 0], 0), "route {i}");
        }
        assert!(!route::add_direct([10, 0, 1, 0], [255, 255, 255, 0], 0), "table is full");

        route::clear();
    }

    // ── firewall ──────────────────────────────────────────────────────────

    #[test]
    fn firewall_defaults_to_accept() {
        let _serial = serial();
        firewall::firewall_init();

        assert_eq!(firewall::firewall_rule_count(), 0);
        assert_eq!(
            firewall::firewall_check(&tcp_packet([1, 2, 3, 4], [5, 6, 7, 8], 1, 2)),
            FirewallAction::Accept
        );
    }

    #[test]
    fn firewall_first_match_wins_and_counts_hits() {
        let _serial = serial();
        firewall::firewall_init();

        let mut drop_all = default_rule("drop-all");
        drop_all.action = FirewallAction::Drop;
        assert!(firewall::firewall_add_rule(drop_all));

        let mut allow_web = default_rule("allow-web");
        allow_web.action = FirewallAction::Accept;
        allow_web.proto = FirewallProto::Tcp;
        allow_web.dst_ip = [192, 168, 0, 0];
        allow_web.dst_mask = [255, 255, 0, 0];
        allow_web.dst_port = 80;
        assert!(firewall::firewall_add_rule(allow_web));

        // drop-all sits at index 0, so it wins for everything...
        assert_eq!(
            firewall::firewall_check(&tcp_packet([1, 1, 1, 1], [2, 2, 2, 2], 1, 2)),
            FirewallAction::Drop
        );
        assert_eq!(firewall::firewall_rule_count(), 2);

        // ...including web traffic. Remove the first rule and web is reachable.
        assert!(firewall::firewall_remove_rule(0));
        assert!(!firewall::firewall_remove_rule(0), "removing twice fails");
        let rules = firewall::firewall_list_rules();
        let before = rules[1].expect("allow-web is still there").packets_matched;
        assert_eq!(
            firewall::firewall_check(&tcp_packet([1, 1, 1, 1], [192, 168, 5, 5], 1234, 80)),
            FirewallAction::Accept
        );
        assert_eq!(
            firewall::firewall_list_rules()[1].unwrap().packets_matched,
            before + 1,
            "the port-80 rule must be the one that matched"
        );
        // Same destination, wrong port -> falls through to the default accept
        // but must not have matched the port-specific rule. `allow-web` now
        // sits at index 1 because removing index 0 leaves a hole.
        let rules = firewall::firewall_list_rules();
        assert!(rules[0].is_none(), "index 0 stays empty after removal");
        let before = rules[1].unwrap().packets_matched;
        assert_eq!(
            firewall::firewall_check(&tcp_packet([1, 1, 1, 1], [192, 168, 5, 5], 1234, 22)),
            FirewallAction::Accept
        );
        assert_eq!(
            firewall::firewall_list_rules()[1].unwrap().packets_matched,
            before
        );

        firewall::firewall_init();
    }

    #[test]
    fn firewall_disabled_rule_is_skipped() {
        let _serial = serial();
        firewall::firewall_init();

        let mut off = default_rule("disabled");
        off.enabled = false;
        off.action = FirewallAction::Reject;
        assert!(firewall::firewall_add_rule(off));

        assert_eq!(
            firewall::firewall_check(&tcp_packet([1, 1, 1, 1], [2, 2, 2, 2], 1, 2)),
            FirewallAction::Accept,
            "a disabled rule must not apply"
        );

        firewall::firewall_init();
    }

    #[test]
    fn firewall_established_rule_needs_conntrack() {
        let _serial = serial();
        firewall::firewall_init();

        let mut established = default_rule("reply-only");
        established.action = FirewallAction::Accept;
        established.proto = FirewallProto::Tcp;
        established.src_ip = [192, 168, 1, 0];
        established.src_mask = [255, 255, 255, 0];
        established.established = true;
        assert!(firewall::firewall_add_rule(established));

        // The rule accepts and so does the fall-through default, so "returned
        // Accept" proves nothing. The per-rule match counter is what actually
        // distinguishes "matched" from "no rule applied".
        let matched = |idx: usize| {
            firewall::firewall_list_rules()[idx]
                .expect("rule present")
                .packets_matched
        };

        let pkt = tcp_packet([192, 168, 1, 50], [10, 0, 0, 1], 4000, 4001);
        let baseline = matched(0);
        assert_eq!(firewall::firewall_check(&pkt), FirewallAction::Accept);
        assert_eq!(
            matched(0),
            baseline,
            "without a conntrack entry the rule must not match"
        );

        firewall::firewall_track_connection(ConnTrack {
            src_ip: pkt.src_ip,
            dst_ip: pkt.dst_ip,
            src_port: pkt.src_port,
            dst_port: pkt.dst_port,
            proto: pkt.proto,
            state: ConnState::Established,
            last_seen: 0,
        });
        assert_eq!(firewall::firewall_conn_count(), 1);
        assert_eq!(firewall::firewall_check(&pkt), FirewallAction::Accept);
        assert_eq!(
            matched(0),
            baseline + 1,
            "with an established conntrack entry the rule must match"
        );

        // Tracking the same 5-tuple again updates state instead of adding a row.
        firewall::firewall_track_connection(ConnTrack {
            src_ip: pkt.src_ip,
            dst_ip: pkt.dst_ip,
            src_port: pkt.src_port,
            dst_port: pkt.dst_port,
            proto: pkt.proto,
            state: ConnState::Related,
            last_seen: 5,
        });
        assert_eq!(firewall::firewall_conn_count(), 1);

        firewall::firewall_init();
    }

    #[test]
    fn firewall_conntrack_table_is_bounded() {
        let _serial = serial();
        firewall::firewall_init();

        for i in 0..MAX_CONNTRACK {
            firewall::firewall_track_connection(ConnTrack {
                src_ip: [10, 0, 0, 1],
                dst_ip: [10, 0, 0, 2],
                src_port: i as u16,
                dst_port: 80,
                proto: FirewallProto::Tcp,
                state: ConnState::New,
                last_seen: 0,
            });
        }
        assert_eq!(firewall::firewall_conn_count(), MAX_CONNTRACK);

        // The table is full: the extra entry is dropped, not overwritten.
        firewall::firewall_track_connection(ConnTrack {
            src_ip: [10, 0, 0, 3],
            dst_ip: [10, 0, 0, 4],
            src_port: 1,
            dst_port: 1,
            proto: FirewallProto::Udp,
            state: ConnState::New,
            last_seen: 0,
        });
        assert_eq!(firewall::firewall_conn_count(), MAX_CONNTRACK);

        // `firewall_clear_connections` is an *age*-based evictor despite the
        // name, so nothing expires unless the tick counter moves. `pit::tick()`
        // is a plain atomic increment (no port I/O), so it is safe here, and it
        // is what makes this path observable at all.
        assert_eq!(firewall::firewall_conn_count(), MAX_CONNTRACK);
        for _ in 0..400 {
            zenus_arch::interrupts::pit::tick();
        }
        firewall::firewall_clear_connections();
        assert_eq!(
            firewall::firewall_conn_count(),
            0,
            "entries older than the timeout must be evicted"
        );

        firewall::firewall_init();
    }

    #[test]
    fn firewall_rule_table_is_bounded() {
        let _serial = serial();
        firewall::firewall_init();

        for i in 0..MAX_RULES {
            let mut rule = default_rule("bulk");
            rule.dst_port = i as u16;
            assert!(firewall::firewall_add_rule(rule), "rule {i}");
        }
        assert!(!firewall::firewall_add_rule(default_rule("overflow")));
        assert!(!firewall::firewall_remove_rule(MAX_RULES));

        firewall::firewall_init();
        assert_eq!(firewall::firewall_rule_count(), 0);
    }

    #[test]
    fn firewall_port_rule_ignores_icmp_ports() {
        let _serial = serial();
        firewall::firewall_init();

        let mut drop_tcp_22 = default_rule("no-ssh");
        drop_tcp_22.action = FirewallAction::Drop;
        drop_tcp_22.proto = FirewallProto::Tcp;
        drop_tcp_22.dst_port = 22;
        assert!(firewall::firewall_add_rule(drop_tcp_22));

        assert_eq!(
            firewall::firewall_check(&tcp_packet([1, 1, 1, 1], [2, 2, 2, 2], 1, 22)),
            FirewallAction::Drop
        );

        // Deliberately give the ICMP packet port 22 — the same number the rule
        // filters on. Only the `proto == Tcp || proto == Udp` guard can keep the
        // port comparison from rejecting it, so this test now fails if that
        // guard is ever removed.
        let icmp = PacketInfo {
            src_ip: [1, 1, 1, 1],
            dst_ip: [2, 2, 2, 2],
            src_port: 22,
            dst_port: 22,
            proto: FirewallProto::Icmp,
        };
        let baseline = firewall::firewall_list_rules()[0].unwrap().packets_matched;
        assert_eq!(
            firewall::firewall_check(&icmp),
            FirewallAction::Accept,
            "ICMP has no ports, so a port rule must not match"
        );
        assert_eq!(
            firewall::firewall_list_rules()[0].unwrap().packets_matched,
            baseline,
            "the TCP-only rule must not have matched"
        );

        firewall::firewall_init();
    }
}
