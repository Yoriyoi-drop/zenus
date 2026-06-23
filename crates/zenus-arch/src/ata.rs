use x86_64::instructions::port::Port;
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;

const PRIMARY_IO: u16 = 0x1F0;
const PRIMARY_CTRL: u16 = 0x3F6;
const SECONDARY_IO: u16 = 0x170;
const SECONDARY_CTRL: u16 = 0x376;

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_READ: u8 = 0x20;
const CMD_WRITE: u8 = 0x30;
const CMD_FLUSH: u8 = 0xE7;

const STATUS_BSY: u8 = 0x80;
const STATUS_DRDY: u8 = 0x40;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

const SECTOR_SIZE: usize = 512;

static ATA_CHANNEL_LOCKS: [SpinLock<()>; 2] = [SpinLock::new(()), SpinLock::new(())];

#[derive(Clone, Copy)]
pub struct AtaDevice {
    io_base: u16,
    ctrl_base: u16,
    drive: u8,
    pub lba_sectors: u64,
    pub model: [u8; 40],
}

pub const MAX_ATA_DEVICES: usize = 4;
static ATA_DEVICES: zenus_sync::spinlock::SpinLock<[Option<AtaDevice>; MAX_ATA_DEVICES]> = zenus_sync::spinlock::SpinLock::new([None; MAX_ATA_DEVICES]);
static ATA_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

fn ata_wait_busy(io_base: u16) -> bool {
    for _ in 0..10000 {
        let status: u8 = unsafe { Port::new(io_base + 7).read() };
        if status & STATUS_BSY == 0 {
            return true;
        }
    }
    false
}

fn ata_wait_drq(io_base: u16) -> bool {
    for _ in 0..10000 {
        let status: u8 = unsafe { Port::new(io_base + 7).read() };
        if status & STATUS_BSY == 0 {
            if status & STATUS_ERR != 0 {
                return false;
            }
            if status & STATUS_DRQ != 0 {
                return true;
            }
        }
    }
    false
}

fn ata_select_drive(io_base: u16, drive: u8) {
    let selector: u8 = if drive == 0 { 0xE0 } else { 0xF0 };
    unsafe {
        Port::new(io_base + 6).write(selector);
    }
}

fn identify_drive(io_base: u16, ctrl_base: u16, drive: u8) -> Option<AtaDevice> {
    ata_select_drive(io_base, drive);

    unsafe {
        Port::<u8>::new(ctrl_base).write(0); // clear nIEN
    }

    if !ata_wait_busy(io_base) {
        return None;
    }

    unsafe {
        Port::<u8>::new(io_base + 2).write(0);
        Port::<u8>::new(io_base + 3).write(0);
        Port::<u8>::new(io_base + 4).write(0);
        Port::<u8>::new(io_base + 5).write(0);
        Port::<u8>::new(io_base + 7).write(CMD_IDENTIFY);
    }

    let status: u8 = unsafe { Port::new(io_base + 7).read() };
    if status == 0 {
        return None;
    }

    if !ata_wait_busy(io_base) {
        return None;
    }

    let lba: u8 = unsafe { Port::new(io_base + 4).read() };
    let lba_hi: u8 = unsafe { Port::new(io_base + 5).read() };
    if lba != 0 || lba_hi != 0 {
        return None;
    }

    if !ata_wait_drq(io_base) {
        return None;
    }

    let mut data = [0u16; 256];
    for word in data.iter_mut() {
        *word = unsafe { Port::new(io_base).read() };
    }

    let lba_sectors = ((data[61] as u64) << 16) | (data[60] as u64);
    if lba_sectors == 0 {
        return None;
    }

    let model = extract_model(&data);

    Some(AtaDevice { io_base, ctrl_base, drive: if drive == 0 { 0xE0 } else { 0xF0 }, lba_sectors, model })
}

fn extract_model(data: &[u16; 256]) -> [u8; 40] {
    let mut model = [0u8; 40];
    for i in 0..20 {
        let w = data[27 + i];
        model[i * 2] = (w & 0xFF) as u8;
        model[i * 2 + 1] = (w >> 8) as u8;
    }
    model
}

fn model_str(model: &[u8; 40]) -> &str {
    let end = model.iter().rposition(|&b| b != 0 && b != ' ' as u8)
        .map(|i| i + 1)
        .unwrap_or(0);
    core::str::from_utf8(&model[..end]).unwrap_or("<non-utf8>")
}

pub fn init() {
    let s = SerialPort::new(0x3F8);
    s.write_str("[ATA] Scanning IDE channels...\n");

    let channels = [
        (PRIMARY_IO, PRIMARY_CTRL, "primary"),
        (SECONDARY_IO, SECONDARY_CTRL, "secondary"),
    ];

    for &(io, ctrl, _name) in &channels {
        for drive in 0..2 {
            let _label = if drive == 0 { "master" } else { "slave" };
            if let Some(dev) = identify_drive(io, ctrl, drive) {
                let mut guard = ATA_DEVICES.lock();
                let idx = ATA_COUNT.load(core::sync::atomic::Ordering::Relaxed);
                if idx < MAX_ATA_DEVICES {
                    guard[idx] = Some(dev);
                    ATA_COUNT.store(idx + 1, core::sync::atomic::Ordering::Relaxed);
                }
            }
        }
    }

    let count = ATA_COUNT.load(core::sync::atomic::Ordering::Relaxed);
    if count > 0 {
        s.write_str("[OK] ATA: ");
        s.write_u64(count as u64);
        s.write_str(" drive(s) found\n");
    } else {
        s.write_str("[ATA] No drives found\n");
    }
}

pub fn device_count() -> usize {
    ATA_COUNT.load(core::sync::atomic::Ordering::Relaxed)
}

fn get_device_copy(dev_idx: usize) -> Option<AtaDevice> {
    let guard = ATA_DEVICES.lock();
    guard.get(dev_idx).and_then(|d| *d)
}

const MAX_RW_SECTORS: u16 = 256;

pub fn read_sectors(dev_idx: usize, lba: u64, count: u16, buf: &mut [u8]) -> bool {
    let dev = match get_device_copy(dev_idx) {
        Some(d) => d,
        None => return false,
    };

    let count = count.min(MAX_RW_SECTORS);

    if count == 0 || lba > dev.lba_sectors || dev.lba_sectors - lba < count as u64 {
        return false;
    }
    if buf.len() < (count as usize) * SECTOR_SIZE {
        return false;
    }

    let io_base = dev.io_base;
    let channel = if io_base == PRIMARY_IO { 0 } else { 1 };
    let _lock = ATA_CHANNEL_LOCKS[channel].lock();

    for sector in 0..count as u64 {
        let current_lba = lba + sector;
        let offset = sector as usize * SECTOR_SIZE;

        if !ata_wait_busy(io_base) {
            return false;
        }

        unsafe {
            Port::<u8>::new(io_base + 6).write(dev.drive | ((current_lba >> 24) as u8 & 0x0F));
            Port::<u8>::new(io_base + 1).write(0);
            Port::<u8>::new(io_base + 2).write(1);
            Port::<u8>::new(io_base + 3).write((current_lba & 0xFF) as u8);
            Port::<u8>::new(io_base + 4).write(((current_lba >> 8) & 0xFF) as u8);
            Port::<u8>::new(io_base + 5).write(((current_lba >> 16) & 0xFF) as u8);
            Port::<u8>::new(io_base + 7).write(CMD_READ);
        }

        if !ata_wait_drq(io_base) {
            return false;
        }

        for i in 0..256 {
            let word: u16 = unsafe { Port::new(io_base).read() };
            buf[offset + i * 2] = (word & 0xFF) as u8;
            buf[offset + i * 2 + 1] = (word >> 8) as u8;
        }
    }

    true
}

pub fn write_sectors(dev_idx: usize, lba: u64, count: u16, buf: &[u8]) -> bool {
    let dev = match get_device_copy(dev_idx) {
        Some(d) => d,
        None => return false,
    };

    let count = count.min(MAX_RW_SECTORS);

    if count == 0 || lba > dev.lba_sectors || dev.lba_sectors - lba < count as u64 {
        return false;
    }
    if buf.len() < (count as usize) * SECTOR_SIZE {
        return false;
    }

    let io_base = dev.io_base;
    let channel = if io_base == PRIMARY_IO { 0 } else { 1 };
    let _lock = ATA_CHANNEL_LOCKS[channel].lock();

    for sector in 0..count as u64 {
        let current_lba = lba + sector;
        let offset = sector as usize * SECTOR_SIZE;

        if !ata_wait_busy(io_base) {
            return false;
        }

        unsafe {
            Port::<u8>::new(io_base + 6).write(dev.drive | ((current_lba >> 24) as u8 & 0x0F));
            Port::<u8>::new(io_base + 1).write(0);
            Port::<u8>::new(io_base + 2).write(1);
            Port::<u8>::new(io_base + 3).write((current_lba & 0xFF) as u8);
            Port::<u8>::new(io_base + 4).write(((current_lba >> 8) & 0xFF) as u8);
            Port::<u8>::new(io_base + 5).write(((current_lba >> 16) & 0xFF) as u8);
            Port::<u8>::new(io_base + 7).write(CMD_WRITE);
        }

        if !ata_wait_drq(io_base) {
            return false;
        }

        for i in 0..256 {
            let word = (buf[offset + i * 2] as u16) | ((buf[offset + i * 2 + 1] as u16) << 8);
            unsafe { Port::new(io_base).write(word); }
        }

        if !ata_wait_busy(io_base) {
            return false;
        }
    }

    unsafe { Port::<u8>::new(io_base + 7).write(CMD_FLUSH); }
    ata_wait_busy(io_base)
}

pub fn get_device(dev_idx: usize) -> Option<AtaDevice> {
    get_device_copy(dev_idx)
}
