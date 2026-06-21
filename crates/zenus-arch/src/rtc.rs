use x86_64::instructions::port::Port;
use zenus_sync::spinlock::SpinLock;

const CMOS_ADDR: u16 = 0x70;
const CMOS_DATA: u16 = 0x71;

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

fn cmos_read(reg: u8) -> u8 {
    unsafe {
        let mut addr = Port::<u8>::new(CMOS_ADDR);
        let mut data = Port::<u8>::new(CMOS_DATA);
        addr.write(reg);
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
    let mut s = zenus_console::serial::SerialPort::new(0x3F8);
    s.write_str("[OK] RTC: ");
    s.write_u64(t.year as u64);
    s.write_str("-");
    s.write_u64(t.month as u64);
    s.write_str("-");
    s.write_u64(t.day as u64);
    s.write_str(" ");
    s.write_u64(t.hour as u64);
    s.write_str(":");
    s.write_u64(t.minute as u64);
    s.write_str(":");
    s.write_u64(t.second as u64);
    s.write_str("\n");
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
