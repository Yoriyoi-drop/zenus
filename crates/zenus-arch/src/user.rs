use crate::cpu;
use crate::gdt;

pub fn switch_to_user(entry: u64, stack_top: u64) -> ! {
    let user_cs = gdt::USER_CODE.index() as u64 * 8 | 3;
    let user_ss = gdt::USER_DATA.index() as u64 * 8 | 3;
    let user_ds = gdt::USER_DATA.index() as u64 * 8 | 3;

    let rsp = stack_top;

    // Set KERNEL_GS_BASE to PerCpu address so the next SYSCALL's
    // SWAPGS produces a valid GS base. DO NOT zero it.
    let percpu_addr = cpu::percpu_virt_addr(0);
    unsafe { cpu::write_msr(0xC0000102, percpu_addr); }

    unsafe {
        core::arch::asm!(
            "mov ds, {user_ds}",
            "mov es, {user_ds}",
            "mov fs, {user_ds}",
            "mov gs, {user_ds}",
            "xor eax, eax",
            "xor edx, edx",
            "mov ecx, 0xC0000101",
            "wrmsr",
            "push {user_ss}",
            "push {rsp}",
            "push 0x202",
            "push {user_cs}",
            "push {entry}",
            "iretq",
            user_ds = in(reg) user_ds,
            user_ss = in(reg) user_ss,
            rsp = in(reg) rsp,
            user_cs = in(reg) user_cs,
            entry = in(reg) entry,
            options(noreturn)
        )
    }
}
