use crate::gdt;

pub fn switch_to_user(entry: u64, stack_top: u64) -> ! {
    let user_cs = gdt::USER_CODE.index() as u64 * 8 | 3;
    let user_ss = gdt::USER_DATA.index() as u64 * 8 | 3;
    let user_ds = gdt::USER_DATA.index() as u64 * 8 | 3;

    let rsp = stack_top;

    unsafe {
        core::arch::asm!(
            "mov ds, {user_ds}",
            "mov es, {user_ds}",
            "mov fs, {user_ds}",
            "mov gs, {user_ds}",
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
