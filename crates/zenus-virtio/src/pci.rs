use core::ptr;
use zenus_arch::pci::PciDevice;
use zenus_console::serial::SerialPort;
use zenus_mem::paging;
use crate::{serial, VIRTIO_VENDOR_ID};

pub const PCI_CAP_ID_VNDR: u8 = 0x09;

pub const VIRTIO_PCI_CAP_COMMON: u8 = 1;
pub const VIRTIO_PCI_CAP_NOTIFY: u8 = 2;
pub const VIRTIO_PCI_CAP_ISR: u8 = 3;
pub const VIRTIO_PCI_CAP_DEVICE: u8 = 4;
pub const VIRTIO_PCI_CAP_PCI_CFG: u8 = 5;

#[derive(Clone, Copy)]
pub struct VirtioPciCap {
    pub cfg_type: u8,
    pub bar: u8,
    pub offset: u32,
    pub length: u32,
}

pub struct VirtioPciTransport {
    pub dev: PciDevice,
    pub common_base: u64,
    pub notify_base: u64,
    pub notify_off_multiplier: u32,
    pub isr_base: u64,
    pub device_base: u64,
}

unsafe fn map_bar(bar_val: u32) -> u64 {
    let hhdm = paging::hhdm_offset();
    if bar_val & 1 == 1 {
        (bar_val & 0xFFFC) as u64 + hhdm
    } else {
        (bar_val & 0xFFFFFFF0) as u64 + hhdm
    }
}

unsafe fn read_bar_phys(dev: &PciDevice, bar_idx: u8) -> u64 {
    let raw = match bar_idx {
        0 => dev.bar0, 1 => dev.bar1, 2 => dev.bar2,
        3 => dev.bar3, 4 => dev.bar4, 5 => dev.bar5,
        _ => return 0,
    };
    if raw & 1 == 1 {
        (raw & 0xFFFC) as u64
    } else {
        (raw & 0xFFFFFFF0) as u64
    }
}

unsafe fn pci_read_config(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    use x86_64::instructions::port::Port;
    let address: u32 = (1 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC);
    Port::new(0xCF8).write(address);
    Port::new(0xCFC).read()
}

unsafe fn pci_read_cap_bytes(dev: &PciDevice, cap_offset: u8, buf: &mut [u8]) {
    for (i, b) in buf.iter_mut().enumerate() {
        let word = pci_read_config(dev.bus, dev.device, dev.function, cap_offset + i as u8);
        *b = ((word >> ((i as u8 % 4) * 8)) & 0xFF) as u8;
    }
}

fn find_virtio_caps(dev: &PciDevice) -> [Option<VirtioPciCap>; 6] {
    let mut caps: [Option<VirtioPciCap>; 6] = [None; 6];
    unsafe {
        let status = pci_read_config(dev.bus, dev.device, dev.function, 0x06);
        if (status >> 4) & 1 == 0 {
            return caps;
        }
        let cap_ptr_reg = pci_read_config(dev.bus, dev.device, dev.function, 0x34);
        let mut cap_off = (cap_ptr_reg & 0xFF) as u8;
        if cap_off < 0x40 {
            return caps;
        }
        loop {
            let cap_id = (pci_read_config(dev.bus, dev.device, dev.function, cap_off) & 0xFF) as u8;
            if cap_id == 0 {
                break;
            }
            if cap_id == PCI_CAP_ID_VNDR {
                let mut hdr = [0u8; 16];
                pci_read_cap_bytes(dev, cap_off, &mut hdr);
                let cap = VirtioPciCap {
                    cfg_type: hdr[3],
                    bar: hdr[4],
                    offset: u32::from_le_bytes([hdr[8], hdr[9], hdr[10], hdr[11]]),
                    length: u32::from_le_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]),
                };
                let idx = cap.cfg_type as usize;
                if idx < 6 {
                    caps[idx] = Some(cap);
                }
            }
            let next = (pci_read_config(dev.bus, dev.device, dev.function, cap_off + 1) & 0xFF) as u8;
            if next == 0 || next < 0x40 {
                break;
            }
            cap_off = next;
        }
    }
    caps
}

pub unsafe fn init_device(dev: &PciDevice) -> Option<VirtioPciTransport> {
    let s = serial();
    let caps = find_virtio_caps(dev);

    let common = caps[VIRTIO_PCI_CAP_COMMON as usize]?;
    let notify = caps[VIRTIO_PCI_CAP_NOTIFY as usize]?;
    let isr = caps[VIRTIO_PCI_CAP_ISR as usize]?;

    let hhdm = paging::hhdm_offset();

    let bar_phys = read_bar_phys(dev, common.bar);

    let common_base = bar_phys + common.offset as u64 + hhdm;
    let notify_base = bar_phys + notify.offset as u64 + hhdm;
    let isr_base = bar_phys + isr.offset as u64 + hhdm;
    let device_base = caps[VIRTIO_PCI_CAP_DEVICE as usize]
        .map(|d| bar_phys + d.offset as u64 + hhdm)
        .unwrap_or(0);

    let notify_off_multiplier = if notify.length >= 4 {
        ptr::read_volatile((notify_base) as *const u32)
    } else {
        0
    };

    let trans = VirtioPciTransport {
        dev: *dev,
        common_base,
        notify_base,
        notify_off_multiplier: notify_off_multiplier.wrapping_add(2),
        isr_base,
        device_base,
    };

    s.write_str("[VIRTIO] PCI device at ");
    s.write_hex(dev.bus as u64);
    s.write_str(":");
    s.write_hex(dev.device as u64);
    s.write_str(".");
    s.write_hex(dev.function as u64);
    s.write_str(" (0x");
    s.write_hex(dev.device_id as u64);
    s.write_str(")\n");
    s.write_str("[VIRTIO] common=0x");
    s.write_hex(common_base);
    s.write_str(" notify=0x");
    s.write_hex(notify_base);
    s.write_str(" isr=0x");
    s.write_hex(isr_base);
    s.write_str("\n");

    Some(trans)
}

impl VirtioPciTransport {
    unsafe fn common_read8(&self, offset: u16) -> u8 {
        ptr::read_volatile((self.common_base + offset as u64) as *const u8)
    }

    unsafe fn common_read16(&self, offset: u16) -> u16 {
        ptr::read_volatile((self.common_base + offset as u64) as *const u16)
    }

    unsafe fn common_read32(&self, offset: u16) -> u32 {
        ptr::read_volatile((self.common_base + offset as u64) as *const u32)
    }

    unsafe fn common_write32(&self, offset: u16, val: u32) {
        ptr::write_volatile((self.common_base + offset as u64) as *mut u32, val);
    }

    unsafe fn common_write16(&self, offset: u16, val: u16) {
        ptr::write_volatile((self.common_base + offset as u64) as *mut u16, val);
    }

    unsafe fn common_write8(&self, offset: u16, val: u8) {
        ptr::write_volatile((self.common_base + offset as u64) as *mut u8, val);
    }

    pub unsafe fn device_read8(&self, offset: u16) -> u8 {
        if self.device_base == 0 { return 0; }
        ptr::read_volatile((self.device_base + offset as u64) as *const u8)
    }

    unsafe fn device_read16(&self, offset: u16) -> u16 {
        if self.device_base == 0 { return 0; }
        ptr::read_volatile((self.device_base + offset as u64) as *const u16)
    }

    unsafe fn device_read32(&self, offset: u16) -> u32 {
        if self.device_base == 0 { return 0; }
        ptr::read_volatile((self.device_base + offset as u64) as *const u32)
    }

    unsafe fn device_write8(&self, offset: u16, val: u8) {
        if self.device_base == 0 { return; }
        ptr::write_volatile((self.device_base + offset as u64) as *mut u8, val);
    }

    unsafe fn device_write32(&self, offset: u16, val: u32) {
        if self.device_base == 0 { return; }
        ptr::write_volatile((self.device_base + offset as u64) as *mut u32, val);
    }

    pub unsafe fn device_status(&self) -> u8 {
        self.common_read8(4)
    }

    pub unsafe fn set_device_status(&self, status: u8) {
        self.common_write8(4, status);
    }

    pub unsafe fn reset(&self) {
        self.set_device_status(0);
        while self.device_status() != 0 {
            core::hint::spin_loop();
        }
    }

    pub unsafe fn negotiate_features(&self, device_features: u64) -> u64 {
        self.common_write32(0x10, (device_features & 0xFFFFFFFF) as u32);
        self.common_write32(0x14, ((device_features >> 32) & 0xFFFFFFFF) as u32);
        let lo = self.common_read32(0x10);
        let hi = self.common_read32(0x14);
        (hi as u64) << 32 | lo as u64
    }

    pub unsafe fn setup_queue(&self, queue_idx: u16, desc_phys: u64, avail_phys: u64, used_phys: u64) -> u16 {
        self.common_write16(0x08, queue_idx);
        let size = self.common_read16(0x06);
        if size == 0 {
            return 0;
        }
        self.common_write64(0x18, desc_phys);
        self.common_write16(0x1e, 0);
        self.common_write16(0x20, 0);
        self.common_write16(0x22, 0);
        self.common_write16(0x24, 0);
        self.common_write32(0x28, avail_phys as u32);
        self.common_write32(0x2c, (avail_phys >> 32) as u32);
        self.common_write32(0x30, used_phys as u32);
        self.common_write32(0x34, (used_phys >> 32) as u32);
        self.common_write16(0x08, queue_idx);
        let enabled = self.common_read16(0x0e);
        if enabled == 0 {
            0
        } else {
            size
        }
    }

    pub unsafe fn queue_notify(&self, queue_idx: u16) {
        ptr::write_volatile(
            (self.notify_base) as *mut u16,
            queue_idx,
        );
    }

    pub unsafe fn read_isr(&self) -> u8 {
        ptr::read_volatile(self.isr_base as *const u8)
    }

    unsafe fn common_write64(&self, offset: u16, val: u64) {
        self.common_write32(offset, (val & 0xFFFFFFFF) as u32);
        self.common_write32(offset + 4, ((val >> 32) & 0xFFFFFFFF) as u32);
    }

    pub fn read_device_features(&self) -> u64 {
        unsafe {
            let lo = self.common_read32(0x00);
            let hi = self.common_read32(0x04);
            (hi as u64) << 32 | lo as u64
        }
    }

    pub unsafe fn get_queue_size(&self, queue_idx: u16) -> u16 {
        self.common_write16(8, queue_idx);
        self.common_read16(6)
    }

    pub unsafe fn get_device_config_space(&self) -> u64 {
        self.device_base
    }
}
