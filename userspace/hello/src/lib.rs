#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

global_asm!(
    ".globl _start",
    "_start:",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + msg]",
    "  mov rdx, 22",
    "  syscall",
    "  mov rax, 60",
    "  xor rdi, rdi",
    "  syscall",
    "msg:",
    "  .ascii \"Hello from userspace!\\n\"",
);
