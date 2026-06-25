## Goal
Enable interactive shell with preemptive multitasking, SMP load balancing, and syscall passthrough.

## Constraints & Preferences
- Must work in QEMU TCG mode (no KVM).
- Shell reads from both PS/2 keyboard and serial UART.
- Output goes through global `OUTPUT_BUF`, flushed on PIT ticks and context switches.
- APs share kernel page tables with BSP; LINT0 ExtINT only configured on BSP.

## Progress
### Done
- Replaced dummy `shell_task()` (KKKKKKK loop) with `shell::Shell::new().run()` – full interactive shell with prompt (`zenus$ `), command parsing (ps, uname, meminfo, help, etc.), and readline via serial + keyboard.
- Removed APIC timer init (`init_timer(48)`) – PIT at ~1 kHz is the sole scheduling tick source.
- Rewired IDT vector 32 (PIT) to `apic_timer_isr_stub` instead of Rust `x86-interrupt` handler – piggybacks on the existing assembly stub that saves GP registers and calls `schedule_tick`.
- Rewrote `schedule_tick()` to perform real preemptive context switching: PIC EOI → APIC EOI → pit::tick → flush_output → save current task RSP/CR3/state → find_next_ready → load next task's RSP/CR3/TSS/KERNEL_GS_BASE → return new RSP.
- Added `migrate_task_to_cpu()` helper and modified `find_next_ready` to first prefer tasks on current CPU, then fall back to stealing from other CPUs.
- Fixed `CURRENT_TASK == IDLE_TASK_IDX` (u32::MAX) handling in both `yield_now` and `schedule_tick` – idle is not a real task entry; skip save/restore for it.
- Verified boot succeeds: full output shows SMP, shell, PIT scheduling messages.

### In Progress
- **SMP balancing**: `find_next_ready` steals from other CPUs when current CPU has no ready task.
- Scheduler code saved to `crates/zenus-sched/src/scheduler.rs`.

### Resolved
- **Boot crash after `make clean` was a red herring**: kernel booted fine all along. GDB confirmed entry point is hit, serial output begins, kernel fully boots. The 5-second QEMU timeout was too short — kernel now takes ~8.5 seconds to initialize (clean debug build recompiles all 30+ workspace crates from scratch). With a 9+ second timeout, the kernel boots successfully and shows the shell prompt. The `-debugcon` bochs port capture doesn't work with this QEMU version — serial output is the correct way to verify boot.
- GDB session confirmed: breakpoint at `entry()` hit, RSP valid (`0xffff80007ffab090`), CPU in long mode (CR0 PG, EFER LME+LMA), paging active (CR3=0x7ff9b000). The kernel code at `entry()` (file offset 0x102c0 within `.text`) executes correctly with `sub $0xf68, %rsp`, `out $0xe9, $0x5a`, followed by function pointer calls.

## Key Decisions
- **PIT → apic_timer_isr_stub**: Instead of a separate Rust `x86-interrupt` handler, reuse the existing assembly stub (saves all GP registers, calls `schedule_tick`, conditionally switches RSP) so context switching logic is unified.
- **`schedule_tick` runs preemptively**: Each PIT tick (~1 ms) now does full context switch logic inside the ISR, enabling true preemptive multitasking for the first time.
- **Shell spawned as scheduler task**: `shell_task()` is created via `create_task` with tid=1, BSP enters `scheduler::idle()`. PIT tick switches from idle to shell, shell runs until tick or yield, then back to idle.
- **No flush_output in schedule_tick**: Removed from the ISR to avoid SpinLock deadlock (if a task is preempted while holding `OUTPUT_BUF`). flush_output is instead called from `yield_now()` (already there) and directly in the shell loop.

## Next Steps
1. **Performance**: Investigate why clean debug build takes ~8.5s to boot. Try release build (`make run CARGO_FLAGS=--release`) for faster initialization.
2. Complete SMP balancing: ensure round-robin/least-loaded CPU selection in `create_task`, ensure `CPU_TASK_COUNT` is updated on migration.
3. Wire up syscall passthrough (`sys_write`, `sys_read`) so user-mode tasks can use kernel services.
4. Validate full preemptive multitasking with multiple tasks (e.g., shell + echo server).

## Critical Context
- PIT at 1193182 Hz, divisor ~1193 → ~1000 Hz (1 ms period). PIC master IMR at port `0x21`; clearing bit 0 enables IRQ0.
- LINT0 ExtINT (`0x700`) configured only on BSP via `enable_pic_lint0()`. APs keep LINT0 masked in `enable_lapic()`.
- `apic_timer_isr_stub` (scheduler.rs line 199) saves all GP regs, calls `schedule_tick(rsp)`. If return value is non-zero, stub restores that RSP and iretq to it (context switch). If zero, restores original regs and returns.
- `schedule_tick` does: PIC EOI (`out 0x20, 0x20`) → APIC EOI → pit::tick → flush_output → save current task (skip if IDLE) → find_next_ready (steal from other CPUs) → load next task's RSP/CR3/TSS/kernel_rsp/GS_BASE → return next RSP.
- `find_next_ready` performs two passes: first for tasks on `cpu`, then for any active task (stealing).
- `IDLE_TASK_IDX = u32::MAX` (4294967295). `CURRENT_TASK[cpu]` stores this when idle runs. `schedule_tick` and `yield_now` both check for this sentinel before indexing `tasks.tasks[]`.
- `Shell::run()` calls `yield_now()` every 10 iterations and when no serial/keyboard data is available. `yield_now()` calls `hlt()` when only one task is active (idle doesn't count as a real task).
- `.text` layout in final ELF: `.limine_reqs` data (0x110 bytes) → kernel code starting at `0x80001110`. Entry point `0x800102c0` through `ENTRY(entry)` in linker script. All 6 Limine request statics verified with correct magic IDs via LLVM IR and raw hex dump.
- BSS segment is ~135 MB (largest contributor: `HEAP` at 128 MB). Fresh build takes longer to boot because all 30+ workspace crates are recompiled in debug mode.

## Relevant Files
- `crates/zenus-arch/src/interrupts/apic.rs`: `enable_pic_lint0()` (BSP-only LINT0 ExtINT); LINT0 now masked in `enable_lapic()` by default.
- `crates/zenus-arch/src/interrupts/pit.rs`: PIT init; `tick()`.
- `crates/zenus-arch/src/interrupts/handler.rs`: `interrupt_timer` removed from IDT (still defined but dead code).
- `crates/zenus-arch/src/interrupts/idt.rs`: Vector 32 → `apic_timer_isr_stub` with `disable_interrupts(true)`; vector 48 stub kept but unused.
- `crates/zenus-arch/src/limine.rs`: All 6 Limine request statics (`BASE_REVISION`, `MP_REQUEST`, `HHDM_REQUEST`, `RSDP_REQUEST`, `MEMMAP_REQUEST`, `MODULE_REQUEST`) in `.limine_reqs` with `#[used]`.
- `crates/zenus-sched/src/scheduler.rs`: `schedule_tick` (full context switch), `find_next_ready` (steal from other CPUs), `migrate_task_to_cpu`, `yield_now` (cooperative switch), `idle()` (BSP idle loop), `ap_idle()` (AP idle loop), `create_task`.
- `apps/src/lib.rs`: `entry()` spawns shell via `create_task(shell_task, 65536)`, then calls `scheduler::idle()`. `shell_task()` calls `shell::Shell::new().run()`.
- `apps/src/shell.rs`: Full interactive shell with `Shell::run()`, `read_line()`, `Writer` impl, command dispatcher (ps, uname, meminfo, help, uptime, etc.).
- `crates/zenus-console/src/serial.rs`: `OUTPUT_BUF`, `flush_output()`, `uart_putchar`.
- `apps/src/linker.ld`: Contains `KEEP(*(.limine_reqs))` at start of `.text`.
- `Makefile`: `run-bios` target updated from `-serial mon:stdio` to `-serial stdio`.
