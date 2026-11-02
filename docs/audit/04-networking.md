## 4. Networking

| Sub-kategori | Status | Level Risiko |
|---|---|---|
| IPv4 | PARTIAL | CRITICAL |
| IPv6 | NOT IMPLEMENTED | MEDIUM |
| TCP | FUNCTIONAL | MEDIUM |
| UDP | FUNCTIONAL | LOW |
| ICMP | FUNCTIONAL | LOW |
| DHCP | ✅ FUNCTIONAL | MEDIUM |
| DNS | ✅ FUNCTIONAL | LOW |
| Routing | ✅ FUNCTIONAL | MEDIUM |
| Firewall | ✅ FUNCTIONAL | MEDIUM |
| NAT | NOT IMPLEMENTED | MEDIUM |
| VLAN | NOT IMPLEMENTED | LOW |
| Bonding | NOT IMPLEMENTED | LOW |
| VPN support | NOT IMPLEMENTED | LOW |

### Detail
- **Driver:** 👾 **IRQ-DRIVEN RTL8139** (PIO, IRQ 11 via I/O APIC vector 43, polling fallback). I/O APIC driver implemented with IOAPIC base at 0xFEC00000, RTE programming for PCI IRQ routing. NIC interrupt handler acks ISR + sends LAPIC EOI. Loopback interface virtual (no actual data path).
- **IPv4:** Parsing header. ICMP echo reply (ping) berfungsi penuh. ARP reply handler. **IP address hardcoded** (10.0.2.15/24, gateway 10.0.2.2).
- **TCP:** **FUNCTIONAL — 11/11 RFC 793 states.** LISTEN, SYN_SENT, SYN_RCVD, ESTABLISHED, FIN_WAIT1, FIN_WAIT2, CLOSE_WAIT, CLOSING, LAST_ACK, TIME_WAIT, CLOSED — semua wired. Connection tracking via 16-entry TCB table dengan port binding. Three-way handshake (SYN→SYN-ACK→ACK + client-side connect), data sequence number validation, ACK processing (send_una tracking), FIN handshake (active + passive close, simultaneous close via CLOSING). ISN randomization via RDRAND/LCG RNG. Multi-connection per listening port (child TCB allocation on SYN). Slot recycling (scans None then CLOSED). Poll-based retransmission (5 retries, tick interval). Window tracking (peer's advertised window). TIME_WAIT timeout (20 poll cycles). Echo server verified end-to-end dengan `nc` via accept-loop. Socket API: accept() untuk incoming, connect() untuk outgoing.

  **Masih kurang:**
  - Congestion control (slow start, congestion avoidance, fast recovery)
  - Window scaling (RFC 7323), selective ACK (SACK), PAWS, MSS
  - keepalive, urgent pointer, TCP options parsing
- **UDP:** **FUNCTIONAL — send/recv datagrams.** Header parsing, payload extraction, echo reply on port 7. Verified with `nc -u`. Socket API: port binding via bind(), send/sendto(), recv() with ring buffer (8 entries, 1500 bytes each). Generic port dispatch — any port with a bound UDP socket receives datagrams. Checksum validation tidak dilakukan (optional di UDP, set 0).
- **DHCP:** ✅ **FUNCTIONAL — client state machine.** DHCPDISCOVER→OFFER→REQUEST→ACK. Menyimpan IP, subnet mask, gateway dari server DHCP. DIimplementasikan sebagai module `dhcp.rs` dengan `dhcp_start(iface_idx)`. Broadcast via ethernet FF:FF:FF:FF:FF:FF (bypass ARP). Verified dengan QEMU user-mode DHCP server (10.0.2.15/24, gateway 10.0.2.2). **Masih kurang:** Lease renewal timer, DHCPRELEASE, DHCPINFORM, multiple interface support.
- **DNS:** ✅ **FUNCTIONAL — stub resolver.** Builds DNS A-record query, sends via UDP to configurable DNS server (default 10.0.2.3), parses response (handles CNAME, compression pointers). Verified with `resolve example.com` → 172.66.147.243 and `resolve google.com` → 74.125.130.113. **Masih kurang:** Recursive resolution, multiple record types (AAAA, MX, TXT), TCP fallback, caching.
- **Routing:** ✅ **FUNCTIONAL — routing table (8 entries, longest-prefix match).** Two route types: direct (gateway=0.0.0.0 → ARP for destination) and gateway (→ ARP for gateway). Default route added at init and by DHCP. `nic::send_packet` uses `route::lookup(next_hop_ip)` to determine next-hop before ARP. **Masih kurang:** Route metrics, multipath, policy routing, route expiry, ICMP redirect handling.
- **Firewall:** Tidak ada netfilter/iptables/nftables.
- **NIC abstraction:** `send_packet` dan `receive_packet` functional via RTL8139 driver. Loopback interface virtual (stub).
- **IPV6:** Hanya di-define di EtherType, tanpa handler.
- **Missing:** TCP outbound connect, firewall, NAT, VLAN, bonding, VPN, WireGuard, IRQ-driven NIC, multi-queue, checksum offload, TSO/GSO, jumbo frames, DHCP lease renewal.

### Perbandingan
| Aspek | Debian | Ubuntu | RHEL | Alpine | Zenus |
|---|---|---|---|---|---|
| TCP | Full | Full | Full | Full | Stub |
| UDP | Full | Full | Full | Full | Stub |
| IPv6 | Full | Full | Full | Full | None |
| Firewall | nftables | nftables | nftables | iptables | None |
| NIC drivers | 100+ | 100+ | 100+ | 100+ | 1 (RTL8139) |
| Socket API | BSD | BSD | BSD | BSD | None |

---

