use core::sync::atomic::{AtomicBool, Ordering};

#[repr(C)]
pub struct CpuRegisters {
    pub rax: u64, rbx: u64, rcx: u64, rdx: u64,
    pub rsi: u64, rdi: u64, rbp: u64, rsp: u64,
    pub r8: u64, r9: u64, r10: u64, r11: u64,
    pub r12: u64, r13: u64, r14: u64, r15: u64,
    pub rip: u64, rflags: u64, cs: u64, ss: u64,
}

pub struct CrashDump {
    pub magic: [u8; 16],
    pub timestamp: u64,
    pub registers: CpuRegisters,
    pub task_id: u64,
    pub cr3: u64,
    pub panic_message: [u8; 256],
    pub backtrace: [u64; 16],
    pub backtrace_count: usize,
}

impl CrashDump {
    pub const fn new() -> Self {
        CrashDump {
            magic: [0; 16],
            timestamp: 0,
            registers: CpuRegisters {
                rax: 0, rbx: 0, rcx: 0, rdx: 0,
                rsi: 0, rdi: 0, rbp: 0, rsp: 0,
                r8: 0, r9: 0, r10: 0, r11: 0,
                r12: 0, r13: 0, r14: 0, r15: 0,
                rip: 0, rflags: 0, cs: 0, ss: 0,
            },
            task_id: 0,
            cr3: 0,
            panic_message: [0; 256],
            backtrace: [0; 16],
            backtrace_count: 0,
        }
    }
}

const MAX_CRASH_CPUS: usize = 8;
static mut CRASH_DUMPS: [CrashDump; MAX_CRASH_CPUS] = [
    CrashDump::new(), CrashDump::new(), CrashDump::new(), CrashDump::new(),
    CrashDump::new(), CrashDump::new(), CrashDump::new(), CrashDump::new(),
];
static CRASH_SAVED: [AtomicBool; MAX_CRASH_CPUS] = [
    AtomicBool::new(false), AtomicBool::new(false),
    AtomicBool::new(false), AtomicBool::new(false),
    AtomicBool::new(false), AtomicBool::new(false),
    AtomicBool::new(false), AtomicBool::new(false),
];

fn crash_cpu() -> usize {
    let cpu: u64;
    unsafe { core::arch::asm!("mov {}, cr8", out(reg) cpu); }
    (cpu as usize) % MAX_CRASH_CPUS
}

pub fn crash_dump_init() {
    let cpu = crash_cpu();
    unsafe {
        CRASH_DUMPS[cpu].magic.copy_from_slice(b"ZENUS_CRASH_DUMP");
    }
}

pub fn crash_dump_save(msg: &str) -> &'static CrashDump {
    let cpu = crash_cpu();
    if CRASH_SAVED[cpu].load(Ordering::SeqCst) {
        return unsafe { &CRASH_DUMPS[cpu] };
    }
    CRASH_SAVED[cpu].store(true, Ordering::SeqCst);

    let dump = unsafe { &mut CRASH_DUMPS[cpu] };
    dump.magic.copy_from_slice(b"ZENUS_CRASH_DUMP");

    unsafe {
        core::arch::asm!("mov {}, rax", out(reg) dump.registers.rax);
        core::arch::asm!("mov {}, rbx", out(reg) dump.registers.rbx);
        core::arch::asm!("mov {}, rcx", out(reg) dump.registers.rcx);
        core::arch::asm!("mov {}, rdx", out(reg) dump.registers.rdx);
        core::arch::asm!("mov {}, rsi", out(reg) dump.registers.rsi);
        core::arch::asm!("mov {}, rdi", out(reg) dump.registers.rdi);
        core::arch::asm!("mov {}, rbp", out(reg) dump.registers.rbp);
        core::arch::asm!("mov {}, r8",  out(reg) dump.registers.r8);
        core::arch::asm!("mov {}, r9",  out(reg) dump.registers.r9);
        core::arch::asm!("mov {}, r10", out(reg) dump.registers.r10);
        core::arch::asm!("mov {}, r11", out(reg) dump.registers.r11);
        core::arch::asm!("mov {}, r12", out(reg) dump.registers.r12);
        core::arch::asm!("mov {}, r13", out(reg) dump.registers.r13);
        core::arch::asm!("mov {}, r14", out(reg) dump.registers.r14);
        core::arch::asm!("mov {}, r15", out(reg) dump.registers.r15);
        core::arch::asm!("mov {}, rsp", out(reg) dump.registers.rsp);
    }

    dump.registers.rip = unsafe {
        let mut rip: u64 = 0;
        core::arch::asm!("lea {}, [rip]", out(reg) rip);
        rip
    };

    dump.registers.rflags = unsafe {
        let mut rflags: u64 = 0;
        core::arch::asm!("pushfq; pop {}", out(reg) rflags);
        rflags
    };

    dump.registers.cs = 0x08;
    dump.registers.ss = 0x10;

    let msg_bytes = msg.as_bytes();
    let n = msg_bytes.len().min(255);
    dump.panic_message[..n].copy_from_slice(&msg_bytes[..n]);
    dump.panic_message[n] = 0;

    dump.cr3 = x86_64::registers::control::Cr3::read().0.start_address().as_u64();

    dump.backtrace_count = capture_backtrace(&mut dump.backtrace);

    dump
}

fn capture_backtrace(buf: &mut [u64; 16]) -> usize {
    let mut count = 0usize;
    unsafe {
        let mut fp: *mut u64;
        core::arch::asm!("mov {}, rbp", out(reg) fp);
        for _ in 0..16 {
            if fp.is_null() || (fp as usize) < 0xFFFFFFFF80000000 {
                break;
            }
            let ret_addr = *fp.add(1);
            buf[count] = ret_addr;
            count += 1;
            fp = core::ptr::read(fp as *mut *mut u64);
        }
    }
    count
}

pub fn crash_dump_print(dump: &CrashDump) {
    let serial = zenus_console::serial::SerialPort::new(0x3F8);
    serial.write_str("\n===== CRASH DUMP =====\n");
    serial.write_str("RAX: 0x"); serial.write_hex(dump.registers.rax);
    serial.write_str("  RBX: 0x"); serial.write_hex(dump.registers.rbx); serial.write_str("\n");
    serial.write_str("RCX: 0x"); serial.write_hex(dump.registers.rcx);
    serial.write_str("  RDX: 0x"); serial.write_hex(dump.registers.rdx); serial.write_str("\n");
    serial.write_str("RSI: 0x"); serial.write_hex(dump.registers.rsi);
    serial.write_str("  RDI: 0x"); serial.write_hex(dump.registers.rdi); serial.write_str("\n");
    serial.write_str("RBP: 0x"); serial.write_hex(dump.registers.rbp);
    serial.write_str("  RSP: 0x"); serial.write_hex(dump.registers.rsp); serial.write_str("\n");
    serial.write_str("R8:  0x"); serial.write_hex(dump.registers.r8);
    serial.write_str("  R9:  0x"); serial.write_hex(dump.registers.r9); serial.write_str("\n");
    serial.write_str("R10: 0x"); serial.write_hex(dump.registers.r10);
    serial.write_str("  R11: 0x"); serial.write_hex(dump.registers.r11); serial.write_str("\n");
    serial.write_str("R12: 0x"); serial.write_hex(dump.registers.r12);
    serial.write_str("  R13: 0x"); serial.write_hex(dump.registers.r13); serial.write_str("\n");
    serial.write_str("R14: 0x"); serial.write_hex(dump.registers.r14);
    serial.write_str("  R15: 0x"); serial.write_hex(dump.registers.r15); serial.write_str("\n");
    serial.write_str("RIP: 0x"); serial.write_hex(dump.registers.rip); serial.write_str("\n");
    serial.write_str("RFLAGS: 0x"); serial.write_hex(dump.registers.rflags); serial.write_str("\n");
    serial.write_str("CS: 0x"); serial.write_hex(dump.registers.cs as u64);
    serial.write_str("  SS: 0x"); serial.write_hex(dump.registers.ss as u64); serial.write_str("\n");
    serial.write_str("CR3: 0x"); serial.write_hex(dump.cr3); serial.write_str("\n");
    serial.write_str("Task ID: "); serial.write_u64(dump.task_id); serial.write_str("\n");
    serial.write_str("Message: ");
    let end = dump.panic_message.iter().position(|&b| b == 0).unwrap_or(255);
    if let Ok(msg) = core::str::from_utf8(&dump.panic_message[..end]) {
        serial.write_str(msg);
    }
    serial.write_str("\n");
    serial.write_str("Backtrace:\n");
    for i in 0..dump.backtrace_count {
        serial.write_str("  [");
        serial.write_u64(i as u64);
        serial.write_str("] 0x");
        serial.write_hex(dump.backtrace[i]);
        serial.write_str("\n");
    }
    serial.write_str("===== END CRASH DUMP =====\n");
}

pub fn crash_dump_get() -> Option<&'static CrashDump> {
    let cpu = crash_cpu();
    if CRASH_SAVED[cpu].load(Ordering::SeqCst) {
        Some(unsafe { &CRASH_DUMPS[cpu] })
    } else {
        None
    }
}

pub fn crash_dump_get_cpu(cpu: usize) -> Option<&'static CrashDump> {
    if cpu < MAX_CRASH_CPUS && CRASH_SAVED[cpu].load(Ordering::SeqCst) {
        Some(unsafe { &CRASH_DUMPS[cpu] })
    } else {
        None
    }
}

pub fn crash_dump_save_to_disk(dev_id: usize, lba: u64) -> bool {
    let cpu = crash_cpu();
    let dump = unsafe { &CRASH_DUMPS[cpu] };
    let dump_bytes = unsafe {
        core::slice::from_raw_parts(
            dump as *const CrashDump as *const u8,
            core::mem::size_of::<CrashDump>(),
        )
    };
    let mut sectors = [0u8; 512];
    let n = dump_bytes.len().min(512);
    sectors[..n].copy_from_slice(&dump_bytes[..n]);
    crate::ata::write_sectors(dev_id, lba, 1, &sectors)
}
