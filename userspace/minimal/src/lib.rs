#![no_std]
#![no_main]

use core::arch::asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

unsafe fn sys_write(fd: u64, buf: u64, count: u64) -> u64 {
    let ret: u64;
    asm!(
        "syscall",
        in("rax") 1u64,
        in("rdi") fd,
        in("rsi") buf,
        in("rdx") count,
        out("rcx") _,
        out("r11") _,
        lateout("rax") ret,
        options(nostack, preserves_flags)
    );
    ret
}

unsafe fn sys_exit(code: u64) -> ! {
    asm!(
        "syscall",
        in("rax") 60u64,
        in("rdi") code,
        options(noreturn)
    );
}

#[no_mangle]
pub unsafe extern "C" fn _start() -> ! {
    // Force string to be on stack, not in .rodata
    // This tests whether the issue is related to .rodata section access
    let msg = [b'K', b'O', b'\n'];
    sys_write(1, msg.as_ptr() as u64, msg.len() as u64);
    sys_exit(0);
}
