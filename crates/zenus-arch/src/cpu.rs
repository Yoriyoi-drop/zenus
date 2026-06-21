use x86_64::registers::model_specific::Msr;
use crate::gdt;

extern "C" {
    pub fn syscall_dispatch(num: u64, arg1: u64, arg2: u64, arg3: u64) -> u64;
}

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl syscall_entry",
    "syscall_entry:",
    "  push rcx",
    "  push r11",
    "  mov rcx, rdx",
    "  mov r8, rsi",
    "  mov r9, rdi",
    "  mov rdi, rax",
    "  mov rsi, r9",
    "  mov rdx, r8",
    "  call syscall_dispatch",
    "  pop r11",
    "  pop rcx",
    "  sysret",
    ".att_syntax prefix",
);

extern "C" {
    pub fn syscall_entry();
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
        efer.write(efer.read() | (1 << 11)); // set EFER.NXE
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
        cr4 |= 1 << 9;
        cr4 |= 1 << 10;
        core::arch::asm!("mov cr4, {}", in(reg) cr4, options(nostack, preserves_flags));
    }
}

pub fn enable_syscall_ap() {
    let mut lstar = Msr::new(0xC000_0082);
    let mut sfmask = Msr::new(0xC000_0084);
    unsafe {
        lstar.write(syscall_entry as *const () as u64);
        sfmask.write(0xFFFFFFFF);
    }
}

fn enable_syscall() {
    // EFER: enable SCE (System Call Enable)
    let mut efer = Msr::new(0xC000_0080);
    let mut star = Msr::new(0xC000_0081);
    let mut lstar = Msr::new(0xC000_0082);
    let mut sfmask = Msr::new(0xC000_0084);

    // STAR[47:32] = kernel CS for SYSCALL entry (= 0x08)
    // STAR[63:48] = base for SYSRET CS/SS in 64-bit mode:
    //   CS = (STAR[63:48] + 16) | 3 → 0x20|3 = 0x23 = USER_CODE
    //   SS = (STAR[63:48] +  8) | 3 → 0x18|3 = 0x1B = USER_DATA
    // So STAR[63:48] = 0x10 (= KERNEL_DATA raw selector)
    let code_seg: u64 = gdt::KERNEL_CODE.index() as u64 * 8;          // 0x08
    let user_base: u64 = gdt::KERNEL_DATA.index() as u64 * 8;        // 0x10

    unsafe {
        efer.write(efer.read() | 1); // set EFER.SCE bit
        star.write((code_seg << 32) | (user_base << 48));
        lstar.write(syscall_entry as *const () as u64);
        sfmask.write(0xFFFFFFFF);
    }
}

pub unsafe fn write_msr(msr: u32, value: u64) {
    let mut m = Msr::new(msr);
    m.write(value);
}

pub unsafe fn read_msr(msr: u32) -> u64 {
    Msr::new(msr).read()
}

pub fn get_cpu_vendor() -> &'static str {
    let mut eax: u32;
    let mut ebx: u32;
    let mut ecx = 0u32;
    let mut edx = 0u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 0",
            "cpuid",
            "mov {:e}, ebx",
            "pop rbx",
            out(reg) ebx,
            lateout("eax") eax,
            lateout("ecx") ecx,
            lateout("edx") edx,
            options(nostack, preserves_flags)
        );
    }
    let _ = eax;
    static mut VENDOR_BUF: [u8; 12] = [0; 12];
    let buf = unsafe { &mut VENDOR_BUF };
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
    core::str::from_utf8(buf).unwrap_or("Unknown")
}

pub fn has_feature(feature: &str) -> bool {
    let mut ecx = 0u32;
    let mut edx = 0u32;
    unsafe {
        core::arch::asm!(
            "push rbx",
            "mov eax, 1",
            "cpuid",
            "pop rbx",
            lateout("ecx") ecx,
            lateout("edx") edx,
            options(nostack, preserves_flags)
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
