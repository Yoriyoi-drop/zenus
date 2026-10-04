use alloc::vec::Vec;
use zenus_sync::spinlock::SpinLock;

const MAX_CRASHES: usize = 256;

static CRASH_LOG: SpinLock<CrashLog> = SpinLock::new(CrashLog::new());

struct CrashLog {
    crashes: [Option<CrashRecord>; MAX_CRASHES],
    count: usize,
}

impl CrashLog {
    const fn new() -> Self {
        CrashLog {
            crashes: [const { None }; MAX_CRASHES],
            count: 0,
        }
    }
}

#[derive(Clone)]
pub struct CrashRecord {
    pub id: u64,
    pub subsystem: u8,
    pub crash_type: CrashType,
    pub input: Vec<u8>,
    pub rip: u64,
    pub stack_trace: [u64; 16],
    pub registers: [u64; 16],
    pub timestamp: u64,
}

/// Subsystem ids used in the `subsystem` field of a record.
pub const SUBSYS_SYSCALL: u8 = 0;
pub const SUBSYS_MEMORY: u8 = 1;
pub const SUBSYS_DEVICE: u8 = 2;
pub const SUBSYS_FILESYSTEM: u8 = 3;
pub const SUBSYS_SHELL: u8 = 4;
pub const SUBSYS_SCHEDULER: u8 = 5;

#[derive(Clone, Copy, PartialEq)]
pub enum CrashType {
    PageFault,
    GeneralProtectionFault,
    StackOverflow,
    DoubleFree,
    UseAfterFree,
    BufferOverflow,
    NullPointerDeref,
    InvalidOpcode,
    DivideByZero,
    Unknown,
}

impl CrashType {
    pub fn name(self) -> &'static str {
        match self {
            CrashType::PageFault => "PAGE_FAULT",
            CrashType::GeneralProtectionFault => "GENERAL_PROTECTION",
            CrashType::StackOverflow => "STACK_OVERFLOW",
            CrashType::DoubleFree => "DOUBLE_FREE",
            CrashType::UseAfterFree => "USE_AFTER_FREE",
            CrashType::BufferOverflow => "BUFFER_OVERFLOW",
            CrashType::NullPointerDeref => "NULL_DEREF",
            CrashType::InvalidOpcode => "INVALID_OPCODE",
            CrashType::DivideByZero => "DIVIDE_BY_ZERO",
            CrashType::Unknown => "UNKNOWN",
        }
    }
}

/// Map a CPU exception vector onto a crash classification.
pub fn crash_type_from_vector(vector: u64) -> CrashType {
    match vector {
        0 => CrashType::DivideByZero,
        6 => CrashType::InvalidOpcode,
        13 => CrashType::GeneralProtectionFault,
        14 => {
            // A #PF whose fault address is exactly 0 is a null dereference;
            // everything else is reported as a plain page fault.
            if zenus_arch::fuzz_guard::LAST_ADDR.load(core::sync::atomic::Ordering::Relaxed) == 0 {
                CrashType::NullPointerDeref
            } else {
                CrashType::PageFault
            }
        }
        _ => CrashType::Unknown,
    }
}

/// Insert a record into the log. Returns the assigned crash id when the record
/// fitted, `None` when the log was full.
///
/// The log is a fixed-size ring of 256 entries: once it is full a campaign that
/// keeps finding the same crash must not keep bumping `count` past the array
/// end (the old code stopped recording but still logged "CRASH DETECTED" for
/// every occurrence, flooding the console with duplicates).
fn store(record: CrashRecord) -> Option<u64> {
    let mut log = CRASH_LOG.lock();
    if log.count >= MAX_CRASHES {
        drop(log);
        return None;
    }
    let id = record.id;
    let idx = log.count;
    log.crashes[idx] = Some(record);
    log.count = idx + 1;
    drop(log);
    Some(id)
}

fn new_record(input: &[u8], subsystem: u8, crash_type: CrashType) -> CrashRecord {
    let count = {
        let log = CRASH_LOG.lock();
        log.count as u64
    };
    CrashRecord {
        // Crash IDs follow the `ZENUS-FUZZ-000127` format from the fuzzing doc.
        id: count,
        subsystem,
        crash_type,
        input: input.to_vec(),
        rip: 0,
        stack_trace: [0; 16],
        registers: [0; 16],
        timestamp: zenus_arch::interrupts::pit::get_ticks(),
    }
}

pub fn record_crash(input: &[u8]) {
    if let Some(id) = store(new_record(input, SUBSYS_SYSCALL, CrashType::Unknown)) {
        zenus_console::kerror!("[FUZZ] CRASH ZENUS-FUZZ-{:06} subsystem=syscall", id);
    }
}

pub fn record_hang(input: &[u8]) {
    if let Some(id) = store(new_record(input, SUBSYS_SYSCALL, CrashType::Unknown)) {
        zenus_console::kerror!("[FUZZ] HANG ZENUS-FUZZ-{:06}", id);
    }
}

/// Record a fault that the guard contained during a test case.
///
/// `subsystem`/`args` are the description of the case that produced it, so a
/// crash can be reproduced from the log alone.
pub fn record_fault(subsystem: u8, args: &[u8]) {
    use core::sync::atomic::Ordering;

    let vector = zenus_arch::fuzz_guard::LAST_VECTOR.load(Ordering::Relaxed);
    let rip = zenus_arch::fuzz_guard::LAST_RIP.load(Ordering::Relaxed);
    let addr = zenus_arch::fuzz_guard::LAST_ADDR.load(Ordering::Relaxed);
    let err = zenus_arch::fuzz_guard::LAST_ERR.load(Ordering::Relaxed);
    let crash_type = crash_type_from_vector(vector);

    // The case descriptor is the corpus input verbatim, so a contained fault
    // is reproducible from the log alone.
    let mut record = new_record(args, subsystem, crash_type);
    record.rip = rip;
    // The checkpoint stack is gone by now, so the trace is reported as the
    // registers we do have: the recovery frame contents.
    record.registers = [vector, rip, addr, err, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0];

    let vector_name = zenus_arch::fuzz_guard::vector_name(vector);
    if let Some(id) = store(record) {
        zenus_console::kerror!(
            "[FUZZ] CRASH ZENUS-FUZZ-{:06} subsystem={} type={} vector={} ({}) rip=0x{:x} addr=0x{:x} err=0x{:x} args={:x?}",
            id,
            subsystem,
            crash_type.name(),
            vector,
            vector_name,
            rip,
            addr,
            err,
            args
        );
    }
}

pub fn classify_crash(syscall_id: u64, args: &[u64; 6], result: u64) {
    let crash_type = if result == 0xDEADBEEF {
        CrashType::PageFault
    } else if result == 0xCAFEBABE {
        CrashType::GeneralProtectionFault
    } else {
        CrashType::Unknown
    };

    zenus_console::kerror!(
        "[FUZZ] classified crash: syscall={} args={:x?} type={}",
        syscall_id,
        args,
        crash_type.name()
    );
}

pub fn get_crash_count() -> u64 {
    let log = CRASH_LOG.lock();
    log.count as u64
}

pub fn get_crash(idx: u64) -> Option<CrashRecord> {
    let log = CRASH_LOG.lock();
    log.crashes.get(idx as usize).and_then(|c| c.clone())
}

/// Print every unique crash in the `ZENUS-FUZZ-NNNNNN` format used by the
/// fuzzing documentation, plus the serialized reproducer input.
pub fn dump_crashes() {
    let count = get_crash_count();
    zenus_console::kinfo!("[FUZZ] REPORT crashes={}", count);
    for i in 0..count {
        if let Some(record) = get_crash(i) {
            zenus_console::kinfo!(
                "[FUZZ]   ZENUS-FUZZ-{:06} subsystem={} type={} rip=0x{:x} input={:02x?}",
                record.id,
                record.subsystem,
                record.crash_type.name(),
                record.rip,
                record.input
            );
        }
    }
}

pub fn clear() {
    let mut log = CRASH_LOG.lock();
    for i in 0..MAX_CRASHES {
        log.crashes[i] = None;
    }
    log.count = 0;
    drop(log);
}
