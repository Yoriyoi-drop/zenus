//! AHCI SATA driver.
//!
//! Discovers AHCI controllers via PCI (class 0x01, subclass 0x06, prog_if 0x01),
//! enumerates ports, and issues IDENTIFY / READ / WRITE commands through the
//! AHCI command list interface. Device info is stored internally and exposed
//! via `device_count()`/`get_device()`; callers (apps/src/lib.rs) register
//! block devices through devfs.

#![allow(clippy::missing_safety_doc)]

use core::sync::atomic::{AtomicU32, Ordering};
use zenus_sync::spinlock::SpinLock;
use alloc::vec::Vec;

// Store ABAR per port index so read/write can find the controller.
static AHCI_ABAR: SpinLock<[Option<u32>; MAX_PORTS]> =
    SpinLock::new([None; MAX_PORTS]);

// ── AHCI constants ──────────────────────────────────────────────────────────

const HBA_PORT_DEV_PRESENT: u32 = 0x3;
const HBA_PORT_IPM_ACTIVE: u32 = 0x1;
const HBA_P0CMD_ST: u32 = 0x0001;  // start
const HBA_P0CMD_FRE: u32 = 0x0010;  // enable FIS receive
const HBA_P0CMD_FR: u32 = 0x4000;  // FIS reset
#[allow(dead_code)]
const HBA_P0IS_TFES: u32 = 1 << 30; // task file error
#[allow(dead_code)]
const HBA_P0IS_PCS: u32 = 1 << 27;  // port connect change status

const SECTOR_SIZE: usize = 512;

const ATA_CMD_IDENTIFY: u8 = 0xEC;
const ATA_CMD_READ_DMA_EXT: u8 = 0x25;
const ATA_CMD_WRITE_DMA_EXT: u8 = 0x35;

const MAX_PORTS: usize = 32;
const MAX_AHCI_DEVICES: usize = 8;

// ── AHCI register structures ────────────────────────────────────────────────

#[repr(C)]
struct HbaMem {
    cap: u32,
    ghc: u32,
    is: u32,
    pi: u32,
    _reserved: [u8; 0x7C - 0x2C], // 0x2C to 0x7C
    vendor: [u8; 0xD4 - 0x7C],    // 0x7C to 0xD4
}

#[repr(C)]
struct HbaPort {
    clb: u32,
    clbu: u32,
    fb: u32,
    fbu: u32,
    is: u32,
    ie: u32,
    cmd: u32,
    _reserved0: u32,
    tfd: u32,
    sig: u32,
    ssts: u32,
    sctl: u32,
    serr: u32,
    sact: u32,
    ci: u32,
    _reserved1: [u8; 128 - 64],
    _reserved2: [u8; 256 - 128],
    _reserved3: [u8; 384 - 256],
    _reserved4: [u8; 512 - 384],
    _reserved5: [u8; 640 - 512],
    _reserved6: [u8; 768 - 640],
    _reserved7: [u8; 896 - 768],
    _reserved8: [u8; 1024 - 896],
}

// ── Command header and table ────────────────────────────────────────────────

#[repr(C, packed)]
struct HbaCmdHeader {
    dw0: u32,
    dw1: u32,
    dw2: u32,
    dw3: u32,
}

#[repr(C, packed)]
struct HbaCmdTbl {
    cfis: [u8; 64],
    acmd: [u8; 16],
    _reserved: [u8; 48],
    prdt_entry: [HbaPrdtEntry; 1],
}

#[repr(C, packed)]
struct HbaPrdtEntry {
    dba: u32,
    dbau: u32,
    _reserved: u32,
    dw3: u32,
}

// ── Device tracking ─────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
pub struct AhciDevice {
    pub port_idx: u8,
    pub lba_sectors: u64,
    pub model: [u8; 40],
}

static AHCI_DEVICES: SpinLock<[Option<AhciDevice>; MAX_AHCI_DEVICES]> =
    SpinLock::new([None; MAX_AHCI_DEVICES]);
static AHCI_COUNT: AtomicU32 = AtomicU32::new(0);

// ── Memory helpers ──────────────────────────────────────────────────────────

fn hba_mem(abar: u32) -> *mut HbaMem {
    abar as *mut HbaMem
}

fn hba_port(abar: u32, port: usize) -> *mut HbaPort {
    unsafe { hba_mem(abar).add(port) as *mut HbaPort }
}

// ── Port operations ─────────────────────────────────────────────────────────

unsafe fn port_start(abar: u32, port: usize) {
    let p = hba_port(abar, port);
    (*p).cmd |= HBA_P0CMD_FRE;
    (*p).cmd |= HBA_P0CMD_ST;
}

#[allow(clippy::while_immutable_condition)]
unsafe fn port_stop(abar: u32, port: usize) {
    let p = hba_port(abar, port);
    (*p).cmd &= !HBA_P0CMD_ST;
    (*p).cmd &= !HBA_P0CMD_FRE;
    while (*p).cmd & HBA_P0CMD_FR != 0 {
        x86_64::instructions::hlt();
    }
}

// ── PCI discovery ───────────────────────────────────────────────────────────

/// Find AHCI controllers already enumerated by the PCI subsystem.
pub fn discover_ahci_controllers() -> Vec<(u32, u8)> {
    let mut result: Vec<(u32, u8)> = Vec::new();
    let pci_count = crate::pci::get_device_count();
    for i in 0..pci_count {
        if let Some(dev) = crate::pci::get_device(i) {
            if dev.class_code == 0x01 && dev.subclass == 0x06 && dev.prog_if == 0x01 {
                let abar = dev.bar5 & !0xF;
                if abar != 0 {
                    result.push((abar, dev.interrupt_line));
                }
            }
        }
    }
    result
}

// ── Port probe and init ─────────────────────────────────────────────────────

unsafe fn probe_ports(abar: u32) -> u32 {
    let hba = hba_mem(abar);
    let nports = (((*hba).cap >> 8) & 0x1F) as usize;
    let mut count = 0u32;

    for port in 0..nports.min(MAX_PORTS) {
        let p = hba_port(abar, port);
        let ssts = (*p).ssts;
        let ipm = (ssts >> 8) & 0x0F;
        let det = ssts & 0x0F;
        if det != HBA_PORT_DEV_PRESENT || ipm != HBA_PORT_IPM_ACTIVE {
            continue;
        }

        let sig = (*p).sig;
        match sig {
            0x0000_0101 => {}
            0xEB14_0101 => continue,
            0x0000_0000 => continue,
            0x9669_0101 => continue,
            _ => continue,
        }

        count += 1;
        let _ = init_port(abar, port);
    }
    count
}

unsafe fn init_port(abar: u32, port: usize) -> bool {
    let p = hba_port(abar, port);

    port_stop(abar, port);

    static mut CMD_MEM: [u8; 4096 * MAX_PORTS] = [0; 4096 * MAX_PORTS];
    let cmd_mem = &mut CMD_MEM[port * 4096..(port + 1) * 4096];

    let clb = cmd_mem.as_ptr() as u32;
    let clbu = 0;
    let fb = clb + 1024;
    let fbu = 0;

    (*p).clb = clb;
    (*p).clbu = clbu;
    (*p).fb = fb;
    (*p).fbu = fbu;

    port_start(abar, port);

    let mut identify_buf = [0u8; SECTOR_SIZE];
    let _ = ata_identify(abar, port, &mut identify_buf);

    true
}

#[allow(clippy::while_immutable_condition)]
unsafe fn ata_identify(abar: u32, port: usize, buf: &mut [u8]) -> bool {
    let p = hba_port(abar, port);

    static mut CMD_TBL_MEM: [u8; 4096 * MAX_PORTS] = [0; 4096 * MAX_PORTS];
    let tbl = &mut CMD_TBL_MEM[port * 4096..(port + 1) * 4096];

    let cmd_header = &mut *(tbl.as_mut_ptr() as *mut HbaCmdHeader);
    cmd_header.dw0 = 5;
    cmd_header.dw1 = tbl.as_ptr() as u32 + core::mem::size_of::<HbaCmdTbl>() as u32;
    cmd_header.dw2 = 0;

    let cmd_tbl = &mut *(tbl.as_mut_ptr().add(core::mem::size_of::<HbaCmdTbl>()) as *mut HbaCmdTbl);

    cmd_tbl.cfis[0] = 0x27;
    cmd_tbl.cfis[1] = 0x80;
    cmd_tbl.cfis[2] = ATA_CMD_IDENTIFY;
    cmd_tbl.cfis[3] = 0;

    let prdt = &mut cmd_tbl.prdt_entry[0];
    prdt.dba = buf.as_ptr() as u32;
    prdt.dbau = 0;
    prdt._reserved = 0;
    prdt.dw3 = SECTOR_SIZE as u32 | (1 << 31);

    (*p).ci = 1 << port;

    let mut ticks = 0;
    while (*p).ci & (1 << port) != 0 {
        x86_64::instructions::hlt();
        ticks += 1;
        if ticks > 1_000_000 {
            return false;
        }
    }

    (*p).tfd & 0x01 == 0
}

#[allow(clippy::while_immutable_condition)]
unsafe fn read_sectors(abar: u32, port: usize, lba: u64, count: u32, buf: &mut [u8]) -> bool {
    let p = hba_port(abar, port);

    static mut CMD_TBL_MEM: [u8; 4096 * MAX_PORTS] = [0; 4096 * MAX_PORTS];
    let tbl = &mut CMD_TBL_MEM[port * 4096..(port + 1) * 4096];

    let cmd_header = &mut *(tbl.as_mut_ptr() as *mut HbaCmdHeader);
    cmd_header.dw0 = 5;
    cmd_header.dw1 = tbl.as_ptr() as u32 + core::mem::size_of::<HbaCmdTbl>() as u32;
    cmd_header.dw2 = 0;

    let cmd_tbl = &mut *(tbl.as_mut_ptr().add(core::mem::size_of::<HbaCmdTbl>()) as *mut HbaCmdTbl);

    cmd_tbl.cfis[0] = 0x27;
    cmd_tbl.cfis[1] = 0x80;
    cmd_tbl.cfis[2] = ATA_CMD_READ_DMA_EXT;
    cmd_tbl.cfis[3] = 0;
    cmd_tbl.cfis[4] = lba as u8;
    cmd_tbl.cfis[5] = (lba >> 8) as u8;
    cmd_tbl.cfis[6] = (lba >> 16) as u8;
    cmd_tbl.cfis[7] = (lba >> 24) as u8;
    cmd_tbl.cfis[8] = (lba >> 32) as u8;
    cmd_tbl.cfis[9] = (lba >> 40) as u8;
    cmd_tbl.cfis[12] = count as u8;
    cmd_tbl.cfis[13] = (count >> 8) as u8;

    let prdt = &mut cmd_tbl.prdt_entry[0];
    prdt.dba = buf.as_ptr() as u32;
    prdt.dbau = 0;
    prdt._reserved = 0;
    prdt.dw3 = (count * SECTOR_SIZE as u32) | (1 << 31);

    (*p).ci = 1 << port;

    let mut ticks = 0;
    while (*p).ci & (1 << port) != 0 {
        x86_64::instructions::hlt();
        ticks += 1;
        if ticks > 10_000_000 {
            return false;
        }
    }

    (*p).tfd & 0x01 == 0
}

#[allow(clippy::while_immutable_condition)]
unsafe fn write_sectors(abar: u32, port: usize, lba: u64, count: u32, buf: &[u8]) -> bool {
    let p = hba_port(abar, port);

    static mut CMD_TBL_MEM: [u8; 4096 * MAX_PORTS] = [0; 4096 * MAX_PORTS];
    let tbl = &mut CMD_TBL_MEM[port * 4096..(port + 1) * 4096];

    let cmd_header = &mut *(tbl.as_mut_ptr() as *mut HbaCmdHeader);
    cmd_header.dw0 = 5;
    cmd_header.dw1 = tbl.as_ptr() as u32 + core::mem::size_of::<HbaCmdTbl>() as u32;
    cmd_header.dw2 = 0;

    let cmd_tbl = &mut *(tbl.as_mut_ptr().add(core::mem::size_of::<HbaCmdTbl>()) as *mut HbaCmdTbl);

    cmd_tbl.cfis[0] = 0x27;
    cmd_tbl.cfis[1] = 0x80;
    cmd_tbl.cfis[2] = ATA_CMD_WRITE_DMA_EXT;
    cmd_tbl.cfis[3] = 0;
    cmd_tbl.cfis[4] = lba as u8;
    cmd_tbl.cfis[5] = (lba >> 8) as u8;
    cmd_tbl.cfis[6] = (lba >> 16) as u8;
    cmd_tbl.cfis[7] = (lba >> 24) as u8;
    cmd_tbl.cfis[8] = (lba >> 32) as u8;
    cmd_tbl.cfis[9] = (lba >> 40) as u8;
    cmd_tbl.cfis[12] = count as u8;
    cmd_tbl.cfis[13] = (count >> 8) as u8;

    let prdt = &mut cmd_tbl.prdt_entry[0];
    prdt.dba = buf.as_ptr() as u32;
    prdt.dbau = 0;
    prdt._reserved = 0;
    prdt.dw3 = (count * SECTOR_SIZE as u32) | (1 << 31);

    (*p).ci = 1 << port;

    let mut ticks = 0;
    while (*p).ci & (1 << port) != 0 {
        x86_64::instructions::hlt();
        ticks += 1;
        if ticks > 10_000_000 {
            return false;
        }
    }

    (*p).tfd & 0x01 == 0
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Initialize AHCI: discover controllers, probe ports, store device info.
pub fn init() {
    let controllers = discover_ahci_controllers();
    if controllers.is_empty() {
        return;
    }

    zenus_console::kwarn!("[AHCI] {} controller(s) found", controllers.len());

    for (abar, _irq) in controllers {
        let found = unsafe { probe_ports(abar) };
        if found > 0 {
            zenus_console::kwarn!("[AHCI] {} device(s) on controller ABAR={:#x}", found, abar);
        }

        // Register devices found on this controller
        register_ahci_devices(abar);
    }
}

/// Number of AHCI block devices found.
pub fn device_count() -> u8 {
    AHCI_COUNT.load(Ordering::Relaxed) as u8
}

/// Get device info by index.
pub fn get_device(idx: usize) -> Option<AhciDevice> {
    let state = AHCI_DEVICES.lock();
    state[idx]
}

/// Read one sector from an AHCI device by index.
pub fn read(idx: usize, lba: u64, buf: &mut [u8]) -> bool {
    let (dev, abar) = {
        let state = AHCI_DEVICES.lock();
        let abar_state = AHCI_ABAR.lock();
        match state[idx] {
            Some(d) => {
                let abar = abar_state[d.port_idx as usize].unwrap_or(0);
                (d, abar)
            }
            None => return false,
        }
    };
    if abar == 0 || buf.len() < SECTOR_SIZE {
        return false;
    }
    let mut sector_buf = [0u8; SECTOR_SIZE];
    let ok = unsafe { read_sectors(abar, dev.port_idx as usize, lba, 1, &mut sector_buf) };
    if ok {
        buf[..SECTOR_SIZE].copy_from_slice(&sector_buf);
    }
    ok
}

/// Write one sector to an AHCI device by index.
pub fn write(idx: usize, lba: u64, buf: &[u8]) -> bool {
    let (dev, abar) = {
        let state = AHCI_DEVICES.lock();
        let abar_state = AHCI_ABAR.lock();
        match state[idx] {
            Some(d) => {
                let abar = abar_state[d.port_idx as usize].unwrap_or(0);
                (d, abar)
            }
            None => return false,
        }
    };
    if abar == 0 || buf.len() < SECTOR_SIZE {
        return false;
    }
    let mut sector_buf = [0u8; SECTOR_SIZE];
    sector_buf[..SECTOR_SIZE].copy_from_slice(&buf[..SECTOR_SIZE]);
    unsafe { write_sectors(abar, dev.port_idx as usize, lba, 1, &sector_buf) }
}

fn register_ahci_devices(abar: u32) {
    unsafe {
        let hba = hba_mem(abar);
        let nports = (((*hba).cap >> 8) & 0x1F) as usize;
        for port in 0..nports.min(MAX_PORTS) {
            let p = hba_port(abar, port);
            let ssts = (*p).ssts;
            let det = ssts & 0x0F;
            let ipm = (ssts >> 8) & 0x0F;
            if det != HBA_PORT_DEV_PRESENT || ipm != HBA_PORT_IPM_ACTIVE {
                continue;
            }
            let sig = (*p).sig;
            if sig != 0x0000_0101 {
                continue;
            }

            let idx = AHCI_COUNT.fetch_add(1, Ordering::SeqCst) as usize;
            if idx >= MAX_AHCI_DEVICES {
                AHCI_COUNT.fetch_sub(1, Ordering::SeqCst);
                continue;
            }

            AHCI_ABAR.lock()[port] = Some(abar);

            let mut buf = [0u8; SECTOR_SIZE];
            let identified = ata_identify(abar, port, &mut buf);
            let mut model = [0u8; 40];
            let mut lba_sectors: u64 = 0;

            if identified {
                model.copy_from_slice(&buf[54..94]);
                let word83 = u16::from_le_bytes([buf[166], buf[167]]);
                if word83 & 0x0400 != 0 {
                    lba_sectors = u32::from_le_bytes([buf[200], buf[201], buf[202], buf[203]]) as u64
                        | (u32::from_le_bytes([buf[204], buf[205], buf[206], buf[207]]) as u64) << 32;
                } else {
                    lba_sectors = u32::from_le_bytes([buf[100], buf[101], buf[102], buf[103]]) as u64;
                }
            }

            let dev = AhciDevice {
                port_idx: port as u8,
                lba_sectors,
                model,
            };
            AHCI_DEVICES.lock()[idx] = Some(dev);
        }
    }
}

#[cfg(test)]
mod host_tests {
    #[test]
    fn ahci_port_is_active_checks_det_and_ipm() {
        let det = 0x03;
        let ipm = 0x01;
        assert_eq!(det, super::HBA_PORT_DEV_PRESENT);
        assert_eq!(ipm, super::HBA_PORT_IPM_ACTIVE);
    }

    #[test]
    fn lba48_bit_is_word_83_bit_10() {
        assert_eq!(1 << 10, 0x0400);
    }
}
