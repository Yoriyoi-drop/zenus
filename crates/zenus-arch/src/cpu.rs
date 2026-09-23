use crate::gdt;
use core::sync::atomic::{AtomicU32, Ordering};
use x86_64::registers::model_specific::Msr;

extern "C" {
    pub fn syscall_dispatch(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64;
}

pub const MAX_CPUS: usize = 8;

// Reference scheduler's CURRENT_TASK (defined in zenus-sched, #[no_mangle])
extern "C" {
    static CURRENT_TASK: [AtomicU32; 8];
}

#[repr(C, align(64))]
#[derive(Copy, Clone)]
pub struct PerCpu {
    pub user_rsp: u64,
    pub kernel_rsp: u64,
}

#[no_mangle]
static mut PER_CPU: [PerCpu; MAX_CPUS] = [PerCpu {
    user_rsp: 0,
    kernel_rsp: 0,
}; MAX_CPUS];

pub fn percpu_virt_addr(cpu_id: u32) -> u64 {
    let idx = (cpu_id as usize).min(MAX_CPUS - 1);
    unsafe { &PER_CPU[idx] as *const _ as u64 }
}

pub fn init_percpu(cpu_id: u32) {
    let addr = percpu_virt_addr(cpu_id);
    unsafe {
        write_msr(0xC0000102, addr);
    }
}

pub fn set_percpu_kernel_rsp(cpu_id: u32, rsp: u64) {
    let idx = (cpu_id as usize).min(MAX_CPUS - 1);
    unsafe {
        PER_CPU[idx].kernel_rsp = rsp;
    }
}

pub fn get_percpu_user_rsp(cpu_id: u32) -> u64 {
    let idx = (cpu_id as usize).min(MAX_CPUS - 1);
    unsafe { PER_CPU[idx].user_rsp }
}

pub fn set_percpu_user_rsp(cpu_id: u32, rsp: u64) {
    let idx = (cpu_id as usize).min(MAX_CPUS - 1);
    unsafe {
        PER_CPU[idx].user_rsp = rsp;
    }
}

pub unsafe fn percpu_ptr() -> u64 {
    &PER_CPU[0] as *const _ as u64
}

// ---------------------------------------------------------------------------
// SYSCALL entry/exit assembly
// ---------------------------------------------------------------------------
// RCX = user RIP, R11 = user RFLAGS on SYSCALL.  Saved at KSP-8/KSP-16,
// but those may be overwritten by heap allocator MAGIC_FREE writes.
// FIX: Push deep copies at KSP-24/KSP-32 and restore from those.
//
// Push order: rcx-orig, r11-orig, r11-deep, rcx-deep
// After call syscall_dispatch + ret:
//   RSP = KSP - 32
//   [RSP+0]  = rcx-deep (RIP at KSP-32) ← PUSHED LAST
//   [RSP+8]  = r11-deep (RFLAGS at KSP-24)
//   [RSP+16] = r11-orig possibly corrupted (KSP-16)
//   [RSP+24] = rcx-orig possibly corrupted (KSP-8)
//
// Restore: pop rcx(RIP-deep), pop r11(RFLAGS-deep), add rsp,16(skip corr)
// ---------------------------------------------------------------------------
core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl syscall_entry",
    "syscall_entry:",
    "  cli",
    "  swapgs",
    "  mov gs:[0], rsp",
    "  mov rsp, gs:[8]",
    // Save originals (may be corrupted by heap operations during handler)
    "  push rcx", // KSP-8: user RIP (original, may be corrupted)
    "  push r11", // KSP-16: user RFLAGS (original, may be corrupted)
    // Deep copies (preserved below corrupted area)
    "  push r11", // KSP-24: user RFLAGS (deep copy, safe)
    "  push rcx", // KSP-32: user RIP (deep copy, safe) ← PUSHED LAST
    "  mov rcx, rdx",
    "  mov r8, rsi",
    "  mov r9, rdi",
    "  mov rdi, rax",
    "  mov rsi, r9",
    "  mov rdx, r8",
    "  call syscall_dispatch",
    // After call+ret: RSP = KSP - 32 (ret_addr consumed by ret inside dispatch)
    // Stack: [rcx-deep(RIP)][r11-deep(RFLAGS)][r11-orig(corr)][rcx-orig(corr)]
    // Pop deep copies, then skip the corrupted originals:
    "  pop rcx",     // RCX = saved RIP from DEEP COPY (safe), RSP = KSP - 24
    "  pop r11",     // R11 = saved RFLAGS from DEEP COPY (safe), RSP = KSP - 16
    "  add rsp, 16", // Skip corrupted originals at KSP-8/KSP-16, RSP = KSP
    // Validate saved RCX before SYSRET.
    // Kernel-space RCX or NULL RCX indicates corruption; route to debug loop.
    "  mov rax, rcx",
    "  shr rax, 47",
    "  cmp rax, 1",
    "  jae 9f",
    "  test ecx, ecx",
    "  jz 9f",
    "10:",
    // Return to user mode via IRETQ (not SYSRET).
    // SYSRET in 64-bit mode sets RSP = RDX, but RDX was clobbered by
    // syscall_dispatch (caller-saved register). IRETQ loads RSP from
    // the stack, which is more predictable and avoids the RDX dependency.
    //
    // Build 5-item IRETQ frame on the KERNEL stack:
    //   [RSP+0]  = RIP  (user return address from RCX)
    //   [RSP+8]  = CS   (USER_CODE | RPL3 = 0x23)
    //   [RSP+16] = RFLAGS (from R11, with IF cleared to avoid timer
    //           interrupts in the brief window after return)
    //   [RSP+24] = RSP  (user stack from gs:[0])
    //   [RSP+32] = SS   (USER_DATA | RPL3 = 0x1b)
    //
    "  mov rax, gs:[0]", // RAX = user RSP (from PerCpu)
    "  push 0x1b",       // SS  = user data segment with RPL=3
    "  push rax",        // RSP = user stack pointer
    "  push r11",        // RFLAGS (IF=0)
    "  push 0x23",       // CS  = user code segment with RPL=3
    "  push rcx",        // RIP = user return address
    // Zero GS_BASE via SWAPGS so user mode can't access kernel PerCpu via GS segment.
    // After SWAPGS: GS_BASE=0 (user cannot access PerCpu) and
    // KERNEL_GS_BASE=PerCpu (next SYSCALL SWAPGS will restore GS_BASE=PerCpu).
    // Critical: do NOT use wrmsr to zero GS_BASE directly, because that
    // would leave KERNEL_GS_BASE=0, causing gs:[0] to be NULL on next SYSCALL.
    "  swapgs",
    "  iretq",
    // Label 9: corrupted RCX (kernel-space or NULL). Normal path falls
    // through iretq above and never reaches here; the validation jumps
    // (`jae 9f`/`jz 9f`) land here. Print bad RCX to COM1, then halt.
    "9:",
    "  mov rdi, rcx",
    "  call dbg_print_hex",
    "98:",
    "  cli",
    "  hlt",
    "  jmp 98b",
    ".att_syntax prefix",
);

extern "C" {
    pub fn syscall_entry();
    pub fn syscall_signal_hook(kernel_rsp: u64);
}

// Debug helper — no extern declarations needed, defined below in this module.
// global_asm! finds them by symbol name (both are #[no_mangle] pub extern "C").

/// Write one byte to UART COM1 (0x3F8).
///
/// Allocation-free on purpose: this runs from the syscall fault/halt path
/// where the heap spinlock may already be held, so `alloc::format!` could
/// deadlock.
#[inline]
unsafe fn dbg_putb(b: u8) {
    core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nostack, preserves_flags));
}

/// Write a 64-bit value as hex to UART COM1 (0x3F8). Never allocates.
#[no_mangle]
pub extern "C" fn dbg_print_hex(val: u64) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    unsafe {
        dbg_putb(b'0');
        dbg_putb(b'x');
        for i in 0..16u32 {
            let nibble = ((val >> ((15 - i) * 4)) & 0xf) as usize;
            dbg_putb(HEX[nibble]);
        }
        dbg_putb(b'\n');
    }
}

/// Print task index, CPU ID, and kernel stack pointer to UART.
/// Never allocates (see `dbg_putb`).
#[no_mangle]
pub extern "C" fn dbg_print_taskinfo() {
    unsafe {
        dbg_putb(b'T');
        let cpu = crate::interrupts::apic::current_apic_id();
        let idx = CURRENT_TASK[(cpu as usize) % MAX_CPUS].load(Ordering::Relaxed);
        dbg_print_dec(idx as u64);
        dbg_putb(b'C');
        dbg_putb(b'P');
        dbg_putb(b'U');
        dbg_print_dec(cpu as u64);
        let rsp: u64;
        core::arch::asm!("mov {}, gs:[8]", out(reg) rsp, options(nostack));
        dbg_putb(b' ');
        dbg_print_hex(rsp);
    }
}

/// Write a decimal value to UART COM1. Never allocates.
#[no_mangle]
pub extern "C" fn dbg_print_dec(mut val: u64) {
    let mut buf = [0u8; 20];
    let mut n = 0usize;
    if val == 0 {
        unsafe {
            dbg_putb(b'0');
        }
        return;
    }
    while val > 0 {
        buf[n] = b'0' + (val % 10) as u8;
        val /= 10;
        n += 1;
    }
    unsafe {
        while n > 0 {
            n -= 1;
            dbg_putb(buf[n]);
        }
    }
}

pub fn init() {
    enable_sse();
    enable_nxe();
    enable_syscall();
    gdt::init();
}

fn enable_nxe() {
    let mut efer = Msr::new(0xC000_0080);
    unsafe {
        efer.write(efer.read() | (1 << 11));
    }
}

pub(crate) fn enable_sse() {
    unsafe {
        let mut cr0: u64;
        core::arch::asm!("mov {}, cr0", out(reg) cr0, options(nostack, preserves_flags));
        cr0 &= !(1 << 2);
        cr0 |= 1 << 1;
        core::arch::asm!("mov cr0, {}", in(reg) cr0, options(nostack, preserves_flags));

        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
        cr4 |= 1 << 9; // OSFXSR
        cr4 |= 1 << 10; // OSXMMEXCPT
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
    }
}

pub fn enable_smep_smap() {
    unsafe {
        let mut cr4: u64;
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
        cr4 |= 1 << 20; // SMEP
        cr4 |= 1 << 21; // SMAP
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
    }
}

#[inline]
pub unsafe fn stac() {
    let cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
    if cr4 & (1 << 21) != 0 {
        core::arch::asm!("stac", options(nostack, nomem));
    }
}

#[inline]
pub unsafe fn clac() {
    let cr4: u64;
    core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
    if cr4 & (1 << 21) != 0 {
        core::arch::asm!("clac", options(nostack, nomem));
    }
}

pub fn enable_syscall_ap() {
    let mut lstar = Msr::new(0xC000_0082);
    let mut sfmask = Msr::new(0xC000_0084);
    unsafe {
        lstar.write(syscall_entry as *const () as u64);
        sfmask.write(!0x202);
    }
}

pub fn init_syscall_ap(cpu_id: u32) {
    init_percpu(cpu_id);
    enable_syscall_ap();
}

fn enable_syscall() {
    let mut efer = Msr::new(0xC000_0080);
    let mut star = Msr::new(0xC000_0081);
    let mut lstar = Msr::new(0xC000_0082);
    let mut sfmask = Msr::new(0xC000_0084);

    let code_seg: u64 = gdt::KERNEL_CODE.index() as u64 * 8; // 0x08
    let user_base: u64 = gdt::KERNEL_DATA.index() as u64 * 8; // 0x10

    unsafe {
        efer.write(efer.read() | 1); // EFER.SCE
        star.write((code_seg << 32) | (user_base << 48));
        lstar.write(syscall_entry as *const () as u64);
        sfmask.write(!0x202);
    }

    init_percpu(0);
}

pub unsafe fn write_msr(msr: u32, value: u64) {
    let mut m = Msr::new(msr);
    m.write(value);
}

pub unsafe fn read_msr(msr: u32) -> u64 {
    Msr::new(msr).read()
}

pub fn get_cpu_vendor() -> [u8; 12] {
    let mut eax: u32;
    let mut ebx: u32;
    let mut ecx: u32;
    let mut edx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 0",
            "cpuid",
            "mov {:e}, ebx",
            "pop rbx",
            out(reg) ebx,
            out("eax") eax,
            out("ecx") ecx,
            out("edx") edx,
            options(nostack)
        );
    }
    let _ = eax;
    let mut buf = [0u8; 12];
    buf[0] = (ebx & 0xFF) as u8;
    buf[1] = ((ebx >> 8) & 0xFF) as u8;
    buf[2] = ((ebx >> 16) & 0xFF) as u8;
    buf[3] = ((ebx >> 24) & 0xFF) as u8;
    buf[4] = (edx & 0xFF) as u8;
    buf[5] = ((edx >> 8) & 0xFF) as u8;
    buf[6] = ((edx >> 16) & 0xFF) as u8;
    buf[7] = ((edx >> 24) & 0xFF) as u8;
    buf[8] = (ecx & 0xFF) as u8;
    buf[9] = ((ecx >> 8) & 0xFF) as u8;
    buf[10] = ((ecx >> 16) & 0xFF) as u8;
    buf[11] = ((ecx >> 24) & 0xFF) as u8;
    buf
}

pub fn has_feature(feature: &str) -> bool {
    let mut ecx: u32;
    let mut edx: u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            out("ecx") ecx,
            out("edx") edx,
            out("eax") _,
            options(nostack)
        );
    }
    match feature {
        "apic" => (edx >> 9) & 1 == 1,
        "x2apic" => (ecx >> 21) & 1 == 1,
        "msr" => (edx >> 5) & 1 == 1,
        "sse" => (edx >> 25) & 1 == 1,
        "sse2" => (edx >> 26) & 1 == 1,
        "pae" => (edx >> 6) & 1 == 1,
        "pge" => (edx >> 13) & 1 == 1,
        "pat" => (edx >> 16) & 1 == 1,
        "nx" => (edx >> 20) & 1 == 1,
        "syscall" => (edx >> 11) & 1 == 1,
        "lm" => (edx >> 29) & 1 == 1,
        "rdrand" => (ecx >> 30) & 1 == 1,
        _ => false,
    }
}
