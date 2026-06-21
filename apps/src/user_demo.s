BITS 64
ORG 0x400000

; Minimal user-space program
; Demonstrates Ring 3 execution with syscalls

section .text
global _start
_start:
    ; write(1, msg, len)
    mov eax, 1          ; SYS_WRITE
    mov edi, 1          ; stdout
    lea rsi, [rel msg]
    mov edx, 23         ; length
    syscall

    ; infinite loop (exit syscall causes problems without kernel stack switch)
.loop:
    jmp .loop

section .data
msg: db "Hello from user mode!", 10
