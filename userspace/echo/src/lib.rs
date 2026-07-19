#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

global_asm!(
    ".globl _start",
    "_start:",
    "  mov r12, [rsp]",
    "  lea r13, [rsp + 8]",
    "  xor r14d, r14d",
    "1:",
    "  cmp r14, r12",
    "  jge 2f",
    "  mov rdi, [r13 + r14*8]",
    "  call print_str",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + newline]",
    "  mov rdx, 1",
    "  syscall",
    "  inc r14",
    "  jmp 1b",
    "2:",
    "  mov rax, 60",
    "  xor rdi, rdi",
    "  syscall",
    "print_str:",
    "  push r12",
    "  push r14",
    "  mov r12, rdi",
    "  xor r14d, r14d",
    "3:",
    "  cmp byte ptr [r12 + r14], 0",
    "  je 4f",
    "  inc r14",
    "  jmp 3b",
    "4:",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  mov rsi, r12",
    "  mov rdx, r14",
    "  syscall",
    "  pop r14",
    "  pop r12",
    "  ret",
    "newline:",
    "  .byte 10",
);
