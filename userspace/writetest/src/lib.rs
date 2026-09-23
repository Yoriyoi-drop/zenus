#![no_std]

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

// Test: 2 write syscalls ("A" + "B") then infinite NOP loop.
// NO int3 — test normal 2-syscall flow directly.
core::arch::global_asm!(
    ".globl _start",
    "_start:",
    // write(1, msg_a, 1)
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + msg_a]",
    "  mov rdx, 1",
    "  syscall",
    // NO int3 here — direct second syscall
    // write(1, msg_b, 1)
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + msg_b]",
    "  mov rdx, 1",
    "  syscall",
    // Infinite NOP loop
    "1:",
    "  nop",
    "  jmp 1b",
    "msg_a:",
    "  .ascii \"A\"",
    "msg_b:",
    "  .ascii \"B\"",
);
