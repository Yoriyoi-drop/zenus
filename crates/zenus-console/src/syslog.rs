use core::sync::atomic::{AtomicBool, Ordering};
use crate::log::{LogLevel, dmesg_push};
use zenus_sync::spinlock::SpinLock;

pub const SYSLOG_MAX_ENTRIES: usize = 1024;
pub const SYSLOG_PATH: &str = "/var/log/syslog";

#[derive(Debug, Clone, Copy)]
pub struct SyslogEntry {
    pub timestamp: u64,
    pub level: LogLevel,
    pub module: [u8; 32],
    pub msg: [u8; 256],
    pub msg_len: u16,
}

impl SyslogEntry {
    const fn new() -> Self {
        SyslogEntry {
            timestamp: 0,
            level: LogLevel::Info,
            module: [0u8; 32],
            msg: [0u8; 256],
            msg_len: 0,
        }
    }
}

pub struct Syslog {
    entries: [SyslogEntry; SYSLOG_MAX_ENTRIES],
    idx: usize,
    count: usize,
    #[allow(dead_code)]
    output_to_file: bool,
    log_file_path: &'static str,
}

impl Syslog {
    const fn new() -> Self {
        const EMPTY: SyslogEntry = SyslogEntry::new();
        Syslog {
            entries: [EMPTY; SYSLOG_MAX_ENTRIES],
            idx: 0,
            count: 0,
            output_to_file: false,
            log_file_path: SYSLOG_PATH,
        }
    }

    fn write(&mut self, level: LogLevel, module: &str, msg: &str) {
        let slot = self.idx % SYSLOG_MAX_ENTRIES;
        let entry = &mut self.entries[slot];
        entry.timestamp = rdtsc();
        entry.level = level;

        let module_bytes = module.as_bytes();
        let module_len = module_bytes.len().min(31);
        entry.module[..module_len].copy_from_slice(&module_bytes[..module_len]);
        if module_len < 32 {
            entry.module[module_len] = 0;
        }

        let msg_bytes = msg.as_bytes();
        let msg_len = msg_bytes.len().min(255);
        entry.msg[..msg_len].copy_from_slice(&msg_bytes[..msg_len]);
        if msg_len < 256 {
            entry.msg[msg_len] = 0;
        }
        entry.msg_len = msg_len as u16;

        self.idx = self.idx.wrapping_add(1);
        if self.count < SYSLOG_MAX_ENTRIES {
            self.count += 1;
        }
    }

    fn entry_start(&self) -> usize {
        if self.count < SYSLOG_MAX_ENTRIES {
            0
        } else {
            self.idx % SYSLOG_MAX_ENTRIES
        }
    }
}

fn rdtsc() -> u64 {
    unsafe {
        let lo: u32;
        let hi: u32;
        core::arch::asm!("rdtsc", out("eax") lo, out("edx") hi, options(nostack, preserves_flags));
        ((hi as u64) << 32) | lo as u64
    }
}

static SYSLOG_INIT: AtomicBool = AtomicBool::new(false);
static SYSLOG: SpinLock<Syslog> = SpinLock::new(Syslog::new());

pub fn syslog_init() {
    SYSLOG_INIT.store(true, Ordering::Release);
    crate::kinfo!("Syslog initialized");
}

pub fn syslog_write(level: LogLevel, module: &str, msg: &str) {
    if !SYSLOG_INIT.load(Ordering::Acquire) {
        return;
    }
    SYSLOG.lock().write(level, module, msg);
    dmesg_push(level, msg);
}

pub fn syslog_flush_to_disk(_dev_id: usize) -> bool {
    false
}

pub fn syslog_set_output_file(path: &'static str) {
    if !SYSLOG_INIT.load(Ordering::Acquire) {
        return;
    }
    SYSLOG.lock().log_file_path = path;
}

pub fn syslog_get_count() -> usize {
    if !SYSLOG_INIT.load(Ordering::Acquire) {
        return 0;
    }
    SYSLOG.lock().count
}

pub fn syslog_get(idx: usize) -> Option<SyslogEntry> {
    if !SYSLOG_INIT.load(Ordering::Acquire) {
        return None;
    }
    let syslog = SYSLOG.lock();
    if idx >= syslog.count {
        return None;
    }
    let start = syslog.entry_start();
    let slot = (start + idx) % SYSLOG_MAX_ENTRIES;
    Some(syslog.entries[slot])
}

pub fn syslog_module_str(entry: &SyslogEntry) -> &str {
    let len = entry.module.iter().position(|&b| b == 0).unwrap_or(32);
    core::str::from_utf8(&entry.module[..len]).unwrap_or("")
}

pub fn syslog_msg_str(entry: &SyslogEntry) -> &str {
    let len = entry.msg_len as usize;
    core::str::from_utf8(&entry.msg[..len]).unwrap_or("")
}

pub fn syslog_rotate() -> bool {
    false
}
