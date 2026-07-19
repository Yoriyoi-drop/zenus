#![no_std]

use core::arch::global_asm;

#[panic_handler]
fn panic(_: &core::panic::PanicInfo) -> ! {
    loop { core::hint::spin_loop() }
}

// args: print argc and all argv entries, one per line.
// Format:
//   argc=N
//   argv[0]=<path>
//   argv[1]=<arg1>
//   ...
//
// Stack convention (Linux): [RSP] = argc, [RSP+8] = argv[0], ... [RSP+8*argc] = NULL
//
// Register usage:
//   r12 = argc (preserved throughout)
//   r13 = argv pointer array base
//   r14 = current index
//   r15 = temporary for string length / decimal conversion
global_asm!(
    ".globl _start",
    "_start:",
    // Save argc and argv base
    "  mov r12, [rsp]",          // r12 = argc
    "  lea r13, [rsp + 8]",      // r13 = argv pointer array
    "",
    // Print "argc="
    "  mov rax, 1",              // sys_write
    "  mov rdi, 1",              // fd = stdout
    "  lea rsi, [rip + argc_str]",
    "  mov rdx, 5",              // len("argc=")
    "  syscall",
    "",
    // Print argc as decimal
    "  mov rax, r12",            // argc value
    "  lea r15, [rip + dec_buf_end]",
    "1:",
    "  dec r15",
    "  xor rdx, rdx",
    "  mov rbx, 10",
    "  div rbx",                 // rax = quotient, rdx = remainder
    "  add dl, '0'",             // convert to ASCII
    "  mov byte ptr [r15], dl",  // use 'byte ptr' for Intel syntax
    "  test rax, rax",
    "  jnz 1b",
    // Print the decimal string
    "  mov rax, 1",
    "  mov rdi, 1",
    "  mov rsi, r15",
    "  lea r8, [rip + dec_buf_end]",
    "  sub r8, r15",             // r8 = length
    "  mov rdx, r8",
    "  syscall",
    "",
    // Print newline
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + newline]",
    "  mov rdx, 1",
    "  syscall",
    "",
    // Loop over argv entries
    "  xor r14d, r14d",          // r14 = index
    "2:",
    "  cmp r14, r12",            // compare with argc
    "  jge 5f",                  // if >= argc, done
    "",
    // Print "argv["
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + argv_pre]",
    "  mov rdx, 5",
    "  syscall",
    "",
    // Print index as decimal
    "  mov rax, r14",
    "  lea r15, [rip + dec_buf_end]",
    "3:",
    "  dec r15",
    "  xor rdx, rdx",
    "  mov rbx, 10",
    "  div rbx",
    "  add dl, '0'",
    "  mov byte ptr [r15], dl",
    "  test rax, rax",
    "  jnz 3b",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  mov rsi, r15",
    "  lea r8, [rip + dec_buf_end]",
    "  sub r8, r15",
    "  mov rdx, r8",
    "  syscall",
    "",
    // Print "]="
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + argv_eq]",
    "  mov rdx, 2",
    "  syscall",
    "",
    // Print argv[r14]
    "  mov r15, [r13 + r14*8]",   // r15 = argv[r14] pointer
    "  test r15, r15",
    "  jz 4f",
    // Find string length (use r8 for length, r15 for pointer)
    "  xor r8d, r8d",
    "6:",
    "  cmp byte ptr [r15 + r8], 0",
    "  je 7f",
    "  inc r8",
    "  jmp 6b",
    "7:",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  mov rsi, r15",
    "  mov rdx, r8",
    "  syscall",
    "",
    // Print newline
    "4:",
    "  mov rax, 1",
    "  mov rdi, 1",
    "  lea rsi, [rip + newline]",
    "  mov rdx, 1",
    "  syscall",
    "",
    "  inc r14",
    "  jmp 2b",
    "",
    // Exit
    "5:",
    "  mov rax, 60",             // sys_exit
    "  xor rdi, rdi",            // code = 0
    "  syscall",
    "",
    // Read-only string data stays in .text (mapped read-execute)
    "argc_str:",
    "  .ascii \"argc=\"",
    "argv_pre:",
    "  .ascii \"argv[\"",
    "argv_eq:",
    "  .ascii \"]=\"",
    "newline:",
    "  .byte 10",
    // Writable dec_buf goes to .bss (mapped read-write via FLAGS(6))
    ".pushsection .bss",
    ".globl dec_buf",
    "dec_buf:",
    "  .space 20",
    "dec_buf_end:",
    "  .byte 0",
    ".popsection",
);
