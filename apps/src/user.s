BITS 64
ORG 0x400000

section .text
global _start
_start:
    mov eax, 1
    mov edi, 1
    lea rsi, [rel msg]
    mov edx, 23
    syscall

.loop:
    jmp .loop

section .data
msg: db "Hello from user mode!", 10
