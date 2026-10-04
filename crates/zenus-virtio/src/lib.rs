#![no_std]
#![allow(static_mut_refs)]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

extern crate alloc;

pub mod balloon;
pub mod blk;
pub mod console;
pub mod net;
pub mod pci;
pub mod queue;

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

pub fn serial() -> zenus_console::serial::SerialPort {
    zenus_console::serial::SerialPort::new(0x3F8)
}

pub unsafe fn init() {
    zenus_console::kinfo!("Virtio scanning for devices...");

    let mut found = 0u32;
    let count = zenus_arch::pci::get_device_count();
    for i in 0..count {
        let dev = match zenus_arch::pci::get_device(i) {
            Some(d) => d,
            None => continue,
        };
        if dev.vendor_id != VIRTIO_VENDOR_ID {
            continue;
        }
        let dev_name = match match_device(dev.device_id) {
            Some(n) => n,
            None => continue,
        };

        zenus_console::kinfo!(
            "Virtio found {} at {}:{}:{}",
            dev_name,
            dev.bus,
            dev.device,
            dev.function
        );

        zenus_arch::pci::enable_bus_master(dev.bus, dev.device, dev.function);

        let trans = match pci::init_device(&dev) {
            Some(t) => t,
            None => {
                zenus_console::kwarn!(
                    "Virtio: failed to initialize PCI transport for {}",
                    dev_name
                );
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
        zenus_console::kinfo!("Virtio: {} device(s) initialized", found);
    } else {
        zenus_console::kwarn!("No virtio devices found");
    }
}

/// Host-side unit tests (`cargo test --workspace`).
///
/// Device matching and the virtqueue ring bookkeeping are pure logic; the MMIO
/// paths (`kick`, the PCI transport) need real hardware and stay out.
#[cfg(test)]
mod host_tests {
    use super::{
        device_name, is_virtio_device, match_device, MAX_QUEUES, QUEUE_SIZE, VIRTIO_DEVICE_BALLOON,
        VIRTIO_DEVICE_BLOCK, VIRTIO_DEVICE_CONSOLE, VIRTIO_DEVICE_NET, VIRTIO_TRANS_BALLOON,
        VIRTIO_TRANS_BLOCK, VIRTIO_TRANS_CONSOLE, VIRTIO_TRANS_NET, VIRTIO_VENDOR_ID,
    };
    use crate::queue::{VirtioDesc, VirtioQueueMem, VRING_DESC_F_NEXT, VRING_DESC_F_WRITE};

    #[test]
    fn every_transitional_id_maps_to_its_driver() {
        for (id, name) in [
            (VIRTIO_DEVICE_NET, "virtio-net"),
            (VIRTIO_TRANS_NET, "virtio-net"),
            (VIRTIO_DEVICE_BLOCK, "virtio-blk"),
            (VIRTIO_TRANS_BLOCK, "virtio-blk"),
            (VIRTIO_DEVICE_CONSOLE, "virtio-console"),
            (VIRTIO_TRANS_CONSOLE, "virtio-console"),
            (VIRTIO_DEVICE_BALLOON, "virtio-balloon"),
            (VIRTIO_TRANS_BALLOON, "virtio-balloon"),
        ] {
            assert_eq!(match_device(id), Some(name), "id {id:#x}");
        }
    }

    #[test]
    fn transitional_and_modern_ids_are_distinct() {
        // A collision would let two different devices claim one driver.
        let ids = [
            VIRTIO_DEVICE_NET,
            VIRTIO_TRANS_NET,
            VIRTIO_DEVICE_BLOCK,
            VIRTIO_TRANS_BLOCK,
            VIRTIO_DEVICE_CONSOLE,
            VIRTIO_TRANS_CONSOLE,
            VIRTIO_DEVICE_BALLOON,
            VIRTIO_TRANS_BALLOON,
        ];
        for (i, a) in ids.iter().enumerate() {
            for b in ids.iter().skip(i + 1) {
                assert_ne!(a, b, "device ids {a:#x} and {b:#x} collide");
            }
        }
    }

    #[test]
    fn unknown_devices_are_reported_as_unknown() {
        assert_eq!(match_device(0xFFFF), None);
        assert_eq!(device_name(0xFFFF), "unknown");
        // Right device id, wrong vendor: not a virtio device.
        assert!(!is_virtio_device(0x8086, VIRTIO_DEVICE_NET));
        assert!(is_virtio_device(VIRTIO_VENDOR_ID, VIRTIO_DEVICE_NET));
        assert!(!is_virtio_device(VIRTIO_VENDOR_ID, 0xDEAD));
    }

    #[test]
    fn queue_geometry_matches_the_spec() {
        // 16-byte descriptors and a 4 KiB-aligned ring are what the split
        // virtqueue layout requires; if these change the driver silently stops
        // working against real hardware.
        assert_eq!(core::mem::size_of::<VirtioDesc>(), 16);
        assert_eq!(core::mem::align_of::<VirtioDesc>(), 16);
        assert_eq!(core::mem::align_of::<VirtioQueueMem>(), 4096);
        assert_eq!(QUEUE_SIZE, 256);
        assert_eq!(MAX_QUEUES, 8);
        assert_eq!(VRING_DESC_F_NEXT, 1);
        assert_eq!(VRING_DESC_F_WRITE, 2);
    }

    #[test]
    fn a_fresh_queue_starts_empty() {
        let mem = VirtioQueueMem::new();
        assert_eq!(mem.avail.idx, 0);
        assert_eq!(mem.used.idx, 0);
        assert_eq!(mem.avail.flags, 0);
        assert!(mem.desc.iter().all(|d| d.flags == 0 && d.len == 0));
    }
}
