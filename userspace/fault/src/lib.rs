//! A program that dereferences a null pointer, to prove the kernel kills *it*
//! and not the machine (BUG-042).
//!
//! It prints a marker, stores to address 0, and — only if that did not kill it —
//! prints a second marker and exits 0. On a kernel that survives a user fault the
//! process dies at the store, so the second marker never appears in the log and
//! the shell is still alive afterwards.

#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop {
        core::hint::spin_loop()
    }
}

global_asm!(
    ".globl _start",
    "_start:",
    // write(1, before, 23)
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + before]",
    "  mov rdx, 23",
    "  syscall",
    // *(volatile u64*)0 = 0x41414141  -> user write to the null page
    "  mov rax, 0x41414141",
    "  mov qword ptr [0], rax",
    // Only reached if the fault was *not* fatal to this process.
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + survived]",
    "  mov rdx, 22",
    "  syscall",
    "  mov rax, 60",
    "  xor rdi, rdi",
    "  syscall",
    "before:",
    "  .ascii \"FAULT: about to die...\\n\"",
    "survived:",
    "  .ascii \"FAULT: I SURVIVED IT!\\n\"",
);
