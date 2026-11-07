## Known Critical Bugs (2026-06-21 Debug Session)

| # | Bug | File:Line | Severity | Status |
|---|---|---|---|---|
| 1 | **DMA use-after-free — TX buffers on stack** | rtl8139.rs:274-293 | **CRITICAL** | ✅ FIXED — wait for TSD_TOK after write ensures DMA completes before caller's buffer (stack) is freed |
| 2 | **FIN+data ordering — data silently dropped when FIN in same segment** | tcp.rs:292-312 | **HIGH** | ✅ FIXED — data handler before FIN handler, `recv_nxt = seq + payload.len() + 1` |
| 3 | **IP `total_length < ihl` slice panic** | ipv4.rs:46 | **HIGH** | ✅ FIXED — guard `total_length < ihl \|\| total_length > packet.len()` added |
| 4 | **TCP data offset < 20 → header treated as payload** | tcp.rs:130 | **HIGH** | ✅ FIXED — guard `hdr_len < 20 \|\| hdr_len > segment.len()` added |
| 5 | **`recv_nxt` acknowledges data dropped due to buffer full** | tcp.rs:267,319 | **HIGH** | ✅ FIXED — `recv_nxt = seq + copy_len` instead of `seq + payload.len()` |
| 6 | **No TCP checksum validation on receive** | tcp.rs:115 | **HIGH** | ✅ FIXED — `if checksum(...) != 0 { return false; }` added |
| 7 | **`listening` flag never cleared after SYN-ACK** | tcp.rs:232-241 | **HIGH** | ✅ FIXED — `tcb.listening = false` set after SYN-ACK |
| 8 | **SYN_RECEIVED: no seq check before ESTABLISHED transition** | tcp.rs:258-261 | **HIGH** | ✅ FIXED — duplicate ACK sent if seq doesn't match recv_nxt |
| 9 | **`send_data` always returns true even on buffer full** | tcp.rs:385-390 | **MEDIUM** | ✅ FIXED — returns `copy_len > 0` |
| 10 | **CAPR `wrapping_sub(0x10)` underflows at small rx_cur** | rtl8139.rs:428 | **MEDIUM** | ❌ FALSE ALARM — `receive_copy` is dead code; `process_rx` always has `rx_cur ≥ 0x10` after any valid packet |
| 11 | **ARP gateway evicted when cache fills** | arp.rs:47 | **MEDIUM** | ✅ FIXED — arp_insert scans for non-gateway slot before evicting |
| 12 | **TX descriptor reuse without completion check (4-slot ring)** | rtl8139.rs:291 | **MEDIUM** | ✅ FIXED — covered by Bug #1 TSD_TOK wait (descriptor is confirmed complete before function returns) |
| 13 | **Journal write_header bypassed cache — stale reads returned wrong num_entries** | journal.rs:229-243 | **HIGH** | ✅ FIXED — changed `block_device_write` to `bc_write`, all entries now recorded correctly |
| 14 | **Journal commit didn't flush target data to disk** | journal.rs:128 | **HIGH** | ✅ FIXED — added `bc_flush()` after writing all target blocks and final header |
| 15 | **DHCP response port dispatch - UDP handle_receive only accepted port 7** | udp.rs:42 | **MEDIUM** | ✅ FIXED — added dispatch for ports 67/68 to dhcp::handle_receive |
| 16 | **DNS poll window too short for slow DNS servers** | dns.rs:194 | **MEDIUM** | ✅ FIXED — increased to 50000 polls with periodic re-sends every 5000 |
| 17 | **send_packet used dst_ip directly instead of routing next-hop** | nic.rs:128 | **MEDIUM** | ✅ FIXED — route::lookup determines next-hop (direct or via gateway) before ARP |
| 18 | **NIC IRQ handler called without clearing RTL8139 ISR — GPF on re-entrant interrupt** | handler.rs:27 | **HIGH** | ✅ FIXED — handler reads RTL8139 ISR, writes it back to clear, then sends LAPIC EOI |
| 19 | **#GP(0x10) at iretq in apic_timer_isr_stub — Ring 3 → kernel stack frame mismatch** | scheduler.rs:95-142 | **HIGH** | ✅ FIXED — ISR distinguishes frame type via CS.RPL: if from Ring 0 (3-item frame), uses `pop rax; add rsp,8; popfq; jmp rax` instead of `iretq`; if from Ring 3 (5-item frame), keeps `iretq` with SS fixup. |

---

*Audit completed 2026-06-21 by automated analysis of source tree at `/home/whale-d/zenus`.*
