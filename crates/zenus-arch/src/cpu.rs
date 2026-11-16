use crate::gdt;
use core::sync::atomic::{AtomicU32, Ordering};
use x86_64::registers::model_specific::Msr;

extern "C" {
    pub fn syscall_dispatch(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64;
    /// Full 6-argument dispatcher used by `syscall_entry`. Args 4/5/6 come
    /// from r10/r8/r9 (Linux ABI) and must be forwarded intact.
    pub fn syscall_dispatch6(
        num: u64,
        arg1: u64,
        arg2: u64,
        arg3: u64,
        arg4: u64,
        arg5: u64,
        arg6: u64,
    ) -> u64;
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
// RCX = user RIP, R11 = user RFLAGS on SYSCALL.  Saved at KSP-40/KSP-32,
// but those may be overwritten by heap allocator MAGIC_FREE writes.
// FIX: Push deep copies at KSP-48/KSP-56 and restore from those.
//
// Syscall args 4/5/6 arrive in r10/r8/r9 (Linux ABI).  They MUST be
// spilled before any register shuffling, otherwise the shuffling below
// (mov rcx,rdx / mov rdx,rsi / mov rsi,rdi) destroys them and the handler
// silently receives 0 for every argument past the third.
//
// Push order: r10, r8, r9, rcx-orig, r11-orig, r11-deep, rcx-deep
// After call syscall_dispatch6 + ret:
//   RSP = KSP - 56
//   [RSP+0]  = rcx-deep (RIP at KSP-56)      ← PUSHED LAST
//   [RSP+8]  = r11-deep (RFLAGS at KSP-48)
//   [RSP+16] = r11-orig possibly corrupted (KSP-40)
//   [RSP+24] = rcx-orig possibly corrupted (KSP-32)
//   [RSP+32] = r9-orig  = arg6 (KSP-24)
//   [RSP+40] = r8-orig  = arg5 (KSP-16)
//   [RSP+48] = r10-orig = arg4 (KSP-8)
//
// Restore: pop rcx(RIP-deep), pop r11(RFLAGS-deep), restore r8/r9/r10,
//          add rsp,40(skip arg slots + corrupted originals)
// ---------------------------------------------------------------------------
core::arch::global_asm!(
    ".intel_syntax noprefix",
    // KPTI: syscall_entry must be reachable through the user view, so it lives
    // in .text.kpti which is mapped into both the kernel and user PML4s.
    ".section .text.kpti, \"ax\"",
    ".globl syscall_entry",
    "syscall_entry:",
    "  cli",
    // CR3 is still the user view here. Switch to the kernel view before
    // touching any kernel memory (including gs). The slot lives in .kpti_data
    // which is mapped in both views.
    //
    // IMPORTANT: `syscall` left the user return RIP in RCX and RFLAGS in R11.
    // Load the kernel CR3 through RBX — NOT RCX — or the next `push rcx` would
    // save the kernel CR3 as the "user RIP" and IRETQ would jump into the page
    // tables. This used to clobber RCX and every returning user syscall landed
    // at the CR3 value (a triple fault on next ring-3 instruction).
    "  mov rbx, [rip + kpti_kernel_cr3_slot]",
    "  mov cr3, rbx",
    "  swapgs",
    "  mov gs:[0], rsp",
    "  mov rsp, gs:[8]",
    // Save originals (may be corrupted by heap operations during handler)
    // Spill syscall args 4/5/6 (r10/r8/r9) FIRST — the rotation below
    // overwrites exactly those registers.
    "  push r10", // KSP-8:  arg4 (orig r10)
    "  push r8",  // KSP-16: arg5 (orig r8)
    "  push r9",  // KSP-24: arg6 (orig r9)
    "  push rcx", // KSP-32: user RIP (original, may be corrupted)
    "  push r11", // KSP-40: user RFLAGS (original, may be corrupted)
    // Deep copies (preserved below corrupted area)
    "  push r11", // KSP-48: user RFLAGS (deep copy, safe)
    "  push rcx", // KSP-56: user RIP (deep copy, safe) ← PUSHED LAST
    // Reload arg4/5/6 from the stack slots, before they are clobbered.
    // After the 7 pushes RSP = KSP-56, so relative to RSP:
    //   orig r10 (arg4) @ KSP-8  = RSP+48
    //   orig r8  (arg5) @ KSP-16 = RSP+40
    //   orig r9  (arg6) @ KSP-24 = RSP+32
    "  mov r8, [rsp + 48]", // r8  = arg4 (orig r10)
    "  mov r9, [rsp + 40]", // r9  = arg5 (orig r8)
    "  mov r10, [rsp + 32]",// r10 = arg6 (orig r9)
    // Rotate arg1..arg3 into position: a3=rdx, a2=rsi, a1=rdi, num=rax
    "  mov rcx, rdx",
    "  mov rdx, rsi",
    "  mov rsi, rdi",
    "  mov rdi, rax",
    "  call syscall_dispatch6",
    // RAX now holds the syscall return value. Save it: the code below
    // reuses RAX (to load the user RSP and then the user CR3) before IRETQ,
    // so without this stash every syscall would return the user CR3 instead of
    // its real result. That made cat (which uses read()'s return as the write
    // count) spew 401408 garbage bytes — the null-spew bug.
    "  mov rbx, rax",
    // After call+ret: RSP = KSP - 56 (ret_addr consumed by ret inside dispatch)
    // Pop deep copies, then skip the arg slots + corrupted originals:
    "  pop rcx", // RCX = saved RIP from DEEP COPY (safe), RSP = KSP - 48
    "  pop r11", // R11 = saved RFLAGS from DEEP COPY (safe), RSP = KSP - 40
    // Restore the user-visible arg registers clobbered above.
    "  mov r10, [rsp + 32]", // orig r10 (arg4) @ KSP-8  = RSP+32
    "  mov r8, [rsp + 24]",  // orig r8  (arg5) @ KSP-16 = RSP+24
    "  mov r9, [rsp + 16]",  // orig r9  (arg6) @ KSP-24 = RSP+16
    "  add rsp, 40",        // Skip arg slots + corrupted originals, RSP = KSP
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
    // KPTI: switch back to the user view before dropping to ring 3. The user
    // CR3 slot lives in .kpti_data, mapped in both views.
    "  mov rax, [rip + kpti_user_cr3_slot]",
    "  mov cr3, rax",
    "  swapgs",
    // Restore the syscall return value into RAX. We stashed it in RBX right
    // after the dispatch call; IRETQ only reloads RIP/CS/RFLAGS/RSP/SS, so the
    // remaining registers keep whatever the kernel left in them — meaning RAX
    // would otherwise still hold the user CR3 we just loaded.
    "  mov rax, rbx",
    "  iretq",
    ".att_syntax prefix",
);

extern "C" {
    pub fn syscall_entry();
    pub fn syscall_signal_hook(kernel_rsp: u64);
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

/// Is SMAP actually switched on?
///
/// The page-fault dump uses this to say "you touched a user page without
/// `stac`" instead of leaving the reader to work out why a *present* page
/// produced a supervisor protection violation. Reading CR4 is privileged, so
/// this is bare-metal only and returns `false` on the host.
#[cfg(target_os = "none")]
pub fn smap_enabled() -> bool {
    let cr4: u64;
    unsafe {
        core::arch::asm!("mov {}, cr4", out(reg) cr4, options(nostack, preserves_flags));
    }
    cr4 & (1 << 21) != 0
}

/// Host twin: SMAP is a ring-0 concept and CR4 is unreadable from ring 3.
#[cfg(not(target_os = "none"))]
pub fn smap_enabled() -> bool {
    false
}

/// Enable supervisor access to user pages for the duration of a copy
/// (SMAP's AC flag).
///
/// **Do not add `nomem` here.** The whole point of this function is to open a
/// window in which the *compiler* is not allowed to move a user-memory access
/// across it. With `nomem`, LLVM is told the asm neither reads nor writes
/// memory, so a `read_volatile`/`copy_nonoverlapping` between `stac` and `clac`
/// is free to be scheduled after the `clac` — the access then happens with
/// AC clear and SMAP faults it. That is not hypothetical: it is why
/// `read_user_cstr` and `copy_{from,kernel}_to_user` were
/// "not reliably" reading user memory with SMAP enabled, and why SMAP had to
/// be left off in `apps/src/lib.rs`.
///
/// The `mov cr4` guard that used to be here was also wrong twice over: it
/// added a second memory-opaque asm to the window, and it was pointless, since
/// per Intel SDM Vol 2, `STAC` sets AC whether or not SMAP is enabled, and AC
/// has no effect at all while SMAP is clear.
///
/// `nostack, preserves_flags` is the correct option set: neither instruction
/// touches the stack or flags.
#[inline]
pub unsafe fn stac() {
    core::arch::asm!("stac", options(nostack, preserves_flags));
}

/// Close the window [`stac`] opened. See that function for why this must not be
/// memory-opaque either, and why it must be paired with it.
#[inline]
pub unsafe fn clac() {
    core::arch::asm!("clac", options(nostack, preserves_flags));
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
