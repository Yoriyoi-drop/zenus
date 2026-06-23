#![no_std]
#![allow(static_mut_refs)]
extern crate alloc;

pub mod pci;
pub mod queue;
pub mod net;
pub mod blk;
pub mod console;
pub mod balloon;

use zenus_console::serial::SerialPort;

pub const VIRTIO_VENDOR_ID: u16 = 0x1AF4;

pub const VIRTIO_DEVICE_NET: u16 = 0x1000;
pub const VIRTIO_DEVICE_BLOCK: u16 = 0x1001;
pub const VIRTIO_DEVICE_CONSOLE: u16 = 0x1003;
pub const VIRTIO_DEVICE_BALLOON: u16 = 0x1002;

pub const VIRTIO_TRANS_NET: u16 = 0x1041;
pub const VIRTIO_TRANS_BLOCK: u16 = 0x1042;
pub const VIRTIO_TRANS_CONSOLE: u16 = 0x1043;
pub const VIRTIO_TRANS_BALLOON: u16 = 0x1044;

pub fn match_device(device_id: u16) -> Option<&'static str> {
    match device_id {
        VIRTIO_DEVICE_NET | VIRTIO_TRANS_NET => Some("virtio-net"),
        VIRTIO_DEVICE_BLOCK | VIRTIO_TRANS_BLOCK => Some("virtio-blk"),
        VIRTIO_DEVICE_CONSOLE | VIRTIO_TRANS_CONSOLE => Some("virtio-console"),
        VIRTIO_DEVICE_BALLOON | VIRTIO_TRANS_BALLOON => Some("virtio-balloon"),
        _ => None,
    }
}

pub fn is_virtio_device(vendor_id: u16, device_id: u16) -> bool {
    vendor_id == VIRTIO_VENDOR_ID && match_device(device_id).is_some()
}

pub fn device_name(device_id: u16) -> &'static str {
    match_device(device_id).unwrap_or("unknown")
}

pub const QUEUE_SIZE: usize = 256;
pub const MAX_QUEUES: usize = 8;

fn serial() -> SerialPort {
    SerialPort::new(0x3F8)
}

pub unsafe fn init() {
    let s = serial();
    s.write_str("[VIRTIO] Scanning for devices...\n");

    let mut found = 0u32;
    for i in 0..zenus_arch::pci::MAX_PCI_DEVICES {
        let dev = &zenus_arch::pci::PCI_DEVICES[i];
        if dev.vendor_id == 0 && dev.device_id == 0 {
            break;
        }
        if dev.vendor_id != VIRTIO_VENDOR_ID {
            continue;
        }
        let dev_name = match match_device(dev.device_id) {
            Some(n) => n,
            None => continue,
        };

        s.write_str("[VIRTIO] Found ");
        s.write_str(dev_name);
        s.write_str(" at ");
        s.write_hex(dev.bus as u64);
        s.write_str(":");
        s.write_hex(dev.device as u64);
        s.write_str(".");
        s.write_hex(dev.function as u64);
        s.write_str("\n");

        zenus_arch::pci::enable_bus_master(dev.bus, dev.device, dev.function);

        let trans = match pci::init_device(dev) {
            Some(t) => t,
            None => {
                s.write_str("[VIRTIO] Failed to initialize PCI transport\n");
                continue;
            }
        };

        match dev.device_id {
            VIRTIO_DEVICE_NET | VIRTIO_TRANS_NET => {
                net::probe_and_init(trans);
            }
            VIRTIO_DEVICE_BLOCK | VIRTIO_TRANS_BLOCK => {
                blk::probe_and_init(trans);
            }
            VIRTIO_DEVICE_CONSOLE | VIRTIO_TRANS_CONSOLE => {
                console::VirtioConsole::new(trans);
            }
            VIRTIO_DEVICE_BALLOON | VIRTIO_TRANS_BALLOON => {
                balloon::VirtioBalloon::new(trans);
            }
            _ => {}
        }

        found += 1;
    }

    if found > 0 {
        s.write_str("[OK] Virtio: ");
        s.write_u64(found as u64);
        s.write_str(" device(s) initialized\n");
    } else {
        s.write_str("[VIRTIO] No virtio devices found\n");
    }
}
