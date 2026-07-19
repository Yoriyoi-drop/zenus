#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

global_asm!(
    ".globl _start",
    "_start:",
    "  mov rax, 60",
    "  xor rdi, rdi",
    "  syscall",
);
