use x86_64::instructions::port::Port;
use zenus_sync::spinlock::SpinLock;

const PRIMARY_IO: u16 = 0x1F0;
const PRIMARY_CTRL: u16 = 0x3F6;
const SECONDARY_IO: u16 = 0x170;
const SECONDARY_CTRL: u16 = 0x376;

const CMD_IDENTIFY: u8 = 0xEC;
const CMD_READ28: u8 = 0x20;
const CMD_WRITE28: u8 = 0x30;
const CMD_READ48: u8 = 0x24;
const CMD_WRITE48: u8 = 0x34;
const CMD_FLUSH: u8 = 0xE7;
const CMD_FLUSH48: u8 = 0xEA;

const STATUS_BSY: u8 = 0x80;
#[allow(dead_code)]
const STATUS_DRDY: u8 = 0x40;
const STATUS_DRQ: u8 = 0x08;
const STATUS_ERR: u8 = 0x01;

// IDENTIFY word 83 bit 10: LBA-48 supported
const IDENTIFY_LBA48_BIT: u16 = 1 << 10;

const SECTOR_SIZE: usize = 512;

static ATA_CHANNEL_LOCKS: [SpinLock<()>; 2] = [SpinLock::new(()), SpinLock::new(())];

#[derive(Clone, Copy)]
pub struct AtaDevice {
    io_base: u16,
    #[allow(dead_code)]
    ctrl_base: u16,
    drive: u8,
    pub lba_sectors: u64,
    pub model: [u8; 40],
    /// True jika drive mendukung LBA-48 (disk > 128 GiB)
    pub lba48: bool,
}

pub const MAX_ATA_DEVICES: usize = 4;
static ATA_DEVICES: SpinLock<[Option<AtaDevice>; MAX_ATA_DEVICES]> =
    SpinLock::new([None; MAX_ATA_DEVICES]);
static ATA_COUNT: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Tunggu BSY clear. Tidak menggunakan hlt() — aman dipanggil di dalam
/// spinlock atau handler interrupt.
fn ata_wait_busy(io_base: u16) -> bool {
    for _ in 0..100_000 {
        let status: u8 = unsafe { Port::new(io_base + 7).read() };
        if status & STATUS_BSY == 0 {
            return true;
        }
        core::hint::spin_loop();
    }
    false
}

/// Tunggu DRQ set (atau ERR). Tidak menggunakan hlt().
fn ata_wait_drq(io_base: u16) -> bool {
    for _ in 0..100_000 {
        let status: u8 = unsafe { Port::new(io_base + 7).read() };
        if status & STATUS_BSY == 0 {
            if status & STATUS_ERR != 0 {
                return false;
            }
            if status & STATUS_DRQ != 0 {
                return true;
            }
        }
        core::hint::spin_loop();
    }
    false
}

fn ata_select_drive(io_base: u16, drive: u8) {
    // drive == 0 → master (0xE0), drive == 1 → slave (0xF0)
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

    // Cek dukungan LBA-48 (word 83 bit 10)
    let lba48 = (data[83] & IDENTIFY_LBA48_BIT) != 0;

    let lba_sectors = if lba48 {
        // Word 100-103: total LBA-48 sectors (64-bit)
        (data[100] as u64)
            | ((data[101] as u64) << 16)
            | ((data[102] as u64) << 32)
            | ((data[103] as u64) << 48)
    } else {
        // Word 60-61: total LBA-28 sectors (28-bit)
        ((data[61] as u64) << 16) | (data[60] as u64)
    };

    if lba_sectors == 0 {
        return None;
    }

    let model = extract_model(&data);
    let drive_sel = if drive == 0 { 0xE0 } else { 0xF0 };

    Some(AtaDevice {
        io_base,
        ctrl_base,
        drive: drive_sel,
        lba_sectors,
        model,
        lba48,
    })
}

/// ATA string byte-swap: setiap word dari IDENTIFY disimpan sebagai
/// high-byte dulu, low-byte kedua (big-endian per karakter pasangan).
fn extract_model(data: &[u16; 256]) -> [u8; 40] {
    let mut model = [0u8; 40];
    for i in 0..20 {
        let w = data[27 + i];
        // ATA: byte tinggi adalah karakter pertama pasangan
        model[i * 2] = (w >> 8) as u8;
        model[i * 2 + 1] = (w & 0xFF) as u8;
    }
    model
}

#[allow(dead_code)]
fn model_str(model: &[u8; 40]) -> &str {
    let end = model
        .iter()
        .rposition(|&b| b != 0 && b != b' ')
        .map(|i| i + 1)
        .unwrap_or(0);
    core::str::from_utf8(&model[..end]).unwrap_or("<non-utf8>")
}

pub fn init() {
    zenus_console::kinfo!("Scanning IDE channels...");

    let channels = [
        (PRIMARY_IO, PRIMARY_CTRL, "primary"),
        (SECONDARY_IO, SECONDARY_CTRL, "secondary"),
    ];

    for &(io, ctrl, _name) in &channels {
        for drive in 0..2u8 {
            if let Some(dev) = identify_drive(io, ctrl, drive) {
                // Lock sekali dan update count di dalam lock untuk mencegah
                // race condition antara load, check, dan store.
                let mut guard = ATA_DEVICES.lock();
                let idx = ATA_COUNT.load(core::sync::atomic::Ordering::Relaxed);
                if idx < MAX_ATA_DEVICES {
                    guard[idx] = Some(dev);
                    ATA_COUNT.store(idx + 1, core::sync::atomic::Ordering::Relaxed);
                }
                // guard drop di sini — count sudah konsisten dengan array
            }
        }
    }

    let count = ATA_COUNT.load(core::sync::atomic::Ordering::Relaxed);
    if count > 0 {
        zenus_console::kinfo!("ATA: {} drive(s) found", count);
    } else {
        zenus_console::kinfo!("No drives found");
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

/// Tulis register LBA-28 ke port ATA.
unsafe fn setup_lba28(io_base: u16, drive: u8, lba: u64, count: u8) {
    Port::<u8>::new(io_base + 6).write(drive | ((lba >> 24) as u8 & 0x0F));
    Port::<u8>::new(io_base + 1).write(0);
    Port::<u8>::new(io_base + 2).write(count);
    Port::<u8>::new(io_base + 3).write((lba & 0xFF) as u8);
    Port::<u8>::new(io_base + 4).write(((lba >> 8) & 0xFF) as u8);
    Port::<u8>::new(io_base + 5).write(((lba >> 16) & 0xFF) as u8);
}

/// Tulis register LBA-48 ke port ATA (HOB dulu, lalu LOB).
unsafe fn setup_lba48(io_base: u16, drive: u8, lba: u64, count: u16) {
    // Bit 6 = LBA mode; tidak pakai bit 24-27 untuk LBA-48
    Port::<u8>::new(io_base + 6).write(drive | 0x40);
    // High-order bytes dulu
    Port::<u8>::new(io_base + 1).write(0);
    Port::<u8>::new(io_base + 2).write((count >> 8) as u8);
    Port::<u8>::new(io_base + 3).write(((lba >> 24) & 0xFF) as u8);
    Port::<u8>::new(io_base + 4).write(((lba >> 32) & 0xFF) as u8);
    Port::<u8>::new(io_base + 5).write(((lba >> 40) & 0xFF) as u8);
    // Low-order bytes
    Port::<u8>::new(io_base + 1).write(0);
    Port::<u8>::new(io_base + 2).write((count & 0xFF) as u8);
    Port::<u8>::new(io_base + 3).write((lba & 0xFF) as u8);
    Port::<u8>::new(io_base + 4).write(((lba >> 8) & 0xFF) as u8);
    Port::<u8>::new(io_base + 5).write(((lba >> 16) & 0xFF) as u8);
}

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

    // Pastikan LBA-28 tidak dipakai untuk alamat yang melebihi 28-bit
    if !dev.lba48 && lba >= (1u64 << 28) {
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
            if dev.lba48 {
                setup_lba48(io_base, dev.drive, current_lba, 1);
                Port::<u8>::new(io_base + 7).write(CMD_READ48);
            } else {
                setup_lba28(io_base, dev.drive, current_lba, 1);
                Port::<u8>::new(io_base + 7).write(CMD_READ28);
            }
        }

        if !ata_wait_drq(io_base) {
            return false;
        }

        unsafe {
            let ptr = buf.as_mut_ptr().add(offset);
            core::arch::asm!(
                "rep insw",
                in("dx") io_base,
                in("rcx") 256u64,
                inout("rdi") ptr => _,
                options(nostack, preserves_flags)
            );
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

    if !dev.lba48 && lba >= (1u64 << 28) {
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
            if dev.lba48 {
                setup_lba48(io_base, dev.drive, current_lba, 1);
                Port::<u8>::new(io_base + 7).write(CMD_WRITE48);
            } else {
                setup_lba28(io_base, dev.drive, current_lba, 1);
                Port::<u8>::new(io_base + 7).write(CMD_WRITE28);
            }
        }

        if !ata_wait_drq(io_base) {
            return false;
        }

        unsafe {
            let ptr = buf.as_ptr().add(offset);
            core::arch::asm!(
                "rep outsw",
                in("dx") io_base,
                in("rcx") 256u64,
                inout("rsi") ptr => _,
                options(nostack, preserves_flags)
            );
        }

        if !ata_wait_busy(io_base) {
            return false;
        }
    }

    // Gunakan FLUSH EXT untuk LBA-48, FLUSH biasa untuk LBA-28
    unsafe {
        let flush_cmd = if dev.lba48 { CMD_FLUSH48 } else { CMD_FLUSH };
        Port::<u8>::new(io_base + 7).write(flush_cmd);
    }
    ata_wait_busy(io_base)
}

pub fn get_device(dev_idx: usize) -> Option<AtaDevice> {
    get_device_copy(dev_idx)
}
