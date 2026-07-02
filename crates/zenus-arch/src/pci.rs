use x86_64::instructions::port::Port;

#[derive(Debug, Clone, Copy)]
pub struct PciDevice {
    pub bus: u8,
    pub device: u8,
    pub function: u8,
    pub vendor_id: u16,
    pub device_id: u16,
    pub class_code: u8,
    pub subclass: u8,
    pub prog_if: u8,
    pub revision: u8,
    pub header_type: u8,
    pub bar0: u32,
    pub bar1: u32,
    pub bar2: u32,
    pub bar3: u32,
    pub bar4: u32,
    pub bar5: u32,
    pub interrupt_line: u8,
    pub interrupt_pin: u8,
}

pub const MAX_PCI_DEVICES: usize = 256;
pub static mut PCI_DEVICES: [PciDevice; MAX_PCI_DEVICES] = unsafe { core::mem::zeroed() };
static mut PCI_COUNT: usize = 0;

const PCI_CONFIG_ADDR: u16 = 0xCF8;
const PCI_CONFIG_DATA: u16 = 0xCFC;

unsafe fn pci_read_config(bus: u8, dev: u8, func: u8, offset: u8) -> u32 {
    let address: u32 = (1 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC);

    Port::new(PCI_CONFIG_ADDR).write(address);
    Port::new(PCI_CONFIG_DATA).read()
}

unsafe fn pci_write_config(bus: u8, dev: u8, func: u8, offset: u8, value: u32) {
    let address: u32 = (1 << 31)
        | ((bus as u32) << 16)
        | ((dev as u32) << 11)
        | ((func as u32) << 8)
        | (offset as u32 & 0xFC);

    Port::new(PCI_CONFIG_ADDR).write(address);
    Port::new(PCI_CONFIG_DATA).write(value);
}

pub fn init() {
    let s = zenus_console::serial::SerialPort::new(0x3F8);
    s.write_str("[PCI] Scanning buses...\n");
    let count = unsafe { scan_all_buses() };
    s.write_str("[OK] PCI: ");
    s.write_u64(count as u64);
    s.write_str(" devices found\n");
}

unsafe fn scan_all_buses() -> usize {
    let header = pci_read_config(0, 0, 0, 0);

    if (header & 0x8000) == 0 {
        return 0;
    }

    let mut total = 0usize;
    // Check if bus 0 has multiple buses via PCI bridge
    let max_bus = if is_multi_bus_system() { 256 } else { 1 };
    for bus in 0..max_bus {
        total += scan_bus(bus as u8);
    }
    total
}

unsafe fn is_multi_bus_system() -> bool {
    // Check if there's a PCI-PCI bridge on bus 0
    for dev in 0..32 {
        let class_reg = pci_read_config(0, dev, 0, 8);
        let class_code = ((class_reg >> 24) & 0xFF) as u8;
        let subclass = ((class_reg >> 16) & 0xFF) as u8;
        if class_code == 0x06 && subclass == 0x04 {
            return true;
        }
        // Check multi-function devices that might be bridges
        let header_type = (pci_read_config(0, dev, 0, 0x0E) >> 16) as u8;
        if (header_type & 0x80) != 0 {
            for func in 1..8 {
                let f_vid_did = pci_read_config(0, dev, func, 0);
                if (f_vid_did & 0xFFFF) != 0xFFFF {
                    let f_class = (pci_read_config(0, dev, func, 8) >> 24) as u8;
                    let f_sub = (pci_read_config(0, dev, func, 8) >> 16) as u8;
                    if f_class == 0x06 && f_sub == 0x04 {
                        return true;
                    }
                }
            }
        }
    }
    false
}

unsafe fn scan_bus(bus: u8) -> usize {
    let mut devices = 0;
    for dev in 0..32 {
        let vid_did = pci_read_config(bus, dev, 0, 0);
        if (vid_did & 0xFFFF) != 0xFFFF {
            let header_type = (pci_read_config(bus, dev, 0, 0x0E) >> 16) as u8;

            let max_funcs = if (header_type & 0x80) != 0 { 8 } else { 1 };
            for func in 0..max_funcs {
                let f_vid_did = pci_read_config(bus, dev, func, 0);
                if (f_vid_did & 0xFFFF) != 0xFFFF {
                    let device = read_device(bus, dev, func);
                    if PCI_COUNT < MAX_PCI_DEVICES {
                        PCI_DEVICES[PCI_COUNT] = device;
                        PCI_COUNT += 1;
                        devices += 1;
                        log_device(&device);
                    }
                }
            }
        }
    }
    devices
}

unsafe fn read_device(bus: u8, dev: u8, func: u8) -> PciDevice {
    let vid_did = pci_read_config(bus, dev, func, 0);
    let class_rev = pci_read_config(bus, dev, func, 8);
    let bar0 = pci_read_config(bus, dev, func, 0x10);
    let bar1 = pci_read_config(bus, dev, func, 0x14);
    let bar2 = pci_read_config(bus, dev, func, 0x18);
    let bar3 = pci_read_config(bus, dev, func, 0x1C);
    let bar4 = pci_read_config(bus, dev, func, 0x20);
    let bar5 = pci_read_config(bus, dev, func, 0x24);
    let int_reg = pci_read_config(bus, dev, func, 0x3C);

    PciDevice {
        bus,
        device: dev,
        function: func,
        vendor_id: (vid_did & 0xFFFF) as u16,
        device_id: ((vid_did >> 16) & 0xFFFF) as u16,
        class_code: ((class_rev >> 24) & 0xFF) as u8,
        subclass: ((class_rev >> 16) & 0xFF) as u8,
        prog_if: ((class_rev >> 8) & 0xFF) as u8,
        revision: (class_rev & 0xFF) as u8,
        header_type: ((pci_read_config(bus, dev, func, 0x0E) >> 16) & 0xFF) as u8,
        bar0,
        bar1,
        bar2,
        bar3,
        bar4,
        bar5,
        interrupt_line: (int_reg & 0xFF) as u8,
        interrupt_pin: ((int_reg >> 8) & 0xFF) as u8,
    }
}

pub unsafe fn enable_bus_master(bus: u8, dev: u8, func: u8) {
    let cmd = pci_read_config(bus, dev, func, 0x04);
    pci_write_config(bus, dev, func, 0x04, cmd | 0x04);
}

fn log_device(dev: &PciDevice) {
    zenus_console::kinfo!("  PCI {:x}:{:x}.{:x}  {:x}:{:x}  Class {:x}:{:x}",
        dev.bus, dev.device, dev.function,
        dev.vendor_id, dev.device_id,
        dev.class_code, dev.subclass);
}
