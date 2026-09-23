use crate::serial::SerialPort;
use core::fmt::Write;
use core::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use zenus_sync::spinlock::SpinLock;

/// Buffer log sementara untuk formatting pesan kernel.
/// Ukuran diperbesar ke 512 byte agar pesan panjang tidak terpotong diam-diam.
pub struct LogBuf {
    buf: [u8; 512],
    pos: usize,
    /// True jika pesan dipotong karena buffer penuh
    truncated: bool,
}

impl Default for LogBuf {
    fn default() -> Self {
        Self::new()
    }
}

impl LogBuf {
    pub fn new() -> Self {
        LogBuf {
            buf: [0u8; 512],
            pos: 0,
            truncated: false,
        }
    }

    pub fn as_str(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.pos]).unwrap_or("")
    }

    /// True jika pesan dipotong karena melebihi kapasitas buffer
    #[allow(dead_code)]
    pub fn was_truncated(&self) -> bool {
        self.truncated
    }
}

impl core::fmt::Write for LogBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
        let bytes = s.as_bytes();
        let remaining = self.buf.len().saturating_sub(self.pos);
        let n = bytes.len().min(remaining);
        self.buf[self.pos..self.pos + n].copy_from_slice(&bytes[..n]);
        self.pos += n;
        if bytes.len() > remaining {
            self.truncated = true;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum LogLevel {
    Trace = 0,
    Debug = 1,
    Notice = 2,
    Info = 3,
    Warn = 4,
    Error = 5,
    Critical = 6,
    Fatal = 7,
    Panic = 8,
}

impl LogLevel {
    pub fn prefix(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Notice => "NOTICE",
            LogLevel::Info => "INFO ",
            LogLevel::Warn => "WARN ",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRIT ",
            LogLevel::Fatal => "FATAL",
            LogLevel::Panic => "PANIC",
        }
    }

    fn from_u8(v: u8) -> Self {
        match v {
            0 => LogLevel::Trace,
            1 => LogLevel::Debug,
            2 => LogLevel::Notice,
            3 => LogLevel::Info,
            4 => LogLevel::Warn,
            5 => LogLevel::Error,
            6 => LogLevel::Critical,
            7 => LogLevel::Fatal,
            _ => LogLevel::Panic,
        }
    }
}

/// Level log global — disimpan sebagai AtomicU8 agar aman diakses dari
/// banyak core tanpa lock.
static LOG_LEVEL_ATOMIC: AtomicU8 = AtomicU8::new(LogLevel::Info as u8);

pub fn set_level(level: LogLevel) {
    LOG_LEVEL_ATOMIC.store(level as u8, Ordering::Relaxed);
}

pub fn get_level() -> LogLevel {
    LogLevel::from_u8(LOG_LEVEL_ATOMIC.load(Ordering::Relaxed))
}

const DMESG_SIZE: usize = 256;

#[derive(Clone, Copy)]
pub struct DmesgEntry {
    pub level: LogLevel,
    pub msg: [u8; 128],
    pub len: u8,
}

pub struct Dmesg {
    buf: [DmesgEntry; DMESG_SIZE],
    idx: usize,
    pub count: usize,
}

impl Dmesg {
    const fn new() -> Self {
        const EMPTY: DmesgEntry = DmesgEntry {
            level: LogLevel::Info,
            msg: [0u8; 128],
            len: 0,
        };
        Dmesg {
            buf: [EMPTY; DMESG_SIZE],
            idx: 0,
            count: 0,
        }
    }

    pub fn push(&mut self, level: LogLevel, msg: &str) {
        let entry = &mut self.buf[self.idx % DMESG_SIZE];
        entry.level = level;
        let bytes = msg.as_bytes();
        let n = bytes.len().min(127);
        entry.msg[..n].copy_from_slice(&bytes[..n]);
        entry.msg[n] = 0;
        entry.len = n as u8;
        self.idx += 1;
        if self.count < DMESG_SIZE {
            self.count += 1;
        }
    }

    pub fn iter(&self) -> DmesgIter<'_> {
        let start = if self.count < DMESG_SIZE {
            0
        } else {
            self.idx % DMESG_SIZE
        };
        DmesgIter {
            buf: &self.buf,
            pos: 0,
            count: self.count,
            start,
        }
    }
}

pub struct DmesgIter<'a> {
    buf: &'a [DmesgEntry; DMESG_SIZE],
    pos: usize,
    count: usize,
    start: usize,
}

impl<'a> Iterator for DmesgIter<'a> {
    type Item = (LogLevel, &'a str);
    fn next(&mut self) -> Option<Self::Item> {
        if self.pos >= self.count {
            return None;
        }
        let i = (self.start + self.pos) % DMESG_SIZE;
        let entry = &self.buf[i];
        let len = entry.len as usize;
        let s = core::str::from_utf8(&entry.msg[..len]).unwrap_or(" ");
        self.pos += 1;
        Some((entry.level, s))
    }
}

static DMESG_INIT: AtomicBool = AtomicBool::new(false);
static DMESG_BUF: SpinLock<Dmesg> = SpinLock::new(Dmesg::new());

pub fn dmesg_init() {
    DMESG_INIT.store(true, Ordering::Release);
}

pub fn dmesg_push(level: LogLevel, msg: &str) {
    if !DMESG_INIT.load(Ordering::Acquire) {
        return;
    }
    DMESG_BUF.lock().push(level, msg);
}

pub struct DmesgSnapshot {
    pub entries: [DmesgEntry; DMESG_SIZE],
    pub count: usize,
}

pub fn dmesg_snapshot() -> DmesgSnapshot {
    let mut snap = DmesgSnapshot {
        entries: [DmesgEntry {
            level: LogLevel::Info,
            msg: [0u8; 128],
            len: 0,
        }; DMESG_SIZE],
        count: 0,
    };
    if !DMESG_INIT.load(Ordering::Acquire) {
        return snap;
    }
    let buf = DMESG_BUF.lock();
    snap.count = buf.count;
    for (i, (level, msg)) in buf.iter().enumerate() {
        if i >= DMESG_SIZE {
            break;
        }
        let entry = &mut snap.entries[i];
        entry.level = level;
        let bytes = msg.as_bytes();
        let n = bytes.len().min(127);
        entry.msg[..n].copy_from_slice(&bytes[..n]);
        entry.msg[n] = 0;
        entry.len = n as u8;
    }
    snap
}

pub fn dmesg_count() -> usize {
    if !DMESG_INIT.load(Ordering::Acquire) {
        return 0;
    }
    DMESG_BUF.lock().count
}

pub fn log(level: LogLevel, module: &str, msg: &str) {
    let mut serial = SerialPort::new(0x3F8);
    let _ = writeln!(serial, "[{}][{}] {}", level.prefix(), module, msg);
    dmesg_push(level, msg);
    // During early boot (interrupts disabled), flush immediately so output
    // is visible even if a crash/hang occurs before the next scheduled flush.
    #[cfg(target_os = "none")]
    if !x86_64::instructions::interrupts::are_enabled() {
        crate::serial::flush_output_blocking();
    }
}

#[macro_export]
macro_rules! ktrace {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        $crate::log::log($crate::log::LogLevel::Trace, module_path!(), _buf.as_str());
    }};
}

#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        $crate::log::log($crate::log::LogLevel::Debug, module_path!(), _buf.as_str());
    }};
}

#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        $crate::log::log($crate::log::LogLevel::Info, module_path!(), _buf.as_str());
    }};
}

#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        $crate::log::log($crate::log::LogLevel::Warn, module_path!(), _buf.as_str());
    }};
}

#[macro_export]
macro_rules! kerror {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        $crate::log::log($crate::log::LogLevel::Error, module_path!(), _buf.as_str());
    }};
}

#[macro_export]
macro_rules! kcrit {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        $crate::log::log($crate::log::LogLevel::Critical, module_path!(), _buf.as_str());
    }};
}

/// Log an error with a structured error code (compact format)
#[macro_export]
macro_rules! kerror_code {
    ($code:expr, $($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        let _msg = _buf.as_str();
        let _level = $code.severity;
        $crate::log::log(_level, module_path!(), _msg);
        $crate::error::record_error(_level, Some($code.code), module_path!(), _msg, file!(), line!());
    }};
}

/// Log an error with full detailed output (error code card)
#[macro_export]
macro_rules! kerror_detail {
    ($code:expr, $($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        let _msg = _buf.as_str();
        let _level = $code.severity;
        let mut _s = $crate::serial::SerialPort::new(0x3F8);
        $crate::error::write_detailed(&mut _s, &$code, _msg, file!(), line!());
        $crate::log::dmesg_push(_level, _msg);
        $crate::error::record_error(_level, Some($code.code), module_path!(), _msg, file!(), line!());
    }};
}

/// Fatal error (system cannot continue) — logs, records, does NOT halt
#[macro_export]
macro_rules! kfatal {
    ($($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        let _msg = _buf.as_str();
        $crate::log::log($crate::log::LogLevel::Fatal, module_path!(), _msg);
        $crate::error::record_error($crate::log::LogLevel::Fatal, None, module_path!(), _msg, file!(), line!());
    }};
}

/// Fatal error with structured error code — logs, dumps detailed card, returns
#[macro_export]
macro_rules! kfatal_code {
    ($code:expr, $($arg:tt)*) => {{
        let mut _buf = $crate::log::LogBuf::new();
        let _ = core::fmt::write(&mut _buf, format_args!($($arg)*));
        let _msg = _buf.as_str();
        let mut _s = $crate::serial::SerialPort::new(0x3F8);
        $crate::error::write_detailed(&mut _s, &$code, _msg, file!(), line!());
        $crate::log::dmesg_push($code.severity, _msg);
        $crate::error::record_error($code.severity, Some($code.code), module_path!(), _msg, file!(), line!());
    }};
}

/// Panic with structured error code — logs, dumps detailed card, then halts
/// FLUSHES output buffer before halting so error is visible on serial.
#[macro_export]
macro_rules! kpanic_code {
    ($code:expr, $($arg:tt)*) => {{
        $crate::kfatal_code!($code, $($arg)*);
        $crate::serial::flush_output_blocking();
        loop { x86_64::instructions::hlt(); }
    }};
}
