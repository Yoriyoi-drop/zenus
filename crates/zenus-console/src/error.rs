use crate::log::LogLevel;
use crate::serial::SerialPort;
use core::fmt::Write;

// ── ANSI Color Support ──
pub mod color {
    pub const RESET:     &str = "\x1b[0m";
    pub const BOLD:      &str = "\x1b[1m";
    pub const DIM:       &str = "\x1b[2m";
    pub const BLACK:     &str = "\x1b[30m";
    pub const RED:       &str = "\x1b[31m";
    pub const GREEN:     &str = "\x1b[32m";
    pub const YELLOW:    &str = "\x1b[33m";
    pub const BLUE:      &str = "\x1b[34m";
    pub const MAGENTA:   &str = "\x1b[35m";
    pub const CYAN:      &str = "\x1b[36m";
    pub const GRAY:      &str = "\x1b[37m";
    pub const WHITE:     &str = "\x1b[97m";
    pub const BG_RED:    &str = "\x1b[41m";
    pub const BG_BLUE:   &str = "\x1b[44m";
    pub const BG_GREEN:  &str = "\x1b[42m";
    pub const BG_YELLOW: &str = "\x1b[43m";
}

pub fn level_color(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace    => color::GRAY,
        LogLevel::Debug    => color::CYAN,
        LogLevel::Info     => color::GREEN,
        LogLevel::Notice   => color::BLUE,
        LogLevel::Warn     => color::YELLOW,
        LogLevel::Error    => color::RED,
        LogLevel::Critical => color::MAGENTA,
        LogLevel::Fatal    => "\x1b[1;31m",
        LogLevel::Panic    => "\x1b[1;37;41m",
    }
}

pub fn level_name(level: LogLevel) -> &'static str {
    match level {
        LogLevel::Trace    => "TRACE",
        LogLevel::Debug    => "DEBUG",
        LogLevel::Info     => "INFO",
        LogLevel::Notice   => "NOTICE",
        LogLevel::Warn     => "WARNING",
        LogLevel::Error    => "ERROR",
        LogLevel::Critical => "CRITICAL",
        LogLevel::Fatal    => "FATAL",
        LogLevel::Panic    => "PANIC",
    }
}

// ── Error Module ──
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum ErrorModule {
    Kernel     = 0,
    Memory     = 1,
    FileSystem = 2,
    Process    = 3,
    Driver     = 4,
    Network    = 5,
    Security   = 6,
}

impl ErrorModule {
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Kernel     => "KRN",
            Self::Memory     => "MEM",
            Self::FileSystem => "FS",
            Self::Process    => "PRC",
            Self::Driver     => "DRV",
            Self::Network    => "NET",
            Self::Security   => "SEC",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Kernel     => "Kernel",
            Self::Memory     => "MemoryManager",
            Self::FileSystem => "FileSystem",
            Self::Process    => "ProcessManager",
            Self::Driver     => "Driver",
            Self::Network    => "Network",
            Self::Security   => "Security",
        }
    }
}

// ── Error Definition ──
#[derive(Debug, Clone, Copy)]
pub struct ErrorDef {
    pub code: &'static str,
    pub module: ErrorModule,
    pub severity: LogLevel,
    pub desc: &'static str,
    pub reason: &'static str,
    pub actions: &'static [&'static str],
    pub suggestion: &'static str,
}

// ── Error Code Catalog ──
pub mod codes {
    use super::*;

    // Kernel
    pub const KRN_PANIC_INVALID_MEM: ErrorDef = ErrorDef {
        code: "ZN-KRN-0001",
        module: ErrorModule::Kernel,
        severity: LogLevel::Panic,
        desc: "Kernel panic: Invalid memory mapping",
        reason: "Virtual page table entry points to an unmapped physical address.",
        actions: &["Halt current CPU", "Save crash dump", "Notify debugger", "Enter recovery mode"],
        suggestion: "Check page table initialization in paging.rs",
    };
    pub const KRN_NULL_PTR: ErrorDef = ErrorDef {
        code: "ZN-KRN-0002",
        module: ErrorModule::Kernel,
        severity: LogLevel::Panic,
        desc: "Null pointer access detected",
        reason: "A kernel function dereferenced a null or near-null virtual address.",
        actions: &["Halt current CPU", "Save crash dump", "Dump register state"],
        suggestion: "Check pointer validation before dereference. Review recent changes to pointer arithmetic.",
    };
    pub const KRN_STACK_OVERFLOW: ErrorDef = ErrorDef {
        code: "ZN-KRN-0003",
        module: ErrorModule::Kernel,
        severity: LogLevel::Panic,
        desc: "Stack overflow in kernel thread",
        reason: "Kernel stack pointer moved into guard page region, indicating stack exhaustion.",
        actions: &["Halt current CPU", "Save crash dump", "Dump backtrace"],
        suggestion: "Increase kernel stack size. Check for deep recursion or large stack allocations.",
    };
    pub const KRN_UNHANDLED_EXCEPTION: ErrorDef = ErrorDef {
        code: "ZN-KRN-0004",
        module: ErrorModule::Kernel,
        severity: LogLevel::Panic,
        desc: "CPU exception was not handled",
        reason: "An unexpected CPU exception occurred with no registered handler.",
        actions: &["Halt current CPU", "Save crash dump", "Dump exception frame"],
        suggestion: "Check IDT entry registration. Verify interrupt handler chain.",
    };

    // Memory
    pub const MEM_ALLOC_FAILED: ErrorDef = ErrorDef {
        code: "ZN-MEM-0001",
        module: ErrorModule::Memory,
        severity: LogLevel::Critical,
        desc: "Physical page allocation failed",
        reason: "The frame allocator found no free physical pages.",
        actions: &["Retry allocation", "Trigger OOM handler", "Try reclaiming pages"],
        suggestion: "Check memory map parsing. Increase available RAM in VM config. Verify no memory leaks.",
    };
    pub const MEM_OUT_OF_RANGE: ErrorDef = ErrorDef {
        code: "ZN-MEM-0002",
        module: ErrorModule::Memory,
        severity: LogLevel::Error,
        desc: "Virtual address out of range",
        reason: "The requested virtual address does not fall within any valid memory region.",
        actions: &["Reject operation", "Log address range"],
        suggestion: "Verify virtual address calculation. Check page table boundaries.",
    };
    pub const MEM_PROTECTION: ErrorDef = ErrorDef {
        code: "ZN-MEM-0003",
        module: ErrorModule::Memory,
        severity: LogLevel::Error,
        desc: "Memory protection violation",
        reason: "A page fault occurred due to insufficient permissions (write to read-only, execute non-executable).",
        actions: &["Send SIGSEGV", "Dump faulting address", "Log access type"],
        suggestion: "Check page flags in map_page. Verify W^X enforcement.",
    };

    // Filesystem
    pub const FS_METADATA_CORRUPT: ErrorDef = ErrorDef {
        code: "ZN-FS-0001",
        module: ErrorModule::FileSystem,
        severity: LogLevel::Critical,
        desc: "Filesystem metadata corrupted",
        reason: "Superblock, inode table, or journal contains invalid or inconsistent data.",
        actions: &["Unmount filesystem", "Trigger fsck", "Switch to read-only"],
        suggestion: "Run fsck on the disk image. Check journal replay logic.",
    };
    pub const FS_INVALID_HANDLE: ErrorDef = ErrorDef {
        code: "ZN-FS-0002",
        module: ErrorModule::FileSystem,
        severity: LogLevel::Error,
        desc: "File handle is invalid",
        reason: "An operation referenced a file descriptor that was not open or had been closed.",
        actions: &["Return EBADF", "Log PID and FD number"],
        suggestion: "Check fd table bounds. Verify close-on-exec handling.",
    };
    pub const FS_MOUNT_FAILED: ErrorDef = ErrorDef {
        code: "ZN-FS-0003",
        module: ErrorModule::FileSystem,
        severity: LogLevel::Error,
        desc: "Partition mount failed",
        reason: "The partition could not be mounted due to unsupported format or device error.",
        actions: &["Return error to caller", "Log block device status"],
        suggestion: "Check partition table. Verify driver supports the filesystem type.",
    };

    // Process Manager
    pub const PRC_CREATE_FAILED: ErrorDef = ErrorDef {
        code: "ZN-PRC-0001",
        module: ErrorModule::Process,
        severity: LogLevel::Error,
        desc: "Process creation failed",
        reason: "fork/exec could not allocate task struct, stack, or address space.",
        actions: &["Return -ENOMEM", "Log resource exhaustion"],
        suggestion: "Check available task slots. Verify memory for new address space.",
    };
    pub const PRC_SCHEDULER_CORRUPT: ErrorDef = ErrorDef {
        code: "ZN-PRC-0002",
        module: ErrorModule::Process,
        severity: LogLevel::Critical,
        desc: "Scheduler queue corrupted",
        reason: "Task linked list or priority queue contains invalid pointers or circular references.",
        actions: &["Halt scheduler", "Dump task table", "Emergency task kill"],
        suggestion: "Check task state transitions. Verify queue insertion/removal logic.",
    };
    pub const PRC_SYNC_TIMEOUT: ErrorDef = ErrorDef {
        code: "ZN-PRC-0003",
        module: ErrorModule::Process,
        severity: LogLevel::Warn,
        desc: "Thread synchronization timeout",
        reason: "A wait operation (mutex, semaphore, condvar) timed out without acquiring the resource.",
        actions: &["Return -ETIMEDOUT", "Log waiter and owner TIDs"],
        suggestion: "Check for deadlock. Verify timeout calculation.",
    };

    // Driver
    pub const DRV_INIT_FAILED: ErrorDef = ErrorDef {
        code: "ZN-DRV-0001",
        module: ErrorModule::Driver,
        severity: LogLevel::Critical,
        desc: "Device initialization failed",
        reason: "The device did not respond to initialization sequence or returned invalid configuration.",
        actions: &["Mark device as unavailable", "Dump device registers", "Notify device manager"],
        suggestion: "Check device power state. Verify PCI enumeration. Check driver-probe sequence.",
    };
    pub const DRV_SIGNATURE_FAILED: ErrorDef = ErrorDef {
        code: "ZN-DRV-0002",
        module: ErrorModule::Driver,
        severity: LogLevel::Error,
        desc: "Driver signature verification failed",
        reason: "The loaded driver module's cryptographic signature did not match the expected value.",
        actions: &["Reject driver load", "Log signature details"],
        suggestion: "Re-sign the driver module. Check for tampering. Verify signing key.",
    };
    pub const DRV_COMM_TIMEOUT: ErrorDef = ErrorDef {
        code: "ZN-DRV-0003",
        module: ErrorModule::Driver,
        severity: LogLevel::Error,
        desc: "Hardware communication timeout",
        reason: "A hardware operation did not complete within the expected time window.",
        actions: &["Reset device", "Retry operation", "Log device state"],
        suggestion: "Check interrupt delivery. Verify MMIO region mapping. Try increasing timeout.",
    };

    // Network
    pub const NET_IF_UNAVAILABLE: ErrorDef = ErrorDef {
        code: "ZN-NET-0001",
        module: ErrorModule::Network,
        severity: LogLevel::Error,
        desc: "Network interface unavailable",
        reason: "The requested network interface is not present, not initialized, or link is down.",
        actions: &["Return ENETDOWN", "Log interface status"],
        suggestion: "Check NIC initialization. Verify link status. Check cable connection.",
    };
    pub const NET_DHCP_FAILED: ErrorDef = ErrorDef {
        code: "ZN-NET-0002",
        module: ErrorModule::Network,
        severity: LogLevel::Warn,
        desc: "DHCP lease acquisition failed",
        reason: "No DHCP server responded to discovery requests within the timeout period.",
        actions: &["Fall back to link-local", "Retry on next interval"],
        suggestion: "Check network connectivity. Verify DHCP server is running. Check for packet filters.",
    };
    pub const NET_CHECKSUM_FAILED: ErrorDef = ErrorDef {
        code: "ZN-NET-0003",
        module: ErrorModule::Network,
        severity: LogLevel::Warn,
        desc: "Packet checksum validation failed",
        reason: "The computed checksum for the packet did not match the header value.",
        actions: &["Drop packet", "Increment error counter"],
        suggestion: "Check NIC offloading settings. Verify TCP/IP checksum logic.",
    };

    // Security
    pub const SEC_UNAUTHORIZED: ErrorDef = ErrorDef {
        code: "ZN-SEC-0001",
        module: ErrorModule::Security,
        severity: LogLevel::Critical,
        desc: "Unauthorized kernel access blocked",
        reason: "A user-space process attempted to access kernel memory or execute privileged instructions.",
        actions: &["Kill offending process", "Log access details", "Raise security alert"],
        suggestion: "Check SMEP/SMAP configuration. Review syscall argument validation.",
    };
    pub const SEC_POLICY_VIOLATION: ErrorDef = ErrorDef {
        code: "ZN-SEC-0002",
        module: ErrorModule::Security,
        severity: LogLevel::Error,
        desc: "Security policy violation detected",
        reason: "An operation was denied by the system security policy (capabilities, seccomp, LSM).",
        actions: &["Deny operation", "Log process credentials", "Audit event"],
        suggestion: "Check process capabilities. Verify security policy configuration.",
    };
}

// ── Error Ring Buffer ──
const ERROR_BUF_SIZE: usize = 64;

struct ErrorBuf {
    entries: [ErrorEntry; ERROR_BUF_SIZE],
    idx: usize,
    count: usize,
}

#[derive(Clone, Copy)]
pub struct ErrorEntry {
    pub timestamp: u64,
    pub cpu: u32,
    pub level: LogLevel,
    pub code: [u8; 16],
    pub module: [u8; 24],
    pub msg: [u8; 128],
    pub msg_len: u8,
    pub file: [u8; 48],
    pub line: u32,
    pub count: u64,
}

impl ErrorEntry {
    const fn new() -> Self {
        ErrorEntry {
            timestamp: 0, cpu: 0, level: LogLevel::Info,
            code: [0; 16], module: [0; 24], msg: [0; 128], msg_len: 0,
            file: [0; 48], line: 0, count: 0,
        }
    }
}

impl ErrorBuf {
    const fn new() -> Self {
        const EMPTY: ErrorEntry = ErrorEntry::new();
        ErrorBuf { entries: [EMPTY; ERROR_BUF_SIZE], idx: 0, count: 0 }
    }

    fn push(&mut self, cpu: u32, level: LogLevel, code: Option<&str>, module: &str, msg: &str, file: &str, line: u32) {
        let slot = &mut self.entries[self.idx % ERROR_BUF_SIZE];
        slot.timestamp = rdtsc();
        slot.cpu = cpu;
        slot.level = level;
        slot.line = line;
        slot.count = slot.count.wrapping_add(1);

        if let Some(c) = code {
            let b = c.as_bytes();
            let n = b.len().min(15);
            slot.code[..n].copy_from_slice(&b[..n]);
            slot.code[n] = 0;
        } else {
            slot.code[0] = 0;
        }

        let mb = module.as_bytes();
        let mn = mb.len().min(23);
        slot.module[..mn].copy_from_slice(&mb[..mn]);
        slot.module[mn] = 0;

        let mlen = msg.as_bytes().len().min(127);
        slot.msg[..mlen].copy_from_slice(&msg.as_bytes()[..mlen]);
        slot.msg[mlen] = 0;
        slot.msg_len = mlen as u8;

        let fb = file.as_bytes();
        let fn_ = fb.len().min(47);
        slot.file[..fn_].copy_from_slice(&fb[..fn_]);
        slot.file[fn_] = 0;

        self.idx = self.idx.wrapping_add(1);
        if self.count < ERROR_BUF_SIZE {
            self.count += 1;
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

// ── Error Counter Helpers ──
static mut ERROR_COUNTS: [u64; 32] = [0; 32];

fn error_code_index(code: &str) -> usize {
    // Map "ZN-XXX-NNNN" to a numeric index
    // Extract the last 4 digits
    if code.len() >= 11 {
        if let Ok(n) = code[code.len()-4..].parse::<usize>() {
            return n % 32;
        }
    }
    0
}

fn bump_error_count(code: &str) {
    let idx = error_code_index(code);
    unsafe { ERROR_COUNTS[idx] = ERROR_COUNTS[idx].wrapping_add(1); }
}

pub fn get_error_count(code: &str) -> u64 {
    let idx = error_code_index(code);
    unsafe { ERROR_COUNTS[idx] }
}

// ── Global Error Buffer ──
use core::sync::atomic::{AtomicBool, Ordering};
use zenus_sync::spinlock::SpinLock;

static ERR_BUF_INIT: AtomicBool = AtomicBool::new(false);
static ERR_BUF: SpinLock<ErrorBuf> = SpinLock::new(ErrorBuf::new());

pub fn error_buf_init() {
    ERR_BUF_INIT.store(true, Ordering::Release);
}

pub fn record_error(level: LogLevel, code: Option<&'static str>, module: &str, msg: &str, file: &str, line: u32) {
    if let Some(c) = code {
        bump_error_count(c);
    }
    if ERR_BUF_INIT.load(Ordering::Acquire) {
        ERR_BUF.lock().push(0, level, code, module, msg, file, line);
    }
}

// ── Timestamp Callback ──
// Allow kernel to register a wall-clock timestamp formatter.
// Falls back to RDTSC-based relative time if no callback registered.

static TIMESTAMP_WRITER: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub type TimestampWriter = fn(s: &mut SerialPort);

pub fn register_timestamp_writer(f: TimestampWriter) {
    TIMESTAMP_WRITER.store(f as u64, Ordering::Release);
}

fn write_timestamp(s: &mut SerialPort) {
    let addr = TIMESTAMP_WRITER.load(Ordering::Acquire);
    if addr != 0 {
        let f: TimestampWriter = unsafe { core::mem::transmute(addr) };
        f(s);
        return;
    }
    // Fallback: RDTSC-based relative time
    let tsc = rdtsc();
    s.write_str("[");
    let frac = (tsc as u32) / 1000;
    let us = frac % 1000;
    let ms = (frac / 1000) % 1000;
    let sec = frac / 1_000_000;
    if sec < 10 { s.write_str("0"); }
    s.write_u64(sec as u64);
    s.write_str(".");
    if ms < 100 { s.write_str("0"); }
    if ms < 10 { s.write_str("0"); }
    s.write_u64(ms as u64);
    if us < 100 { s.write_str("0"); }
    if us < 10 { s.write_str("0"); }
    s.write_u64(us as u64);
    s.write_str("]");
}

/// Compact single-line log output with color support
pub fn write_compact(s: &mut SerialPort, level: LogLevel, module: &str, code: Option<&str>, msg: &str) {
    write_timestamp(s);
    let _ = write!(s, " {}{}{}", level_color(level), level_name(level), color::RESET);
    s.write_str(" [");
    s.write_str(module);
    s.write_str("]");
    if let Some(c) = code {
        s.write_str(" ");
        let _ = write!(s, "{}{}{}", color::BOLD, c, color::RESET);
    }
    s.write_str(" ");
    s.write_str(msg);
    s.write_str("\n");
}

/// Full detail error output (like error code reference card)
pub fn write_detailed(s: &mut SerialPort, def: &ErrorDef, extra: &str, file: &str, line: u32) {
    let _ = write!(s, "{}", color::BOLD);
    s.write_str("  ╔══════════════════════════════════════════════════════════╗\n");
    let _ = write!(s, "{}", color::RESET);
    write_code_banner(s, def);
    s.write_str("  ╠══════════════════════════════════════════════════════════╣\n");
    write_field(s, "Time", "");
    write_timestamp(s);
    s.write_str("\n");
    write_field(s, "Level", "");
    let _ = write!(s, "{}{}{}", level_color(def.severity), level_name(def.severity), color::RESET);
    s.write_str("\n");
    write_field(s, "Module", def.module.name());
    s.write_str("\n");
    write_field(s, "Code", def.code);
    s.write_str("\n");
    if !extra.is_empty() {
        write_field(s, "Detail", extra);
        s.write_str("\n");
    }
    write_field(s, "File", file);
    s.write_str("\n");
    write_field(s, "Line", "");
    s.write_u64(line as u64);
    s.write_str("\n");
    s.write_str("  ╠══════════════════════════════════════════════════════════╣\n");
    write_field(s, "Description", def.desc);
    s.write_str("\n");
    write_field(s, "Reason", def.reason);
    s.write_str("\n");
    s.write_str("  ╠══════════════════════════════════════════════════════════╣\n");
    s.write_str("   Actions:\n");
    for a in def.actions {
        s.write_str("     • ");
        s.write_str(a);
        s.write_str("\n");
    }
    s.write_str("  ╠══════════════════════════════════════════════════════════╣\n");
    write_field(s, "Suggestion", def.suggestion);
    s.write_str("\n");
    let _ = write!(s, "{}", color::BOLD);
    s.write_str("  ╚══════════════════════════════════════════════════════════╝\n");
    let _ = write!(s, "{}", color::RESET);
}

fn write_code_banner(s: &mut SerialPort, def: &ErrorDef) {
    let _ = write!(s, "  ║ {}{}{} {}",
        level_color(def.severity),
        level_name(def.severity),
        color::RESET,
        color::BOLD,
    );
    s.write_str(def.code);
    let _ = write!(s, "{} ║\n", color::RESET);
}

fn write_field(s: &mut SerialPort, label: &str, value: &str) {
    s.write_str(color::DIM);
    s.write_str("   ");
    s.write_str(label);
    s.write_str(": ");
    s.write_str(color::RESET);
    if !value.is_empty() {
        s.write_str(value);
    }
}

/// Write error code reference: list all known error codes
pub fn dump_error_catalog(s: &mut SerialPort) {
    let all: &[&ErrorDef] = &[
        &codes::KRN_PANIC_INVALID_MEM,
        &codes::KRN_NULL_PTR,
        &codes::KRN_STACK_OVERFLOW,
        &codes::KRN_UNHANDLED_EXCEPTION,
        &codes::MEM_ALLOC_FAILED,
        &codes::MEM_OUT_OF_RANGE,
        &codes::MEM_PROTECTION,
        &codes::FS_METADATA_CORRUPT,
        &codes::FS_INVALID_HANDLE,
        &codes::FS_MOUNT_FAILED,
        &codes::PRC_CREATE_FAILED,
        &codes::PRC_SCHEDULER_CORRUPT,
        &codes::PRC_SYNC_TIMEOUT,
        &codes::DRV_INIT_FAILED,
        &codes::DRV_SIGNATURE_FAILED,
        &codes::DRV_COMM_TIMEOUT,
        &codes::NET_IF_UNAVAILABLE,
        &codes::NET_DHCP_FAILED,
        &codes::NET_CHECKSUM_FAILED,
        &codes::SEC_UNAUTHORIZED,
        &codes::SEC_POLICY_VIOLATION,
    ];
    s.write_str("\n");
    s.write_str("  ╔══════════════════════════════════════════════╗\n");
    s.write_str("  ║        Zenus$ Error Code Reference           ║\n");
    s.write_str("  ╚══════════════════════════════════════════════╝\n\n");
    let mut cur_module: Option<ErrorModule> = None;
    for def in all {
        if cur_module.map(|m| m != def.module).unwrap_or(true) {
            s.write_str(" ");
            s.write_str(color::BOLD);
            s.write_str(def.module.name());
            let _ = write!(s, "{}", color::RESET);
            s.write_str("\n");
            s.write_str("  ");
            for _ in 0..def.module.name().len() { s.write_str("─"); }
            s.write_str("\n");
            cur_module = Some(def.module);
        }
        let _ = write!(s, "  {}{:<16}{}  {}",
            color::BOLD, def.code, color::RESET, def.desc);
        s.write_str("\n");
        let count = get_error_count(def.code);
        if count > 0 {
            s.write_str("       ");
            let _ = write!(s, "{}freq: {}{}", color::DIM, count, color::RESET);
            s.write_str("\n");
        }
    }
    s.write_str("\n");
}
