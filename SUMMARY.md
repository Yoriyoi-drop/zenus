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
- **Fixed immediate output flush**: added `flush_output()` calls after `execute()`, after character echo, after backspace echo, and after newline echo in the shell. Previously output accumulated in `OUTPUT_BUF` until the next PIT tick (~1ms delayed), causing typing to feel unresponsive.
- Verified boot succeeds: full output shows SMP, shell, PIT scheduling messages.

### In Progress
- **SMP balancing**: `find_next_ready` steals from other CPUs when current CPU has no ready task.

### Resolved
- **Boot crash after `make clean` was a timeout issue**: kernel booted fine. GDB confirmed entry point is hit. Clean debug build takes ~8.5s to initialize (135 MB BSS, 30+ workspace crates recompiled). With 9+ second timeout, the kernel boots successfully to shell prompt.

## Key Decisions
- **PIT → apic_timer_isr_stub**: Instead of a separate Rust `x86-interrupt` handler, reuse the existing assembly stub (saves all GP registers, calls `schedule_tick`, conditionally switches RSP) so context switching logic is unified.
- **`schedule_tick` runs preemptively**: Each PIT tick (~1 ms) now does full context switch logic inside the ISR, enabling true preemptive multitasking.
- **Shell spawned as scheduler task**: `shell_task()` is created via `create_task` with tid=1, BSP enters `scheduler::idle()`. PIT tick switches from idle to shell, shell runs until tick or yield, then back to idle.
- **Immediate flush in shell**: `flush_output()` is now called after every `execute()`, each character echo, backspace echo, and newline echo — so interactive typing feels instantaneous. Previously only the prompt write had a flush; all other output accumulated until the next PIT tick or `yield_now()`.

## Next Steps
1. Complete SMP balancing: ensure round-robin/least-loaded CPU selection in `create_task`, ensure `CPU_TASK_COUNT` is updated on migration.
2. Wire up syscall passthrough (`sys_write`, `sys_read`) so user-mode tasks can use kernel services.
3. Validate full preemptive multitasking with multiple tasks (e.g., shell + echo server).

## Critical Context
- PIT at 1193182 Hz, divisor ~11932 → ~100 Hz (10 ms period). Reduced from 1000 Hz to 100 Hz to cut TCG emulation overhead by 10x. PIC master IMR at port `0x21`; clearing bit 0 enables IRQ0.
- LINT0 ExtINT (`0x700`) configured only on BSP via `enable_pic_lint0()`. APs keep LINT0 masked in `enable_lapic()`.
- `apic_timer_isr_stub` (scheduler.rs line 199) saves all GP regs, calls `schedule_tick(rsp)`. If return value is non-zero, stub restores that RSP and iretq to it (context switch). If zero, restores original regs and returns.
- `schedule_tick` does: PIC EOI → APIC EOI → pit::tick → flush_output → save/load task state → return next RSP.
- `find_next_ready` performs two passes: first for tasks on `cpu`, then for any active task (stealing).
- `IDLE_TASK_IDX = u32::MAX` (4294967295). `CURRENT_TASK[cpu]` stores this when idle runs.
- `Shell::run()` calls `yield_now()` every 10 iterations and flushes output after each command.
- BSS segment is ~23 MB (largest contributor: `HEAP` at 16 MB, reduced from 128 MB). Boot to shell prompt takes ~4.4s in QEMU TCG mode (down from ~20s+).

## Relevant Files
- `apps/src/shell.rs`: Shell with immediate flush after execute(), char echo, backspace, newline.
- `crates/zenus-sched/src/scheduler.rs`: `schedule_tick`, `find_next_ready`, `yield_now`, `idle()`.
- `apps/src/lib.rs`: `entry()` and `shell_task()`.
- `crates/zenus-console/src/serial.rs`: `OUTPUT_BUF`, `flush_output()`.
- `crates/zenus-arch/src/interrupts/`: PIT, APIC, IDT setup.
- `crates/zenus-sync/src/spinlock.rs`: SpinLock with IRQ-disabling on lock.
- `apps/src/linker.ld`: Contains `KEEP(*(.limine_reqs))` at start of `.text`.
