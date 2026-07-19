#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

// minimal: write 'KO' to stdout via syscall SYS_WRITE, then exit.
// NO privileged instructions, NO stack reads until syscall.
// Uses only RIP-relative data addressing.
global_asm!(
    ".globl _start",
    "_start:",
    // Write 3 bytes ("KO\n") to fd=1 (stdout)
    "  mov rax, 1",              // SYS_WRITE
    "  mov rdi, 1",              // fd = stdout
    "  lea rsi, [rip + msg]",    // buf = msg
    "  mov rdx, 3",              // len = 3
    "  syscall",
    // Exit with code 0
    "  mov rax, 60",             // SYS_EXIT
    "  xor rdi, rdi",            // code = 0
    "  syscall",
    "msg:",
    "  .ascii \"KO\"",
    "  .byte 10",                // newline (LLVM .ascii doesn't interpret \\n)
);
