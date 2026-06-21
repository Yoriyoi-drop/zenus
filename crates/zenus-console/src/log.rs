use core::fmt::Write;
use crate::serial::SerialPort;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Critical,
}

impl LogLevel {
    pub fn prefix(self) -> &'static str {
        match self {
            LogLevel::Trace => "TRACE",
            LogLevel::Debug => "DEBUG",
            LogLevel::Info => "INFO ",
            LogLevel::Warn => "WARN ",
            LogLevel::Error => "ERROR",
            LogLevel::Critical => "CRIT ",
        }
    }
}

pub static mut LOG_LEVEL: LogLevel = LogLevel::Info;

pub fn set_level(level: LogLevel) {
    unsafe { LOG_LEVEL = level };
}

const DMESG_SIZE: usize = 32;

#[derive(Clone, Copy)]
pub struct DmesgEntry {
    pub level: LogLevel,
    pub msg: [u8; 128],
    pub len: u8,
}

pub struct Dmesg {
    buf: [DmesgEntry; DMESG_SIZE],
    idx: usize,
    count: usize,
}

impl Dmesg {
    const fn new() -> Self {
        const EMPTY: DmesgEntry = DmesgEntry { level: LogLevel::Info, msg: [0u8; 128], len: 0 };
        Dmesg { buf: [EMPTY; DMESG_SIZE], idx: 0, count: 0 }
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
        let start = if self.count < DMESG_SIZE { 0 } else { self.idx % DMESG_SIZE };
        DmesgIter { buf: &self.buf, pos: 0, count: self.count, start }
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
        let s = core::str::from_utf8(&entry.msg[..len]).unwrap_or("");
        self.pos += 1;
        Some((entry.level, s))
    }
}

use core::sync::atomic::{AtomicBool, Ordering};

static DMESG_INIT: AtomicBool = AtomicBool::new(false);
static mut DMESG_BUF: Dmesg = Dmesg::new();

pub fn dmesg_init() {
    DMESG_INIT.store(true, Ordering::Release);
}

pub fn dmesg_push(level: LogLevel, msg: &str) {
    if !DMESG_INIT.load(Ordering::Acquire) { return; }
    unsafe { DMESG_BUF.push(level, msg); }
}

pub struct DmesgSnapshot {
    pub entries: [DmesgEntry; DMESG_SIZE],
    pub count: usize,
}

pub fn dmesg_snapshot() -> DmesgSnapshot {
    let mut snap = DmesgSnapshot {
        entries: [DmesgEntry { level: LogLevel::Info, msg: [0u8; 128], len: 0 }; DMESG_SIZE],
        count: 0,
    };
    if !DMESG_INIT.load(Ordering::Acquire) { return snap; }
    unsafe {
        snap.count = DMESG_BUF.count;
        for (i, (level, msg)) in DMESG_BUF.iter().enumerate() {
            if i >= DMESG_SIZE { break; }
            let entry = &mut snap.entries[i];
            entry.level = level;
            let bytes = msg.as_bytes();
            let n = bytes.len().min(127);
            entry.msg[..n].copy_from_slice(&bytes[..n]);
            entry.msg[n] = 0;
            entry.len = n as u8;
        }
    }
    snap
}

pub fn dmesg_count() -> usize {
    if !DMESG_INIT.load(Ordering::Acquire) { return 0; }
    unsafe { DMESG_BUF.count }
}

pub fn log(level: LogLevel, module: &str, msg: &str) {
    let mut serial = SerialPort::new(0x3F8);
    let _ = write!(serial, "[{}][{}] {}\n", level.prefix(), module, msg);
    dmesg_push(level, msg);
}

#[macro_export]
macro_rules! ktrace {
    ($($arg:tt)*) => { $crate::log::log($crate::log::LogLevel::Trace, module_path!(), &format_args!($($arg)*).to_string()); };
}

#[macro_export]
macro_rules! kdebug {
    ($($arg:tt)*) => { $crate::log::log($crate::log::LogLevel::Debug, module_path!(), &format_args!($($arg)*).to_string()); };
}

#[macro_export]
macro_rules! kinfo {
    ($($arg:tt)*) => { $crate::log::log($crate::log::LogLevel::Info, module_path!(), &format_args!($($arg)*).to_string()); };
}

#[macro_export]
macro_rules! kwarn {
    ($($arg:tt)*) => { $crate::log::log($crate::log::LogLevel::Warn, module_path!(), &format_args!($($arg)*).to_string()); };
}

#[macro_export]
macro_rules! kerror {
    ($($arg:tt)*) => { $crate::log::log($crate::log::LogLevel::Error, module_path!(), &format_args!($($arg)*).to_string()); };
}

#[macro_export]
macro_rules! kcrit {
    ($($arg:tt)*) => { $crate::log::log($crate::log::LogLevel::Critical, module_path!(), &format_args!($($arg)*).to_string()); };
}
