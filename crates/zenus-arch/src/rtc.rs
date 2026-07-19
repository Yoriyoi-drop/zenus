use x86_64::instructions::port::Port;
use zenus_sync::spinlock::SpinLock;
use core::sync::atomic::{AtomicU64, Ordering};

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

static BOOT_EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn boot_epoch() -> u64 {
    BOOT_EPOCH.load(Ordering::Relaxed)
}

const RTC_SECONDS: u8 = 0x00;
const RTC_MINUTES: u8 = 0x02;
const RTC_HOURS: u8 = 0x04;
const RTC_DAY: u8 = 0x07;
const RTC_MONTH: u8 = 0x08;
const RTC_YEAR: u8 = 0x09;
const RTC_STATUS_A: u8 = 0x0A;
const RTC_STATUS_B: u8 = 0x0B;

static BOOT_TIME: SpinLock<Option<RtcTime>> = SpinLock::new(None);

#[derive(Debug, Clone, Copy)]
pub struct RtcTime {
    pub second: u8,
    pub minute: u8,
    pub hour: u8,
    pub day: u8,
    pub month: u8,
    pub year: u16,
}

fn io_wait() {
    unsafe {
        // Read from port 0x80 (POST diagnostic port, no side effects)
        // to create a small I/O delay so CMOS has time to process register select.
        core::arch::asm!("in al, dx", out("al") _, in("dx") 0x80u16, options(nostack, preserves_flags));
    }
}

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        let mut addr = Port::<u8>::new(CMOS_ADDR);
        let mut data = Port::<u8>::new(CMOS_DATA);
        addr.write(reg);
        io_wait();
        data.read()
    }
}

fn is_updating() -> bool {
    cmos_read(RTC_STATUS_A) & 0x80 != 0
}

fn is_binary() -> bool {
    cmos_read(RTC_STATUS_B) & 0x04 != 0
}

fn bcd_to_binary(bcd: u8) -> u8 {
    (bcd & 0x0F) + ((bcd >> 4) * 10)
}

fn cmos_read_rtc(reg: u8, binary: bool) -> u8 {
    let val = cmos_read(reg);
    if binary { val } else { bcd_to_binary(val) }
}

fn read_all() -> RtcTime {
    while is_updating() {}
    let binary = is_binary();

    let second = cmos_read_rtc(RTC_SECONDS, binary);
    let minute = cmos_read_rtc(RTC_MINUTES, binary);
    let hour = cmos_read_rtc(RTC_HOURS, binary);
    let day = cmos_read_rtc(RTC_DAY, binary);
    let month = cmos_read_rtc(RTC_MONTH, binary);
    let year_raw = cmos_read_rtc(RTC_YEAR, binary);
    let year = 2000 + year_raw as u16;

    RtcTime { second, minute, hour, day, month, year }
}

pub fn init() {
    let boot = read_all();
    *BOOT_TIME.lock() = Some(boot);
    let t = boot;
    zenus_console::kinfo!("RTC: {:04}-{:02}-{:02} {:02}:{:02}:{:02}", t.year, t.month, t.day, t.hour, t.minute, t.second);
}

pub fn read_time() -> RtcTime {
    read_all()
}

pub fn boot_time() -> Option<RtcTime> {
    *BOOT_TIME.lock()
}

pub fn format_time(t: &RtcTime, buf: &mut [u8]) -> usize {
    let s: [u8; 19] = [
        (t.year / 1000 % 10) as u8 + b'0',
        (t.year / 100 % 10) as u8 + b'0',
        (t.year / 10 % 10) as u8 + b'0',
        (t.year % 10) as u8 + b'0',
        b'-',
        t.month / 10 + b'0',
        t.month % 10 + b'0',
        b'-',
        t.day / 10 + b'0',
        t.day % 10 + b'0',
        b' ',
        t.hour / 10 + b'0',
        t.hour % 10 + b'0',
        b':',
        t.minute / 10 + b'0',
        t.minute % 10 + b'0',
        b':',
        t.second / 10 + b'0',
        t.second % 10 + b'0',
    ];
    let len = s.len().min(buf.len());
    buf[..len].copy_from_slice(&s[..len]);
    len
}

/// Convert RtcTime to Unix epoch seconds.
pub fn rtc_to_epoch(t: &RtcTime) -> u64 {
    let year = t.year as i64;
    let month = t.month as i64;
    let day = t.day as i64;
    let hour = t.hour as i64;
    let minute = t.minute as i64;
    let second = t.second as i64;

    let y = year - if month <= 2 { 1 } else { 0 };
    let m = month + if month <= 2 { 12 } else { 0 };
    let era = y / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m - 3) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    let epoch = days * 86400 + hour * 3600 + minute * 60 + second;
    epoch as u64
}

/// Cache epoch at boot time.
pub fn cache_boot_epoch() {
    if let Some(t) = boot_time() {
        BOOT_EPOCH.store(rtc_to_epoch(&t), core::sync::atomic::Ordering::Relaxed);
    }
}
