#![no_std]

use core::arch::global_asm;

// cat: read stdin (fd 0) 4KB at a time, write to stdout (fd 1)
global_asm!(
    ".globl _start",
    "_start:",
    "  sub rsp, 4096",
    "1:",
    "  mov rax, 0",
    "  mov rdi, 0",
    "  mov rsi, rsp",
    "  mov rdx, 4096",
    "  syscall",
    "  test rax, rax",
    "  jle 2f",
    "  mov rdx, rax",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  mov rsi, rsp",
    "  syscall",
    "  jmp 1b",
    "2:",
    "  add rsp, 4096",
    "  mov rax, 60",
    "  xor rdi, rdi",
    "  syscall",
);

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}
