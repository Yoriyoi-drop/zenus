#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

// TEST 5: Write "A\n" via syscall, then small busy-loop, then
// write "B\n" via syscall, then big busy-loop, repeat.
// No privileged instructions (no `out`).
// Purpose: see if MULTIPLE syscalls work, or if the 2nd syscall
// always crashes.
global_asm!(
    ".globl _start",
    "_start:",
    // write(1, msg_a, 2)
    "1:",
    "  mov rax, 1",              // sys_write
    "  mov rdi, 1",              // fd = stdout
    "  lea rsi, [rip + msg_a]",  // buf = msg_a
    "  mov rdx, 2",              // len = 2
    "  syscall",
    // Small busy-wait loop to let timer potentially fire
    "  mov rcx, 100000",
    "2:",
    "  dec rcx",
    "  jnz 2b",
    // write(1, msg_b, 2)
    "  mov rax, 1",              // sys_write
    "  mov rdi, 1",              // fd = stdout
    "  lea rsi, [rip + msg_b]",  // buf = msg_b
    "  mov rdx, 2",              // len = 2
    "  syscall",
    // Big busy-wait loop
    "  mov rcx, 5000000",
    "3:",
    "  dec rcx",
    "  jnz 3b",
    "  jmp 1b",
    "msg_a:",
    "  .ascii \"A\\n\"",
    "msg_b:",
    "  .ascii \"B\\n\"",
);
