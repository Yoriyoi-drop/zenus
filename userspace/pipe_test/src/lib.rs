#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

// pipe_test: create a pipe, write, read back, print result, exit.
//
// Stack layout:
//   [rsp]     = fds[0] (read fd)
//   [rsp+8]   = fds[1] (write fd)
//   [rsp+16]  = read buffer (64 bytes)
//
// Algorithm:
//   1. sys_pipe(fds)  → SYS_PIPE=22, rdi=rsp
//   2. Write "Hello from pipe!\n" to write_fd via SYS_WRITE=1
//   3. Read from read_fd via SYS_READ=0
//   4. Write buffer to stdout (fd 1)
//   5. sys_exit(0)

global_asm!(
    ".globl _start",
    "_start:",
    // Allocate stack space: 8 bytes for fds + 64 bytes for read buffer + padding
    "  sub rsp, 96",
    "",
    // 1. sys_pipe(fds) — create pipe, fds at [rsp]
    "  mov rax, 22",              // SYS_PIPE
    "  mov rdi, rsp",             // fds buffer
    "  xor rsi, rsi",             // flags (0)
    "  xor rdx, rdx",             // unused
    "  syscall",
    "  test rax, rax",
    "  jnz pipe_fail",            // if return != 0, pipe failed",
    "",
    // Read fds from stack
    "  mov r12d, [rsp]",          // r12 = read_fd (lower 32 bits)
    "  mov r13d, [rsp + 4]",      // r13 = write_fd (lower 32 bits)",
    "",
    // 2. Write message to write_fd
    "  mov rax, 1",               // SYS_WRITE
    "  mov rdi, r13",             // fd = write_fd
    "  lea rsi, [rip + msg]",     // buf = msg
    "  mov rdx, 17",              // count = len(\"Hello from pipe!\\n\")",
    "  syscall",
    "",
    // 3. Read from read_fd into buffer at [rsp+16]
    "  mov rax, 0",               // SYS_READ
    "  mov rdi, r12",             // fd = read_fd
    "  lea rsi, [rsp + 16]",      // buf
    "  mov rdx, 64",              // count
    "  syscall",
    "  mov r14, rax",             // r14 = bytes read
    "",
    // 4. Write prefix \"pipe: \" to stdout
    "  mov rax, 1",
    "  mov rdi, 1",               // fd = stdout
    "  lea rsi, [rip + prefix]",
    "  mov rdx, 6",               // len(\"pipe: \")
    "  syscall",
    "",
    // Write buffer to stdout
    "  mov rax, 1",
    "  mov rdi, 1",               // fd = stdout
    "  lea rsi, [rsp + 16]",      // buf
    "  mov rdx, r14",             // count = bytes read
    "  syscall",
    "",
    // 5. Exit with code 0
    "  mov rax, 60",              // SYS_EXIT
    "  xor rdi, rdi",             // code = 0
    "  syscall",
    "",
    // Error path: print \"pipe: FAIL\\n\" and exit with code 1
    "pipe_fail:",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + fail_msg]",
    "  mov rdx, 11",              // len(\"pipe: FAIL\\n\")
    "  syscall",
    "  mov rax, 60",
    "  mov rdi, 1",               // exit code 1
    "  syscall",
    "",
    // Data
    "msg:",
    "  .ascii \"Hello from pipe!\\n\"",
    "prefix:",
    "  .ascii \"pipe: \"",
    "fail_msg:",
    "  .ascii \"pipe: FAIL\\n\"",
);
