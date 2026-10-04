use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use zenus_sync::spinlock::SpinLock;

pub const MAX_SYSCTLS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SysctlType {
    Int,
    Uint,
    Bool,
    String,
}

#[derive(Debug, Clone)]
pub enum SysctlValue {
    IntVal(i64),
    UintVal(u64),
    BoolVal(bool),
    StrVal(&'static str),
}

impl SysctlValue {
    pub fn ty(&self) -> SysctlType {
        match self {
            SysctlValue::IntVal(_) => SysctlType::Int,
            SysctlValue::UintVal(_) => SysctlType::Uint,
            SysctlValue::BoolVal(_) => SysctlType::Bool,
            SysctlValue::StrVal(_) => SysctlType::String,
        }
    }

    pub fn as_int(&self) -> Option<i64> {
        match self {
            SysctlValue::IntVal(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_uint(&self) -> Option<u64> {
        match self {
            SysctlValue::UintVal(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SysctlValue::BoolVal(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&'static str> {
        match self {
            SysctlValue::StrVal(v) => Some(v),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct SysctlEntry {
    pub name: &'static str,
    pub description: &'static str,
    pub value: SysctlValue,
    pub read_only: bool,
}

struct SysctlTable {
    entries: [SysctlEntry; MAX_SYSCTLS],
    count: usize,
}

impl SysctlTable {
    const fn new() -> Self {
        const EMPTY: SysctlEntry = SysctlEntry {
            name: "",
            description: "",
            value: SysctlValue::IntVal(0),
            read_only: false,
        };
        SysctlTable {
            entries: [EMPTY; MAX_SYSCTLS],
            count: 0,
        }
    }

    fn register(&mut self, name: &'static str, desc: &'static str, initial: SysctlValue) -> usize {
        if self.count >= MAX_SYSCTLS {
            return usize::MAX;
        }
        let idx = self.count;
        self.entries[idx] = SysctlEntry {
            name,
            description: desc,
            value: initial,
            read_only: false,
        };
        self.count += 1;
        idx
    }

    fn find(&self, name: &str) -> Option<usize> {
        for i in 0..self.count {
            if self.entries[i].name == name {
                return Some(i);
            }
        }
        None
    }
}

static SYSCTL_INIT: AtomicBool = AtomicBool::new(false);
static SYSCTL_TABLE: SpinLock<SysctlTable> = SpinLock::new(SysctlTable::new());

static UPTIME_TICKS: AtomicU64 = AtomicU64::new(0);

/// Tick rate of the scheduler tick that feeds [`sysctl_tick`].
///
/// `pit::init()` programs `PIT_FREQ / 100`, so the timer interrupt — and with
/// it `SYS_TICKS` / `UPTIME_TICKS` — fires 100 times per second. Dividing by
/// 1000 here (as this module used to) under-reported uptime by 10x.
const TICK_HZ: u64 = 100;

/// Advance the uptime counter by one scheduler tick.
///
/// Called from the scheduler tick path — without a caller `kernel.uptime`
/// stayed pinned at 0 forever.
pub fn sysctl_tick() {
    UPTIME_TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Current uptime in whole seconds.
fn uptime_seconds() -> u64 {
    UPTIME_TICKS.load(Ordering::Relaxed) / TICK_HZ
}

pub fn sysctl_init() {
    let mut table = SYSCTL_TABLE.lock();
    // Reset before registering. `register` has no duplicate check and nothing
    // cleared the table, so every call appended a second copy of all eight
    // defaults (measured: 8 -> 16 -> 24 entries), and `sysctl_set` on the first
    // copy of a key no longer affected the value that `sysctl_get` returned
    // from the last one.
    table.count = 0;
    table.register(
        "kernel.hostname",
        "System hostname",
        SysctlValue::StrVal("zenus"),
    );
    table.register(
        "kernel.log_level",
        "Console log level (0=Trace..5=Critical)",
        SysctlValue::IntVal(2),
    );
    table.register(
        "kernel.version",
        "Kernel version string",
        SysctlValue::StrVal("Zenus OS v0.1.0"),
    );
    table.register(
        "kernel.uptime",
        "System uptime in seconds",
        SysctlValue::UintVal(0),
    );
    table.register(
        "kernel.max_tasks",
        "Maximum number of tasks",
        SysctlValue::IntVal(128),
    );
    table.register(
        "kernel.watchdog_timeout",
        "Watchdog timeout in seconds",
        SysctlValue::IntVal(30),
    );
    table.register(
        "net.ipv4.ip_forward",
        "IP forwarding enabled",
        SysctlValue::BoolVal(false),
    );
    table.register(
        "net.dns.server",
        "DNS server address",
        SysctlValue::StrVal("10.0.2.3"),
    );

    if let Some(idx) = table.find("kernel.uptime") {
        table.entries[idx].read_only = true;
    }

    SYSCTL_INIT.store(true, Ordering::Release);
    zenus_console::kinfo!("Sysctl initialized");
}

pub fn sysctl_register(name: &'static str, desc: &'static str, initial: SysctlValue) -> usize {
    if !SYSCTL_INIT.load(Ordering::Acquire) {
        return usize::MAX;
    }
    SYSCTL_TABLE.lock().register(name, desc, initial)
}

pub fn sysctl_get(idx: usize) -> Option<SysctlEntry> {
    if !SYSCTL_INIT.load(Ordering::Acquire) {
        return None;
    }
    let table = SYSCTL_TABLE.lock();
    if idx >= table.count {
        return None;
    }
    let mut entry = table.entries[idx].clone();

    if entry.name == "kernel.uptime" {
        entry.value = SysctlValue::UintVal(uptime_seconds());
    }

    Some(entry)
}

pub fn sysctl_set(idx: usize, value: SysctlValue) -> bool {
    if !SYSCTL_INIT.load(Ordering::Acquire) {
        return false;
    }
    let mut table = SYSCTL_TABLE.lock();
    if idx >= table.count {
        return false;
    }
    if table.entries[idx].read_only {
        return false;
    }
    if table.entries[idx].value.ty() != value.ty() {
        return false;
    }
    table.entries[idx].value = value;
    true
}

pub fn sysctl_find(name: &str) -> Option<usize> {
    if !SYSCTL_INIT.load(Ordering::Acquire) {
        return None;
    }
    SYSCTL_TABLE.lock().find(name)
}

pub fn sysctl_count() -> usize {
    if !SYSCTL_INIT.load(Ordering::Acquire) {
        return 0;
    }
    SYSCTL_TABLE.lock().count
}

pub fn sysctl_list() -> Vec<SysctlEntry> {
    let mut result = Vec::with_capacity(MAX_SYSCTLS);
    if !SYSCTL_INIT.load(Ordering::Acquire) {
        return result;
    }
    let table = SYSCTL_TABLE.lock();
    for i in 0..table.count {
        let mut entry = table.entries[i].clone();
        if entry.name == "kernel.uptime" {
            entry.value = SysctlValue::UintVal(uptime_seconds());
        }
        result.push(entry);
    }
    result
}
