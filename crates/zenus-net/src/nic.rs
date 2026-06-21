use zenus_console::serial::SerialPort;
use crate::rtl8139::Rtl8139;

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
    let mut s = SerialPort::new(0x3F8);

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
        s.write_str("[OK] RTL8139 NIC registered\n");

        crate::arp::add_static(
            [10, 0, 2, 2],
            [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02],
        );
        crate::arp::add_static(
            [10, 0, 2, 3],
            [0x52, 0x55, 0x0a, 0x00, 0x02, 0x02],
        );
        crate::route::add_direct([10, 0, 2, 0], [255, 255, 255, 0], 1);
        crate::route::add_default([10, 0, 2, 2], 1);
        s.write_str("[OK] Routes configured (10.0.2.0/24 + default via 10.0.2.2)\n");
    }

    s.write_str("[OK] Network subsystem initialized\n");
}

pub fn net_poll() {
    if let Some(ref mut rtl) = Rtl8139::get_instance() {
        rtl.poll();
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
    if let Some(ref mut rtl) = Rtl8139::get_instance() {
        if frame.len() < 60 {
            let mut buf = [0u8; 60];
            buf[..frame.len()].copy_from_slice(frame);
            rtl.send_raw(&buf)
        } else {
            rtl.send_raw(frame)
        }
    } else {
        false
    }
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
        None => {
            return false;
        }
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

    if total_len < 60 {
        let pad = 60 - total_len;
        for i in 0..pad {
            buf[total_len + i] = 0;
        }
        send_frame(iface_idx, &buf[..60])
    } else {
        send_frame(iface_idx, &buf[..total_len])
    }
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
    for i in 0..6 { buf[i] = 0xFF; }
    buf[6..12].copy_from_slice(&iface.mac);
    buf[12..14].copy_from_slice(&crate::ethernet::ETH_IPV4.to_be_bytes());
    buf[14..total_len].copy_from_slice(data);

    if total_len < 60 {
        let mut frame = [0u8; 60];
        frame[..total_len].copy_from_slice(&buf[..total_len]);
        send_frame(iface_idx, &frame)
    } else {
        send_frame(iface_idx, &buf[..total_len])
    }
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
    if iface_idx == 1 {
        if let Some(ref mut rtl) = Rtl8139::get_instance() {
            rtl.set_ip(ip);
            rtl.set_subnet(subnet);
            rtl.set_gateway(gateway);
        }
    }
    true
}

pub fn receive_packet(_iface: usize, buf: &mut [u8]) -> Option<usize> {
    if let Some(ref mut rtl) = Rtl8139::get_instance() {
        rtl.receive_copy(buf)
    } else {
        None
    }
}
