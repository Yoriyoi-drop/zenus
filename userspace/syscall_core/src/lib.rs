#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

pub const SYS_READ: u64 = 0;
pub const SYS_WRITE: u64 = 1;
pub const SYS_OPEN: u64 = 2;
pub const SYS_CLOSE: u64 = 3;
pub const SYS_EXIT: u64 = 60;
pub const SYS_EXIT_GROUP: u64 = 231;

#[inline(always)]
pub unsafe fn syscall3(n: u64, a1: u64, a2: u64, a3: u64) -> u64 {
    let r: u64;
    core::arch::asm!(
        "syscall",
        inlateout("rax") n => r,
        in("rdi") a1,
        in("rsi") a2,
        in("rdx") a3,
        lateout("rcx") _,
        lateout("r11") _,
        options(nostack)
    );
    r
}

#[inline(always)]
pub fn sys_write(fd: u64, buf: &[u8]) -> u64 {
    unsafe { syscall3(SYS_WRITE, fd, buf.as_ptr() as u64, buf.len() as u64) }
}

#[inline(always)]
pub fn sys_exit(code: u32) -> ! {
    unsafe {
        core::arch::asm!(
            "syscall",
            in("rax") SYS_EXIT, in("rdi") code as u64,
            options(noreturn)
        );
    }
}

#[inline(always)]
pub fn sys_read(fd: u64, buf: &mut [u8]) -> u64 {
    unsafe { syscall3(SYS_READ, fd, buf.as_ptr() as u64, buf.len() as u64) }
}


