use crate::rtl8139::Rtl8139;
use zenus_virtio::net::VirtioNet;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NicType {
    Unknown,
    IntelPro1000,
    Rtl8139,
    Virtio,
    Loopback,
}

#[derive(Debug, Clone, Copy)]
pub struct NetworkInterface {
    pub nic_type: NicType,
    pub mac: [u8; 6],
    pub ip: [u8; 4],
    pub subnet_mask: [u8; 4],
    pub gateway: [u8; 4],
    pub mmio_base: u64,
    pub irq: u8,
    pub link_up: bool,
}

const MAX_INTERFACES: usize = 8;
static mut INTERFACES: [Option<NetworkInterface>; MAX_INTERFACES] = [None; MAX_INTERFACES];
static mut IFACE_COUNT: usize = 0;

pub fn init() {
    let lo = NetworkInterface {
        nic_type: NicType::Loopback,
        mac: [0; 6],
        ip: [127, 0, 0, 1],
        subnet_mask: [255, 0, 0, 0],
        gateway: [0; 4],
        mmio_base: 0,
        irq: 0,
        link_up: true,
    };
    unsafe {
        INTERFACES[0] = Some(lo);
        IFACE_COUNT = 1;
    }

    let mut found_nic = false;

    if let Some(rtl) = Rtl8139::probe_and_init() {
        let iface = NetworkInterface {
            nic_type: NicType::Rtl8139,
            mac: *rtl.mac(),
            ip: *rtl.ip(),
            subnet_mask: [255, 255, 255, 0],
            gateway: [10, 0, 2, 2],
            mmio_base: 0,
            irq: 0,
            link_up: rtl.is_link_up(),
        };
        unsafe {
            INTERFACES[IFACE_COUNT] = Some(iface);
            IFACE_COUNT += 1;
        }
        zenus_console::kinfo!("RTL8139 NIC registered");
        found_nic = true;
    }

    if let Some(virtio) = VirtioNet::nic_ref() {
        let iface = NetworkInterface {
            nic_type: NicType::Virtio,
            mac: virtio.mac,
            ip: [10, 0, 2, 15],
            subnet_mask: [255, 255, 255, 0],
            gateway: [10, 0, 2, 2],
            mmio_base: 0,
            irq: 0,
            link_up: true,
        };
        unsafe {
            INTERFACES[IFACE_COUNT] = Some(iface);
            IFACE_COUNT += 1;
        }
        zenus_console::kinfo!("Virtio-NIC registered");
        found_nic = true;
    }

    if found_nic {
        crate::arp::add_static([10, 0, 2, 2], [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02]);
        crate::arp::add_static([10, 0, 2, 3], [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02]);
        crate::route::add_direct([10, 0, 2, 0], [255, 255, 255, 0], 1);
        crate::route::add_default([10, 0, 2, 2], 1);
        zenus_console::kinfo!("Routes configured (10.0.2.0/24 + default via 10.0.2.2)");
    }

    zenus_console::kinfo!("Network subsystem initialized");
}

pub fn net_poll() {
    Rtl8139::with_nic(|rtl| rtl.poll());
    VirtioNet::with_nic(|v| {
        v.poll();
        while let Some(mut pkt) = {
            let mut buf = [0u8; 1514];
            v.receive(&mut buf).map(|len| (buf, len))
        } {
            let (buf, len) = &mut pkt;
            let idx = match virtio_iface_index() {
                Some(i) => i,
                None => return,
            };
            let iface = get_iface(idx).expect("index came from the table");
            let mut reply = [0u8; 1514];
            if let Some(len) = poll_packet(idx, &iface, &buf[..*len], &mut reply) {
                // Sent with `v` directly, not through `send_frame`: we are
                // already inside `with_nic`, and going back through it would
                // take `NET_LOCK` a second time — the same self-deadlock the
                // RTL8139 `poll()` comment describes.
                let _ = v.send_raw(&reply[..len]);
            }
        }
    });
}

/// Index of the virtio NIC in the interface table.
///
/// Hardcoding `1` was only right when the RTL8139 probe happened first, and
/// `poll_packet` also passed `1` to `tcp`/`udp::handle_receive` regardless of
/// which NIC the frame actually arrived on. Both are wrong whenever the other
/// driver registers first, or when only virtio is present.
pub fn virtio_iface_index() -> Option<usize> {
    for i in 0..iface_count() {
        if let Some(iface) = get_iface(i) {
            if iface.nic_type == NicType::Virtio {
                return Some(i);
            }
        }
    }
    None
}

/// Handle one received frame. Returns the length of a frame to send back, if
/// this frame needs an answer.
fn poll_packet(
    iface_idx: usize,
    iface: &NetworkInterface,
    data: &[u8],
    reply: &mut [u8; 1514],
) -> Option<usize> {
    if data.len() < 14 {
        return None;
    }
    let (eth_hdr, eth_payload) = match crate::ethernet::parse(data) {
        Some(h) => h,
        None => return None,
    };
    match eth_hdr.ether_type {
        crate::ethernet::ETH_ARP => {
            // The reply was computed and then dropped: the return value was
            // ignored, so a virtio NIC never answered ARP and no outbound IPv4
            // traffic could leave the machine. `iface.ip`/`iface.mac` are our
            // own identity — the old code passed a hardcoded address and the
            // requester's MAC, which would have made the answer sticky
            // poisoning against ourselves.
            let arp_reply = crate::arp::handle(&eth_hdr, eth_payload, &iface.ip, &iface.mac)?;
            if reply.len() < arp_reply.len() {
                return None;
            }
            reply[..arp_reply.len()].copy_from_slice(&arp_reply);
            Some(arp_reply.len())
        }
        crate::ethernet::ETH_IPV4 => {
            let (ip_hdr, ip_payload) = crate::ipv4::parse(eth_payload)?;
            // Not for us: the RTL8139 path has always checked this and the
            // virtio path did not, so every host on the segment was being
            // handed to TCP, UDP and the firewall.
            if ip_hdr.dst_ip != iface.ip && ip_hdr.dst_ip != [255; 4] {
                return None;
            }
            let pkt_proto = match ip_hdr.protocol {
                crate::ipv4::PROTO_TCP => crate::firewall::FirewallProto::Tcp,
                crate::ipv4::PROTO_UDP => crate::firewall::FirewallProto::Udp,
                crate::ipv4::PROTO_ICMP => crate::firewall::FirewallProto::Icmp,
                _ => crate::firewall::FirewallProto::Any,
            };
            let (src_port, dst_port) = if ip_payload.len() >= 4 {
                let sp = u16::from_be_bytes([ip_payload[0], ip_payload[1]]);
                let dp = u16::from_be_bytes([ip_payload[2], ip_payload[3]]);
                (sp, dp)
            } else {
                (0, 0)
            };
            let pkt = crate::firewall::PacketInfo {
                src_ip: ip_hdr.src_ip,
                dst_ip: ip_hdr.dst_ip,
                src_port,
                dst_port,
                proto: pkt_proto,
            };
            if crate::firewall::firewall_check(&pkt) != crate::firewall::FirewallAction::Accept {
                return None;
            }
            let ticks = zenus_arch::interrupts::pit::get_ticks();
            let ct = crate::firewall::ConnTrack {
                src_ip: ip_hdr.src_ip,
                dst_ip: ip_hdr.dst_ip,
                src_port,
                dst_port,
                proto: pkt_proto,
                state: crate::firewall::ConnState::Established,
                last_seen: ticks,
            };
            crate::firewall::firewall_track_connection(ct);
            match ip_hdr.protocol {
                crate::ipv4::PROTO_TCP => {
                    crate::tcp::handle_receive(iface_idx, ip_hdr.src_ip, ip_hdr.dst_ip, ip_payload);
                }
                crate::ipv4::PROTO_UDP => {
                    crate::udp::handle_receive(iface_idx, ip_hdr.src_ip, ip_hdr.dst_ip, ip_payload);
                }
                // The virtio path dropped ICMP entirely, so `ping` never got an
                // answer on that NIC. The RTL8139 path has always replied.
                crate::ipv4::PROTO_ICMP => {
                    if let Some(icmp_reply) = crate::icmp::handle_echo(
                        &ip_hdr,
                        ip_payload,
                        &iface.mac,
                        &eth_hdr.src_mac,
                        &iface.ip,
                    ) {
                        if reply.len() >= icmp_reply.len() {
                            reply[..icmp_reply.len()].copy_from_slice(&icmp_reply);
                            return Some(icmp_reply.len());
                        }
                    }
                }
                _ => {}
            }
            None
        }
        _ => None,
    }
}

pub fn iface_count() -> usize {
    unsafe { IFACE_COUNT }
}

pub fn get_iface(idx: usize) -> Option<NetworkInterface> {
    unsafe {
        if idx < IFACE_COUNT {
            INTERFACES[idx]
        } else {
            None
        }
    }
}

pub fn send_frame(iface_idx: usize, frame: &[u8]) -> bool {
    if iface_idx == 0 {
        return false;
    }
    let mut sent = false;
    Rtl8139::with_nic(|rtl| {
        if frame.len() < 60 {
            let mut buf = [0u8; 60];
            buf[..frame.len()].copy_from_slice(frame);
            sent = rtl.send_raw(&buf);
        } else {
            sent = rtl.send_raw(frame);
        }
    });
    if sent {
        return true;
    }
    VirtioNet::with_nic(|v| {
        sent = v.send_raw(frame);
    });
    sent
}

pub fn send_packet(iface_idx: usize, data: &[u8]) -> bool {
    let iface = match get_iface(iface_idx) {
        Some(iface) => iface,
        None => return false,
    };
    if iface.nic_type == NicType::Loopback {
        return false;
    }
    if data.len() < 20 {
        return false;
    }

    let dst_ip = [data[16], data[17], data[18], data[19]];
    let next_hop = match crate::route::lookup(dst_ip) {
        Some((crate::route::GatewayAction::Via(gw), _)) => gw,
        _ => dst_ip,
    };
    let dst_mac = match crate::arp::resolve(iface_idx, next_hop) {
        Some(mac) => mac,
        None => return false,
    };

    let total_len = 14 + data.len();
    if total_len > 1514 {
        return false;
    }

    let mut buf = [0u8; 1514];
    buf[0..6].copy_from_slice(&dst_mac);
    buf[6..12].copy_from_slice(&iface.mac);
    buf[12..14].copy_from_slice(&crate::ethernet::ETH_IPV4.to_be_bytes());
    buf[14..total_len].copy_from_slice(data);

    let frame = if total_len < 60 {
        let pad = 60 - total_len;
        for i in 0..pad {
            buf[total_len + i] = 0;
        }
        &buf[..60]
    } else {
        &buf[..total_len]
    };

    send_frame(iface_idx, frame)
}

pub fn send_broadcast_packet(iface_idx: usize, data: &[u8]) -> bool {
    let iface = match get_iface(iface_idx) {
        Some(iface) => iface,
        None => return false,
    };
    if iface.nic_type == NicType::Loopback {
        return false;
    }
    if data.len() < 20 {
        return false;
    }

    let total_len = 14 + data.len();
    if total_len > 1514 {
        return false;
    }

    let mut buf = [0u8; 1514];
    for i in 0..6 {
        buf[i] = 0xFF;
    }
    buf[6..12].copy_from_slice(&iface.mac);
    buf[12..14].copy_from_slice(&crate::ethernet::ETH_IPV4.to_be_bytes());
    buf[14..total_len].copy_from_slice(data);

    let frame_len = core::cmp::max(total_len, 60);
    send_frame(iface_idx, &buf[..frame_len])
}

pub fn set_iface_ip(iface_idx: usize, ip: [u8; 4], subnet: [u8; 4], gateway: [u8; 4]) -> bool {
    unsafe {
        if iface_idx >= IFACE_COUNT {
            return false;
        }
        if let Some(ref mut iface) = INTERFACES[iface_idx] {
            iface.ip = ip;
            iface.subnet_mask = subnet;
            iface.gateway = gateway;
        }
    }
    true
}

pub fn receive_packet(_iface: usize, buf: &mut [u8]) -> Option<usize> {
    let mut result = None;
    Rtl8139::with_nic(|rtl| {
        result = rtl.receive_copy(buf);
    });
    if result.is_some() {
        return result;
    }
    VirtioNet::with_nic(|v| {
        result = v.receive(buf);
    });
    result
}
