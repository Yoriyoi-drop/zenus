use super::task::{Task, TaskInfo, TaskState, MAX_TASKS};
use core::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use zenus_mem::allocator::ALLOCATOR;
use zenus_ns::{NsId, NS_ROOT};
use zenus_sync::spinlock::SpinLock;

const MAX_ZOMBIES: usize = 64;

#[derive(Clone, Copy)]
pub struct ZombieRecord {
    pub child_pid: u64,
    pub parent_pid: u64,
    pub exit_code: u64,
}

struct ZombieList {
    records: [Option<ZombieRecord>; MAX_ZOMBIES],
    count: usize,
}

impl ZombieList {
    const fn new() -> Self {
        ZombieList {
            records: [None; MAX_ZOMBIES],
            count: 0,
        }
    }

    fn push(&mut self, rec: ZombieRecord) -> bool {
        if self.count >= MAX_ZOMBIES {
            return false;
        }
        self.records[self.count] = Some(rec);
        self.count += 1;
        true
    }

    fn find_by_child(&mut self, child_pid: u64) -> Option<ZombieRecord> {
        for i in 0..self.count {
            if let Some(rec) = self.records[i] {
                if rec.child_pid == child_pid {
                    self.records[i] = None;
                    // Compact
                    self.count -= 1;
                    if i < self.count {
                        self.records.swap(i, self.count);
                    }
                    return Some(rec);
                }
            }
        }
        None
    }

    fn find_any_child(&mut self, parent_pid: u64) -> Option<ZombieRecord> {
        for i in 0..self.count {
            if let Some(rec) = self.records[i] {
                if rec.parent_pid == parent_pid {
                    self.records[i] = None;
                    self.count -= 1;
                    if i < self.count {
                        self.records.swap(i, self.count);
                    }
                    return Some(rec);
                }
            }
        }
        None
    }

    fn reap_all_for_parent(&mut self, parent_pid: u64) {
        let mut i = 0;
        while i < self.count {
            if let Some(rec) = self.records[i] {
                if rec.parent_pid == parent_pid {
                    self.records[i] = None;
                    self.count -= 1;
                    if i < self.count {
                        self.records.swap(i, self.count);
                    }
                    continue;
                }
            }
            i += 1;
        }
    }
}

static ZOMBIE_LIST: SpinLock<ZombieList> = SpinLock::new(ZombieList::new());

static IDLE_RSP: AtomicU64 = AtomicU64::new(0);

pub const TIME_SLICE: u64 = 5;
const MAX_CPUS: usize = 8;

pub const IDLE_TASK_IDX: u32 = u32::MAX;

#[no_mangle]
static CURRENT_TASK: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static CPU_TASK_COUNT: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(1),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
    AtomicU32::new(0),
];
static TASK_COUNT: AtomicU32 = AtomicU32::new(0);
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);
static SYS_TICKS: AtomicU64 = AtomicU64::new(0);

/// CPU that owns the scheduler and the PIC.
const BSP_CPU_ID: u32 = 0;

/// Untouchable zone at the top of every task stack.
///
/// The timer ISR runs on `TSS.RSP0`, which points at the *top* of the
/// interrupted task's stack, and everything `schedule_tick()` calls runs
/// further down from there. A suspended task's saved context also lives on
/// that stack, so as soon as the task was suspended near its stack top -
/// exactly what `idle()` does, it calls `yield_now()` almost immediately -
/// a deep enough ISR descended straight through the saved frame and
/// overwrote it. Restoring it later produced frames whose CS slot held a CR3
/// value or a heap pointer: the "corrupt frame" reports and the jumps into
/// garbage we chased for hours.
///
/// The zone at the top of every task stack reserved for interrupt handling.
///
/// `TSS.RSP0` points at `kernel_rsp_top`, so the CPU pushes the interrupt
/// frame there and `schedule_tick()` keeps descending from it. Task frames
/// must therefore live strictly *below* this zone.
///
/// Layout per task stack (top → bottom):
///   [ GUARD: interrupt area | kernel frames (live + saved) | bottom ]
///
/// With frames at the top of the stack the first timer tick walked straight
/// through them: the ISR pushed its frame over the task's saved context and
/// the next restore picked up a frame whose CS slot held a CR3 value (or, for
/// the idle task, heap pointers instead of a CS selector).
pub const STACK_GUARD: u64 = 32 * 1024;

/// Smallest stack a task may be created with.
///
/// Every constructor computes its frame base as `frame_base(stack_top)` and
/// then writes a `FRAME_BYTES` context frame there, so the stack has to be
/// meaningfully larger than the guard. Below that the subtraction wraps
/// (`u64`) and the frame lands at ~`0xFFFF_FFFF_FFFF_FFF0`.
/// `init::service_register` forwards an arbitrary `stack_size` straight into
/// `create_task_named`, so this cannot be assumed away.
pub const MIN_TASK_STACK: u64 = STACK_GUARD + 4 * 1024;

/// Bytes of context frame per task (15 GPRs + RIP/CS/RFLAGS/RSP/SS).
pub const FRAME_BYTES: u64 = 160;

/// Worst-case distance the timer ISR descends from `TSS.RSP0`: the CPU pushes
/// its 40-byte interrupt frame and `apic_timer_isr_stub` saves 15 GP registers
/// on top of that.
pub const ISR_DESCENT: u64 = 40 + 15 * 8;

/// Would a task created with this `stack_size` get a usable frame?
///
/// Pure predicate, deliberately separated from the constructors so the
/// invariant is testable off-target. The upper bound matters as much as the
/// lower one: an absurd size passes the lower checks, then
/// `stack_base + stack_size` wraps and `frame_base()` becomes a wild address.
pub const fn stack_size_is_valid(stack_size: u64) -> bool {
    stack_size >= MIN_TASK_STACK
        && stack_size <= isize::MAX as u64
        && stack_size - STACK_GUARD >= FRAME_BYTES
}

/// Address of a task's context frame, given the top of its kernel stack.
///
/// EVERY task constructor must build its frame here, and the frame must end at
/// least `ISR_DESCENT` bytes below the top so the timer ISR cannot walk into
/// it. `create_user_task` used to start at `stack_top` — right where
/// `TSS.RSP0` points — so the first tick pushed the CPU frame straight over the
/// task's saved user context.
pub const fn frame_base(stack_top: u64) -> u64 {
    stack_top - STACK_GUARD
}

/// Atomic flag for child exit notification.
/// Format: bit 63 = valid flag, bits 47:0 = exit_code, bits 31:0 = child_pid (OR'd)
/// When bit 63 is set, a child has exited. Cleaned to 0 by the parent after read.
/// This bypasses TASKS/ZOMBIE_LIST which have state visibility issues.
static CHILD_EXIT_NOTIFY: AtomicU64 = AtomicU64::new(0);

static TASKS: SpinLock<TaskArray> = SpinLock::new(TaskArray::new());

struct TaskArray {
    tasks: [Option<Task>; MAX_TASKS],
    next_free: usize,
}

impl TaskArray {
    const fn new() -> Self {
        TaskArray {
            tasks: [None; MAX_TASKS],
            next_free: 0,
        }
    }

    fn find_free(&mut self) -> Option<usize> {
        for i in self.next_free..MAX_TASKS {
            if self.tasks[i].is_none() {
                self.next_free = i + 1;
                return Some(i);
            }
        }
        for i in 0..self.next_free {
            if self.tasks[i].is_none() {
                self.next_free = i + 1;
                return Some(i);
            }
        }
        None
    }

    fn mark_freed(&mut self, idx: usize) {
        if idx < self.next_free {
            self.next_free = idx;
        }
    }
}

// Unified frame format for all Ring 0 context switches:
// Stack layout (from RSP upward):
//   15 GP registers (R15..RAX)
//   5-slot interrupt frame (RIP, CS, RFLAGS, RSP, SS)
// Every producer writes exactly that (timer ISR, context_switch_yield,
// create_task, clone_task) and every consumer finishes with `cli; iretq`,
// which unwinds all 5 slots and restores RFLAGS in one atomic step — so a
// task's RSP is reproduced exactly, with no window in which the timer can
// interrupt a half-unwound frame. Ring 3 frames are the same 5 slots with
// CS=0x23 / SS=0x1b and the user RSP, returned by the same iretq.

// Called from yield_now() — pops return addr, saves 15 regs + 5-slot frame
extern "C" {
    fn context_switch_yield(save_rsp: *mut u64, new_rsp: u64);
}

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl context_switch_yield",
    // Entry: rdi = &save_rsp (destination), rsi = new_rsp (target stack)
    // Saves 15 GP registers + a 5-item interrupt frame
    // Stack layout at switch point (top→bottom):
    //   RSP, SS, RFLAGS, CS, RIP  ← 5-item frame (matches the CPU's own
    //                               interrupt frame, so every restore path
    //                               consumes exactly 40 frame bytes)
    //   R15..RAX                 ← 15 GP registers (ALL preserved)
    //
    // Fix: push all 15 GP registers FIRST (preserving RAX),
    // then write the 5-item frame above them using RAX as temp
    // (RAX is already saved at [rsp+112], so using it as temp is safe).
    //
    // Restore: pops 15 GP regs, then `cli` + `iretq`.
    // iretq consumes all 5 frame slots atomically and restores RFLAGS (with
    // IF) as part of the SAME instruction — no window in which a timer
    // interrupt can be delivered on a half-unwound stack. The old
    // `pop rax / add rsp,8 / popfq / lea rsp,[rsp+16] / jmp rax` sequence
    // enabled IF at `popfq`, i.e. with RSP still mid-unwind: the timer
    // fired inside that 4-instruction window, the ISR saved that
    // half-unwound state as the task's context, and every preemption
    // walked the task's RSP down by 8-16 bytes (stack exhaustion after a
    // few hundred ticks) — see the apic_timer_isr_stub header.
    "context_switch_yield:",
    "  cli",
    // Reserve 32 bytes so the 5-slot frame ([RSP+120..159]) does NOT
    // overwrite the caller's stack above the return address.
    // Entry: RSP = R (points at the return address into the caller).
    // After `sub rsp,32` + 15 pushes, S = R-152, so the frame slots land
    // on R-32..R and the return address is read from [S+152] = [R].
    // The RSP slot gets R+8 (exactly what `ret` would leave behind) and
    // the SS slot gets the current kernel SS (0x10), so iretq returns
    // with the caller's stack pointer and a valid selector.
    "  sub rsp, 32",
    // Save all 15 GP registers first (preserves original RAX, RCX, etc.)
    "  push rax",
    "  push rcx",
    "  push rdx",
    "  push rbx",
    "  push rbp",
    "  push rsi",
    "  push rdi",
    "  push r8",
    "  push r9",
    "  push r10",
    "  push r11",
    "  push r12",
    "  push r13",
    "  push r14",
    "  push r15",
    // Stack: [R15..RAX][32 reserved][return_addr]
    // return_addr is at [rsp + 152] (15 items × 8 + 32 reserved)
    // Read return_addr into rax (RAX is safe — saved at [rsp+112])
    "  mov rax, [rsp + 152]",
    // Write the 5-slot frame ABOVE the 15 regs, overwriting the reserved
    // slots and the return_addr slot itself (already copied into rax).
    // Layout: [rsp+120]=RIP, +128=CS, +136=RFLAGS, +144=RSP, +152=SS.
    "  mov [rsp + 120], rax",
    // Store the REAL CS/SS. Hardcoding 0x08/0x10 stamped ring-0 selectors
    // onto a ring-3 task that called yield_now(), so its next resume ran
    // user code in ring 0 and its user RSP got consumed as a kernel frame.
    "  mov rax, cs",
    "  mov [rsp + 128], rax",
    "  pushfq",
    "  pop rax",
    "  mov [rsp + 136], rax",
    "  or qword ptr [rsp + 136], 0x200",
    "  lea rax, [rsp + 160]", // = R + 8: caller's RSP after `ret`
    "  mov [rsp + 144], rax",
    "  mov rax, ss",
    "  mov [rsp + 152], rax",
    // Save RSP (points to R15) into *save_rsp, then load new RSP
    "  mov [rdi], rsp",
    "  mov rsp, rsi",
    // Restore: 15 GP registers
    "  pop r15",
    "  pop r14",
    "  pop r13",
    "  pop r12",
    "  pop r11",
    "  pop r10",
    "  pop r9",
    "  pop r8",
    "  pop rdi",
    "  pop rsi",
    "  pop rbp",
    "  pop rbx",
    "  pop rdx",
    "  pop rcx",
    "  pop rax",
    // Strict CS dispatch, then ONE atomic `iretq` (same reasoning as the
    // timer ISR: a multi-instruction unwind leaves a window in which an
    // interrupt can be taken after the frame is gone, and the frame's RSP
    // slot is the only exact source of the task's stack pointer — the CPU
    // aligns the stack before pushing an interrupt frame, so
    // frame_base+160 is 8 bytes off whenever the interrupted RSP was
    // 8 mod 16).
    "  cmp qword ptr [rsp + 8], 0x08",
    "  je 2f",
    "  cmp qword ptr [rsp + 8], 0x23",
    "  jne 4f",
    // Ring 3: KERNEL_GS_BASE points at this CPU's PerCpu struct. Zero
    // GS_BASE so user mode can't reach kernel memory through GS.
    "  xor eax, eax",
    "  xor edx, edx",
    "  mov ecx, 0xC0000101",
    "  wrmsr",
    "2:",
    "  cli",
    "  iretq",
    // Frame with a CS that is neither 0x08 nor 0x23.
    // RSP points at the frame's RIP slot here — hand it to the reporter,
    // otherwise it reads its own prologue/locals and prints nonsense.
    "4:",
    "  xor edi, edi",
    "  mov rsi, rsp",
    "  call frame_error_report",
    // `frame_error_report` is `-> !`; trap instead of running off the end of
    // the asm block if that ever changes.
    "  ud2",
    ".att_syntax prefix",
);

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl uart_dbg_putchar",
    "uart_dbg_putchar:",
    "  mov al, dil",
    "  mov dx, 0x3F8",
    "  out dx, al",
    "  ret",
    ".att_syntax prefix",
);

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl safe_yield",
    // Safe yield: enables interrupts, halts, disables interrupts.
    // Implemented as extern "C" function so the compiler must spill all
    // caller-saved registers to the actual stack (not the red zone)
    // before calling. This prevents the timer ISR from corrupting
    // register values stored in the red zone.
    "safe_yield:",
    "  sti",
    "  hlt",
    "  cli",
    "  ret",
    ".att_syntax prefix",
);

extern "C" {
    pub fn safe_yield();
}

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl bochs_putchar",
    "bochs_putchar:",
    "  mov al, dil",
    "  mov dx, 0xE9", // Bochs/QEMU debug port
    "  out dx, al",
    "  ret",
    ".att_syntax prefix",
);

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl apic_timer_isr_stub",
    "apic_timer_isr_stub:",
    // Timer ISR — NO red-zone gap. The `x86_64-unknown-none` target has
    // `disable-redzone: true`, so nothing below the interrupted RSP needs
    // protecting. The old `sub rsp, 128` put the save area 288 bytes below
    // the interrupted RSP while the restore consumed only 144: every
    // preemption leaked 144 bytes of stack. Task stacks live in the kernel
    // heap, so after a few dozen ticks the saved RSP walked below the stack
    // bottom into neighbouring heap blocks and the resumed task jumped into
    // heap data (`#PF RIP=0xffffffff80814f58`, instruction fetch on a
    // BlockHeader holding MAGIC_USED / 0xdeadbeefcafEBABE canaries).
    //
    // Every task frame is now the CPU's own 5-slot interrupt frame:
    //   [S+0..119]  = R15..RAX (15 saved GP registers)
    //   [S+120]     = RIP
    //   [S+128]     = CS
    //   [S+136]     = RFLAGS
    //   [S+144]     = RSP
    //   [S+152]     = SS
    // The ISR pushes 15 regs so the untouched CPU frame lands exactly at
    // [S+120] — the same layout context_switch_yield / create_task write.
    //
    // Both exits converge on `pop x15..x15 / cli / iretq`. iretq unwinds all
    // 5 slots and restores RFLAGS (IF included) as ONE atomic step: there
    // is no window in which a timer interrupt can land on a half-unwound
    // stack. The previous popfq+jmp sequence enabled IF before the frame was
    // finished, so the timer ISR saved a half-unwound context as the task's
    // state and every preemption shifted the task's RSP by 8-16 bytes.
    "  push rax",
    "  push rcx",
    "  push rdx",
    "  push rbx",
    "  push rbp",
    "  push rsi",
    "  push rdi",
    "  push r8",
    "  push r9",
    "  push r10",
    "  push r11",
    "  push r12",
    "  push r13",
    "  push r14",
    "  push r15",
    // The CPU frame (RIP, CS, RFLAGS, RSP, SS) is already at [RSP+120]
    // — nothing to copy. RDI = saved RSP (points to R15 at the bottom of
    // the 15-reg save area).
    "  mov rdi, rsp",
    "  call schedule_tick",
    "  test rax, rax",
    "  jz timer_no_switch",
    // Context switch: rax = new task RSP (15 regs + 5-slot frame)
    "  mov rsp, rax",
    "  jmp timer_restore",
    "timer_no_switch:",
    // Stay on the interrupted context. RSP already points at the task's
    // 15-reg save area (the value `mov rdi, rsp` handed schedule_tick), so
    // fall straight through to the pops — they restore R15..RAX from
    // [RSP..RSP+119] and leave RSP at the CPU frame (base+120), exactly
    // like the switch path. The old `lea rax, [rsp + 120]; mov rsp, rax`
    // advanced RSP onto the CPU frame BEFORE the pops, so every no-switch
    // tick popped the interrupt frame + the caller's stack into R15..RAX
    // and then ran the CS check / iretq 120 bytes too high: the CS slot
    // read stale stack garbage (`BADFRAME src=1` with CS=&TASKS+8,
    // RSP=0) as soon as the scheduler had a tick with nothing to switch
    // to (all other tasks blocked).
    "timer_restore:",
    "  pop r15",
    "  pop r14",
    "  pop r13",
    "  pop r12",
    "  pop r11",
    "  pop r10",
    "  pop r9",
    "  pop r8",
    "  pop rdi",
    "  pop rsi",
    "  pop rbp",
    "  pop rbx",
    "  pop rdx",
    "  pop rcx",
    "  pop rax",
    // Strict CS dispatch: 0x08 = ring 0 (kernel tail), 0x23 = ring 3
    // (iretq). Anything else means the saved frame is corrupt — report it
    // with its five slots instead of jumping into it.
    "  cmp qword ptr [rsp + 8], 0x08",
    "  je timer_iret",
    "  cmp qword ptr [rsp + 8], 0x23",
    "  jne timer_bad_frame",
    // Ring 3: KERNEL_GS_BASE points at this CPU's PerCpu struct, so zero
    // GS_BASE before dropping to user mode.
    "  xor eax, eax",
    "  xor edx, edx",
    "  mov ecx, 0xC0000101",
    "  wrmsr",
    "timer_iret:",
    // One `iretq` for both rings. It loads RIP/CS/RFLAGS/RSP/SS and raises
    // IF as a SINGLE atomic step, which is the only way to resume a task
    // with no window: any multi-instruction unwind (popfq / mov rsp / jmp)
    // can be interrupted after the frame has already been consumed, and the
    // nested ISR then "restores" a frame that no longer exists — it jumped
    // to stack garbage (#PF/#GP with a nonsense error code, seen as
    // `#GP code=0x102 RIP=<ret inside hlt()>`).
    // It also restores the task's EXACT RSP: the CPU aligns the stack
    // before pushing an interrupt frame (`esp &= ~0xf`), so deriving the
    // resume point as frame_base+160 is 8 bytes off whenever the
    // interrupted RSP was 8 mod 16 — which used to hand the shell a stack
    // 8 bytes too low and make its first `ret` jump to a RFLAGS word.
    "  cli",
    "  iretq",
    // Frame with a CS that is neither 0x08 nor 0x23: dump it and stop.
    "timer_bad_frame:",
    "  mov edi, 1",
    "  mov rsi, rsp",
    "  call frame_error_report",
    // `frame_error_report` is `-> !`; trap instead of running off the end of
    // the asm block if that ever changes.
    "  ud2",
    ".att_syntax prefix",
);

pub fn init() {
    let mut tasks = TASKS.lock();

    // Idle task: allocate a 16K kernel stack and construct a 5-item kernel frame
    // so the scheduler can switch to idle() with a valid RSP.
    let (idle_stack_base, _idle_layout) = unsafe { alloc_stack(65536) };
    let idle_stack_top = idle_stack_base.wrapping_add(65536);
    // `idle()`'s asm switches RSP to IDLE_RSP, which sits at the *bottom* of
    // the stack; the whole guard above it belongs to interrupt handling.
    unsafe {
        // Frame layout (memory order, matching every other task):
        //   [rsp+0..119]  = 15 zeroed GP registers
        //   [rsp+120]     = RIP        (idle)
        //   [rsp+128]     = CS         (0x08, ring 0)
        //   [rsp+136]     = RFLAGS     (IF set)
        //   [rsp+144]     = RSP        (frame base = idle_stack_top - GUARD)
        //   [rsp+152]     = SS         (0x10)
        // written upwards from the frame base (writing top-down from the stack
        // base underflowed into the heap block in front of the allocation).
        let f = frame_base(idle_stack_top) as *mut u64;
        for i in 0..15usize {
            f.add(i).write(0);
        }
        f.add(15).write(idle as *const () as usize as u64);
        f.add(16).write(0x08u64);
        f.add(17).write(0x202u64);
        f.add(18).write(frame_base(idle_stack_top));
        f.add(19).write(0x10u64);
        // Zero the interrupt area below the frame so stale heap data can
        // never be mistaken for a frame or a return address.
        let mut clear = idle_stack_base as *mut u64;
        while clear < f {
            clear.write(0);
            clear = clear.add(1);
        }
    }
    let idle_initial_rsp = frame_base(idle_stack_top);

    // TEMP DEBUG: print every stack region so overlaps are visible.
    {
        let boot_rsp: u64;
        unsafe { core::arch::asm!("mov {}, rsp", out(reg) boot_rsp, options(nostack, preserves_flags)) };
        zenus_console::kinfo!(
            "DBG stacks: boot_rsp={:#x} idle={:#x}..{:#x} idlersp={:#x}",
            boot_rsp,
            idle_stack_base,
            idle_stack_top,
            idle_initial_rsp
        );
    }

    let kernel_cr3 = zenus_mem::paging::get_level4_addr().as_u64();
    let mut idle_task = Task::new(0, idle_initial_rsp, "idle");
    idle_task.rsp = idle_initial_rsp;
    idle_task.cr3 = kernel_cr3;
    idle_task.stack_alloc = idle_stack_base;
    idle_task.stack_size = 65536;
    // MUST be set: schedule_tick/yield_now program the TSS RSP0 from
    // `kernel_rsp_top` before switching. Leaving it 0 skipped the update, so
    // the TSS still pointed at the *previous* task's stack top and the timer
    // ISR pushed its frame onto a live task's stack — clobbering the frames
    // and locals of a task that was running (the corrupted frames that the
    // restore path later rejected came from here).
    idle_task.kernel_rsp_top = idle_stack_top;
    tasks.tasks[0] = Some(idle_task);
    IDLE_RSP.store(frame_base(idle_stack_top), Ordering::Release);
    TASK_COUNT.store(1, Ordering::Release);

    zenus_console::kinfo!("Scheduler initialized");
}

fn current_cpu() -> u32 {
    zenus_arch::smp::current_cpu()
}

/// Clone the current task, optionally creating new namespaces.
/// flags: bitmask of CLONE_NEW* constants.
/// Returns the new task's global task ID.
pub fn clone_task(
    flags: u64,
    _stack: u64,
    stack_size: usize,
    entry: u64,
    cr3: u64,
    user_rsp: u64,
    heap_brk: u64,
) -> u64 {
    if !stack_size_is_valid(stack_size as u64) {
        zenus_console::kwarn!(
            "scheduler: rejecting {} byte task stack (minimum is {})",
            stack_size,
            MIN_TASK_STACK
        );
        return 0;
    }
    let cpu = current_cpu();
    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    let parent = match tasks.tasks[current as usize].as_ref() {
        Some(t) => t.clone(),
        None => return 0,
    };
    drop(tasks);

    if entry < 0x1000 || entry >= 0x0000_8000_0000_0000 {
        return 0;
    }

    let (stack_base, _stack_layout) = unsafe { alloc_stack(stack_size) };
    if stack_base == 0 {
        return 0;
    }
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::SeqCst);
    let stack_top = stack_base + stack_size as u64;

    let mut new_uts_ns = parent.uts_ns;
    let mut new_pid_ns = parent.pid_ns;
    let mut new_mnt_ns = parent.mnt_ns;
    let mut new_net_ns = parent.net_ns;
    let mut new_user_ns = parent.user_ns;
    let mut new_ipc_ns = parent.ipc_ns;

    // Create new namespaces if requested
    if flags & zenus_ns::CLONE_NEWUTS != 0 {
        match zenus_ns::uts::create() {
            Some(id) => new_uts_ns = id,
            None => return 0,
        }
    }
    if flags & zenus_ns::CLONE_NEWPID != 0 {
        match zenus_ns::pid::create() {
            Some(id) => new_pid_ns = id,
            None => return 0,
        }
    }
    if flags & zenus_ns::CLONE_NEWNS != 0 {
        match zenus_ns::mnt::create() {
            Some(id) => {
                new_mnt_ns = id;
                if !zenus_fs::vfs::create_mnt_ns(id) {
                    return 0;
                }
            }
            None => return 0,
        }
    }
    if flags & zenus_ns::CLONE_NEWNET != 0 {
        match zenus_ns::net::create() {
            Some(id) => new_net_ns = id,
            None => return 0,
        }
    }
    if flags & zenus_ns::CLONE_NEWUSER != 0 {
        match zenus_ns::user::create() {
            Some(id) => new_user_ns = id,
            None => return 0,
        }
    }
    if flags & zenus_ns::CLONE_NEWIPC != 0 {
        match zenus_ns::ipc::create() {
            Some(id) => new_ipc_ns = id,
            None => return 0,
        }
    }

    unsafe {
        let mut sp = frame_base(stack_top) as *mut u64;
        sp = sp.sub(1);
        sp.write(0x1bu64);
        sp = sp.sub(1);
        sp.write(user_rsp);
        sp = sp.sub(1);
        sp.write(0x202u64);
        sp = sp.sub(1);
        sp.write(0x23u64);
        sp = sp.sub(1);
        sp.write(entry);
        for _ in 0..15 {
            sp = sp.sub(1);
            sp.write(0u64);
        }
        let initial_rsp = sp as u64;

        let mut task = Task::new(
            id,
            initial_rsp,
            core::str::from_utf8(&parent.name).unwrap_or(""),
        );
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.kernel_rsp_top = stack_top;
        task.user_rsp = user_rsp;
        task.cpu = cpu;
        task.cr3 = cr3;
        task.heap_brk = heap_brk;
        task.heap_floor = heap_brk;
        task.uid = parent.uid;
        task.gid = parent.gid;
        task.euid = parent.euid;
        task.egid = parent.egid;
        task.parent_pid = parent.id;
        task.uts_ns = new_uts_ns;
        task.pid_ns = new_pid_ns;
        task.mnt_ns = new_mnt_ns;
        task.net_ns = new_net_ns;
        task.user_ns = new_user_ns;
        task.ipc_ns = new_ipc_ns;

        let mut tasks = TASKS.lock();
        match tasks.find_free() {
            Some(i) => {
                tasks.tasks[i] = Some(task);
                TASK_COUNT.fetch_add(1, Ordering::Release);
            }
            None => {
                dealloc_stack(stack_base, stack_size);
                return 0;
            }
        }
    }

    // Register in PID namespace if it's a new or existing non-root NS
    if new_pid_ns != NS_ROOT {
        zenus_ns::pid::register_task(new_pid_ns, id);
    }

    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    id
}

pub fn create_user_task(
    entry: u64,
    stack_size: usize,
    user_rsp: u64,
    cr3: u64,
    heap_base: u64,
) -> u64 {
    // Validate entry point: must be a canonical user-space address.
    // Entry values in the 1-16MB range likely indicate a physical address
    // was accidentally passed as the virtual entry point.
    if entry < 0x1000 || entry >= 0x0000_8000_0000_0000 {
        return 0;
    }
    if !stack_size_is_valid(stack_size as u64) {
        zenus_console::kwarn!(
            "scheduler: rejecting {} byte task stack (minimum is {})",
            stack_size,
            MIN_TASK_STACK
        );
        return 0;
    }
    let (stack_base, _stack_layout) = unsafe { alloc_stack(stack_size) };
    if stack_base == 0 {
        return 0;
    }
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::SeqCst);
    let stack_top = stack_base + stack_size as u64;
    // Run the new task on the CPU that created it. Handing it to
    // `least_loaded_cpu()` placed the boot-time shell on an AP (cpu=1) that
    // never runs the scheduler, so the BSP could only reach it through the
    // work-steal path.
    let cpu = current_cpu();

    // Frames live below the interrupt area. `create_user_task` used to start
    // at `stack_top`, i.e. inside the zone `TSS.RSP0` points at, so the very
    // first timer tick pushed the CPU frame over the task's saved user
    // context and `run <elf>` / `execve` died with garbage CS/RSP slots.
    let frame_base = frame_base(stack_top);

    let aslr_user_rsp = if user_rsp == 0 {
        let slide = zenus_arch::random::get_random_page_aligned(0, 0x2000_0000u64);
        let rsp = 0x7FFF_FFFF_F000u64.saturating_sub(slide);
        if rsp < 0x1000 {
            0x7FFF_FFFF_F000u64
        } else {
            rsp
        }
    } else {
        user_rsp
    };

    let heap_brk = if heap_base != 0 {
        heap_base
    } else {
        zenus_arch::random::get_random_page_aligned(0x6000_0000_0000u64, 0x6000_0020_0000u64)
    };

    unsafe {
        // Zero the interrupt area so stale heap bytes can never be mistaken
        // for a frame (same treatment as `create_task_named`).
        core::ptr::write_bytes(frame_base as *mut u8, 0, STACK_GUARD as usize);

        let mut sp = frame_base as *mut u64;
        sp = sp.sub(1);
        sp.write(0x1bu64);
        sp = sp.sub(1);
        sp.write(aslr_user_rsp);
        sp = sp.sub(1);
        sp.write(0x202u64);
        sp = sp.sub(1);
        sp.write(0x23u64);
        sp = sp.sub(1);
        sp.write(entry);
        for _ in 0..15 {
            sp = sp.sub(1);
            sp.write(0u64);
        }
        let initial_rsp = sp as u64;

        let mut task = Task::new(id, initial_rsp, "user");
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.kernel_rsp_top = stack_top;
        task.user_rsp = aslr_user_rsp;
        task.cpu = cpu;
        task.cr3 = cr3;
        task.heap_brk = heap_brk;
        task.heap_floor = heap_brk;
        // Capture the parent BEFORE taking TASKS — `current_task_id()`
        // locks TASKS itself, so calling it here deadlocked the CPU.
        task.parent_pid = {
            let cpu = current_cpu();
            let idx = if cpu as usize >= MAX_CPUS {
                0
            } else {
                CURRENT_TASK[cpu as usize].load(Ordering::Acquire)
            } as usize;
            // Read without the lock: the array slot is stable memory and a
            // stale id is harmless here.
            TASKS
                .try_lock()
                .and_then(|g| g.tasks[idx].as_ref().map(|t| t.id))
                .unwrap_or(0)
        };

        let mut tasks = TASKS.lock();
        match tasks.find_free() {
            Some(i) => {
                tasks.tasks[i] = Some(task);
                TASK_COUNT.fetch_add(1, Ordering::Release);
            }
            None => {
                dealloc_stack(stack_base, stack_size);
                return 0;
            }
        }
    }

    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    id
}

pub fn create_task(entry: fn(), stack_size: usize) -> u64 {
    create_task_named(entry, stack_size, "")
}

pub fn create_task_named(entry: fn(), stack_size: usize, name: &str) -> u64 {
    if !stack_size_is_valid(stack_size as u64) {
        zenus_console::kwarn!(
            "scheduler: rejecting {} byte task stack (minimum is {})",
            stack_size,
            MIN_TASK_STACK
        );
        return 0;
    }
    let (stack_base, _stack_layout) = unsafe { alloc_stack(stack_size) };
    if stack_base == 0 {
        return 0;
    }
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::SeqCst);
    let stack_top = stack_base + stack_size as u64;

    // Run the new task on the CPU that created it. An earlier load-balancing
    // helper placed the boot-time shell on an AP (cpu=1), which never runs the
    // scheduler, so the BSP could only reach it via the work-steal path.
    let cpu = current_cpu();

    // Task frames live at the top of the usable area; the bottom
    // STACK_GUARD bytes belong to the interrupt stack (see STACK_GUARD).
    let frame_base = frame_base(stack_top);

    unsafe {
        // Frame written upwards from the frame base — see the identical
        // layout comment in `init()`.
        let f = frame_base as *mut u64;
        for i in 0..15usize {
            f.add(i).write(0);
        }
        f.add(15).write(entry as u64);
        f.add(16).write(0x08u64);
        f.add(17).write(0x202u64);
        f.add(18).write(frame_base);
        f.add(19).write(0x10u64);
        // Zero the interrupt area so stale heap data can never be mistaken
        // for a frame or return address.
        let mut clear = stack_base as *mut u64;
        while clear < f {
            clear.write(0);
            clear = clear.add(1);
        }
        let initial_rsp = frame_base;

        let mut task = Task::new(id, initial_rsp, name);
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.kernel_rsp_top = stack_top;
        task.cpu = cpu;

        let mut tasks = TASKS.lock();
        match tasks.find_free() {
            Some(i) => {
                tasks.tasks[i] = Some(task);
                TASK_COUNT.fetch_add(1, Ordering::Release);
            }
            None => {
                dealloc_stack(stack_base, stack_size);
                return 0;
            }
        }
    }

    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    id
}

unsafe fn dealloc_stack(base: u64, size: usize) {
    if base == 0 || size == 0 {
        return;
    }
    let Ok(layout) = core::alloc::Layout::from_size_align(size, 16) else {
        return;
    };
    alloc::alloc::dealloc(base as *mut u8, layout);
}

unsafe fn alloc_stack(size: usize) -> (u64, core::alloc::Layout) {
    use core::alloc::Layout;
    let Ok(layout) = Layout::from_size_align(size, 16) else {
        return (0, Layout::new::<u8>());
    };
    let ptr = {
        use core::alloc::GlobalAlloc;
        ALLOCATOR.alloc(layout)
    };
    if ptr.is_null() {
        return (0, layout);
    }
    (ptr as u64, layout)
}

/// `hlt` that is safe regardless of the caller's IF state.
///
/// The shell calls `yield_now()` right after `read_line()`, which leaves
/// interrupts DISABLED (it does `sti; hlt; cli` per poll). A bare `hlt`
/// with IF=0 can never wake — no IRQ can be delivered — so the kernel
/// hung permanently after every shell command (gdb showed RFLAGS=0x92 at
/// scheduler.rs hlt sites). Enable interrupts only for the wait, then
/// restore the caller's original state.
fn hlt_wait() {
    let was_enabled = x86_64::instructions::interrupts::are_enabled();
    if !was_enabled {
        x86_64::instructions::interrupts::enable();
    }
    x86_64::instructions::hlt();
    if !was_enabled {
        x86_64::instructions::interrupts::disable();
    }
}

pub fn yield_now() {
    // Saved at entry and restored before returning: yield_now must be
    // interrupt-state-neutral. `context_switch_yield` leaves IF=0 on the
    // resumed side, so a caller that entered with IF=1 (e.g. the idle
    // loop's `sti; hlt`) would otherwise come back with IF=0 and sleep
    // forever, and a caller with IF=0 must not wake up "enabled".
    let irq_was_enabled = x86_64::instructions::interrupts::are_enabled();

    let cpu = current_cpu();
    if (cpu as usize) >= MAX_CPUS {
        hlt_wait();
        return;
    }
    let count = TASK_COUNT.load(Ordering::Acquire);
    if count <= 1 {
        hlt_wait();
        return;
    }

    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    // BUG FIX: Use lock_no_irq() instead of lock() to prevent the spinlock
    // from re-enabling interrupts on drop(). The regular lock() saves the
    // IF flag and restores it on unlock. This creates a race window between
    // unlock (which STIs) and the manual interrupts::disable() that follows.
    // During that window, the timer ISR can acquire TASKS via try_lock()
    // and corrupt the scheduler state while yield_now uses stale data.
    let mut tasks = TASKS.lock_no_irq();

    if tasks.tasks[current as usize].is_none() {
        drop(tasks);
        return;
    }

    let next = find_next_ready(&tasks, current, cpu);
    if next == current {
        drop(tasks);
        hlt_wait();
        return;
    }

    if tasks.tasks[next as usize].is_none() {
        drop(tasks);
        return;
    }

    // Migrate to this CPU if stolen from another
    migrate_task_to_cpu(&mut tasks, next, cpu);

    if let Some(ref next_task) = tasks.tasks[next as usize] {
        if next_task.stack_alloc != 0 && next_task.stack_size > 0 {
            let rsp = next_task.rsp;
            let stack_bottom = next_task.stack_alloc;
            let stack_top = stack_bottom + next_task.stack_size as u64;
            let margin = 256u64;
            if rsp < stack_bottom + margin || rsp >= stack_top {
                drop(tasks);
                return;
            }
        }
    }

    let current_cr3_raw = zenus_mem::paging::get_level4_addr().as_u64();

    if let Some(current_task) = tasks.tasks[current as usize].as_mut() {
        // Don't overwrite Terminated — task is exiting, skip scheduling it
        if current_task.state != TaskState::Terminated {
            current_task.state = TaskState::Ready;
        }
        current_task.cr3 = current_cr3_raw;
    } else {
        drop(tasks);
        return;
    }
    let (next_rsp, next_cr3, do_switch) = match tasks.tasks[next as usize].as_mut() {
        Some(next_task) => {
            next_task.state = TaskState::Running;
            next_task.ticks_left = TIME_SLICE;
            (next_task.rsp, next_task.cr3, true)
        }
        None => (0, 0, false),
    };
    if !do_switch {
        drop(tasks);
        return;
    }

    // Save user_rsp from PerCpu to current task before switching
    if let Some(current_task) = tasks.tasks[current as usize].as_mut() {
        current_task.user_rsp = zenus_arch::cpu::get_percpu_user_rsp(cpu);
    }

    CURRENT_TASK[cpu as usize].store(next, Ordering::Release);

    let next_kernel_rsp = tasks.tasks[next as usize]
        .as_ref()
        .map(|t| t.kernel_rsp_top)
        .unwrap_or(0);
    let next_user_rsp = tasks.tasks[next as usize]
        .as_ref()
        .map(|t| t.user_rsp)
        .unwrap_or(0);
    let save_rsp = match tasks.tasks[current as usize].as_mut() {
        Some(t) => &raw mut t.rsp as *mut u64,
        None => {
            drop(tasks);
            return;
        }
    };
    // Close IRQ window BEFORE releasing the lock, so no interrupt
    // handler can observe the TASKS array in an inconsistent state
    // or steal a context switch via try_lock().
    x86_64::instructions::interrupts::disable();
    // SpinLockGuard::drop() may re-enable IF. Disable again after drop
    // to close the window before context_switch_yield cli.
    drop(tasks);
    x86_64::instructions::interrupts::disable();

    // Restore next task's user_rsp into PerCpu
    if next_user_rsp != 0 {
        zenus_arch::cpu::set_percpu_user_rsp(cpu, next_user_rsp);
    } else {
        zenus_arch::cpu::set_percpu_user_rsp(cpu, 0);
    }

    if next_cr3 != 0 && next_cr3 != current_cr3_raw {
        zenus_mem::paging::set_cr3(next_cr3);
    }

    if next_kernel_rsp != 0 {
        zenus_arch::cpu::set_percpu_kernel_rsp(cpu, next_kernel_rsp);
        zenus_arch::gdt::set_tss_stack(x86_64::VirtAddr::new(next_kernel_rsp));
    }

    // Ensure KERNEL_GS_BASE points to this CPU's PerCpu struct before
    // transitioning. Required when returning to Ring 3 so the next SYSCALL
    // SWAPGS finds the correct GS base. Also safe for Ring 0→0 switches.
    unsafe {
        let percpu_addr = zenus_arch::cpu::percpu_virt_addr(cpu);
        zenus_arch::cpu::write_msr(0xC0000102, percpu_addr);
    }

    {
        let f = next_rsp;
        if f != 0 {
            let cs = unsafe { *(f as *const u64).add(16) };
            if cs != 0x08 && cs != 0x23 {
                emergency_bad_frame("yield", next, f);
            }
        }
    }
    unsafe {
        context_switch_yield(save_rsp, next_rsp);
    }

    // Resumed after another task yielded back to us. The switch path
    // disabled interrupts (cli before context_switch_yield); restore the
    // state the caller entered with (see `irq_was_enabled` at fn top).
    if irq_was_enabled {
        x86_64::instructions::interrupts::enable();
    }
}

pub fn check_yield() {
    yield_now();
}

fn find_next_ready(tasks: &TaskArray, current: u32, cpu: u32) -> u32 {
    // Round-robin: find next ready task after current, wrap around
    for idx in (current + 1)..MAX_TASKS as u32 {
        if idx == 0 {
            continue;
        } // skip idle — only pick as last resort
        if let Some(ref task) = tasks.tasks[idx as usize] {
            if task.is_active() && task.cpu == cpu {
                return idx;
            }
        }
    }
    // Wrap around: scan from 1 to current (skip idle at 0)
    let start = if 1u32 < current { 1u32 } else { u32::MAX }; // start=MAX → loop skipped
    for idx in start..current {
        if let Some(ref task) = tasks.tasks[idx as usize] {
            if task.is_active() && task.cpu == cpu {
                return idx;
            }
        }
    }
    // Steal from other CPUs BEFORE checking idle (prevents scheduler deadlock
    // when tasks are assigned to other CPUs but BSP idles forever).
    // Skip `current`: without this the steal loop finds `current` itself
    // (it scans from index 1) and always returns it, so the caller hits
    // `next == current` and idle (index 0) is never reached — round-robin
    // never parks the current task.
    for idx in 1..MAX_TASKS as u32 {
        if idx == current {
            continue;
        }
        if let Some(ref task) = tasks.tasks[idx as usize] {
            if task.is_active() {
                return idx;
            }
        }
    }
    // Last resort: idle task on this CPU
    if let Some(ref task) = tasks.tasks[0] {
        if task.is_active() && task.cpu == cpu {
            return 0;
        }
    }
    current
}

/// Update a stolen task's CPU affinity when it's migrated to a new CPU.
/// The caller should hold the TASKS lock and call this after find_next_ready
/// when the returned idx belongs to a different CPU.
fn migrate_task_to_cpu(tasks: &mut TaskArray, idx: u32, cpu: u32) {
    if let Some(ref mut task) = tasks.tasks[idx as usize] {
        task.cpu = cpu;
    }
}

pub fn uptime_ticks() -> u64 {
    SYS_TICKS.load(Ordering::Relaxed)
}

pub fn task_count() -> u64 {
    TASK_COUNT.load(Ordering::Acquire) as u64
}

/// Task *index* currently running on this CPU — lock free.
///
/// `current_task_id()` below locks TASKS, and calling it from code that
/// already holds that lock (the idle bridge's frame check, task creation
/// inside a locked region) spins forever with interrupts disabled: the whole
/// machine hangs with the scheduler idle. Use this whenever the caller may
/// already hold TASKS.
pub fn current_task_index() -> u32 {
    let cpu = current_cpu();
    if cpu as usize >= MAX_CPUS {
        return 0;
    }
    CURRENT_TASK[cpu as usize].load(Ordering::Acquire)
}

pub fn current_task_id() -> u64 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.id)
        .unwrap_or(0)
}

pub fn list_tasks() -> [Option<TaskInfo>; MAX_TASKS] {
    let tasks = TASKS.lock();
    let mut result: [Option<TaskInfo>; MAX_TASKS] = [None; MAX_TASKS];
    for (i, t) in tasks.tasks.iter().enumerate() {
        if let Some(ref task) = t {
            result[i] = Some(TaskInfo {
                id: task.id,
                state: task.state,
                cpu: task.cpu,
                uid: task.uid,
                gid: task.gid,
                uts_ns: task.uts_ns,
                pid_ns: task.pid_ns,
                net_ns: task.net_ns,
                user_ns: task.user_ns,
                ipc_ns: task.ipc_ns,
                name: task.name,
            });
        }
    }
    result
}

/// Get the PID namespace of the current task.
pub fn current_pid_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.pid_ns)
        .unwrap_or(0)
}

/// Get the mount namespace of the current task.
pub fn current_mnt_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.mnt_ns)
        .unwrap_or(0)
}

/// Get the UTS namespace of the current task.
pub fn current_uts_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.uts_ns)
        .unwrap_or(0)
}

/// Get the NET namespace of the current task.
pub fn current_net_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.net_ns)
        .unwrap_or(0)
}

/// Get the USER namespace of the current task.
pub fn current_user_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.user_ns)
        .unwrap_or(0)
}

/// Get the IPC namespace of the current task.
pub fn current_ipc_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.ipc_ns)
        .unwrap_or(0)
}

/// Get the local PID for the current task within its PID namespace.
pub fn current_local_pid() -> u64 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    let (tid, pid_ns) = match tasks.tasks[idx as usize].as_ref() {
        Some(t) => (t.id, t.pid_ns),
        None => return 0,
    };
    drop(tasks);
    if pid_ns == NS_ROOT || pid_ns == 0 {
        return tid;
    }
    zenus_ns::pid::local_pid(pid_ns, tid).unwrap_or(tid)
}

pub fn current_uid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.uid)
        .unwrap_or(0)
}

pub fn current_gid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.gid)
        .unwrap_or(0)
}

pub fn current_euid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.euid)
        .unwrap_or(0)
}

pub fn current_egid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize]
        .as_ref()
        .map(|t| t.egid)
        .unwrap_or(0)
}

pub fn set_current_uid(uid: u32) -> bool {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = TASKS.lock();
    if let Some(ref mut t) = tasks.tasks[idx as usize] {
        // Only root (uid=0) or current user can set uid
        if t.euid == 0 || t.euid == uid {
            t.uid = uid;
            t.euid = uid;
            return true;
        }
    }
    false
}

pub fn set_current_gid(gid: u32) -> bool {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = TASKS.lock();
    if let Some(ref mut t) = tasks.tasks[idx as usize] {
        if t.egid == 0 || t.egid == gid {
            t.gid = gid;
            t.egid = gid;
            return true;
        }
    }
    false
}

pub fn get_task_heap_brk(id: u64) -> u64 {
    let tasks = TASKS.lock();
    for t in tasks.tasks.iter() {
        if let Some(ref task) = t {
            if task.id == id {
                let brk = task.heap_brk;
                if brk == 0 {
                    // Default fallback
                    return 0x6000_0000_0000u64;
                }
                return brk;
            }
        }
    }
    0x6000_0000_0000u64
}

/// Lowest address `brk` may shrink this task's heap to.
///
/// Equal to the initial break, i.e. where the loader placed the heap. A
/// program calling `brk(0x1000)` gets `ENOMEM` here instead of a shrink that
/// walks back over its own text segment.
pub fn get_task_heap_floor(id: u64) -> u64 {
    let tasks = TASKS.lock();
    for t in tasks.tasks.iter() {
        if let Some(ref task) = t {
            if task.id == id {
                return task.heap_floor;
            }
        }
    }
    // Same fallback as `get_task_heap_brk`, and deliberately not 0: a floor of 0
    // would make *every* shrink legal, which is the unsafe direction to fail in.
    0x6000_0000_0000u64
}

/// Move the heap floor, for `exec` which loads a different image.
pub fn reset_task_heap_floor(id: u64, floor: u64) {
    let mut tasks = TASKS.lock();
    for t in tasks.tasks.iter_mut() {
        if let Some(ref mut task) = t {
            if task.id == id {
                task.heap_floor = floor;
                return;
            }
        }
    }
}

pub fn set_task_heap_brk(id: u64, brk: u64) {
    let mut tasks = TASKS.lock();
    for t in tasks.tasks.iter_mut() {
        if let Some(ref mut task) = t {
            if task.id == id {
                task.heap_brk = brk;
                return;
            }
        }
    }
}

pub fn set_task_cr3(id: u64, cr3: u64) {
    let mut tasks = TASKS.lock();
    for t in tasks.tasks.iter_mut() {
        if let Some(ref mut task) = t {
            if task.id == id {
                task.cr3 = cr3;
                return;
            }
        }
    }
}

pub fn set_task_name(id: u64, name: &str) {
    let mut tasks = TASKS.lock();
    for t in tasks.tasks.iter_mut() {
        if let Some(ref mut task) = t {
            if task.id == id {
                let len = name.as_bytes().len().min(31);
                task.name = [0u8; 32];
                task.name[..len].copy_from_slice(&name.as_bytes()[..len]);
                return;
            }
        }
    }
}

pub fn get_task(id: u64) -> Option<super::task::Task> {
    let tasks = TASKS.lock();
    for t in tasks.tasks.iter() {
        if let Some(ref task) = t {
            if task.id == id {
                return Some(task.clone());
            }
        }
    }
    None
}

#[no_mangle]
pub extern "C" fn schedule_tick(current_rsp: u64) -> u64 {
    // EOI FIRST, before any early return below. Vector 32 arrives via
    // IOAPIC → LAPIC; the LAPIC keeps its in-service (ISR) bit set for
    // vector 32 until EOI. Without it the LAPIC refuses the NEXT timer
    // interrupt → ticks freeze after the first one (SYS_TICKS stuck),
    // preemption dies, and every `hlt` waiting on a tick hangs forever.
    // PIC EOI too: PIT can arrive via the PIC path (IRQ0), and a pending
    // PIC ISR bit would block IRQ0/1 forever. BSP only — an EOI issued from
    // an AP re-aims the line at the wrong CPU, which is precisely how an AP
    // ended up running a task frame the BSP was already using.
    if current_cpu() == BSP_CPU_ID {
        unsafe {
            core::arch::asm!("out 0x20, al", in("al") 0x20u8, options(nostack, preserves_flags));
        }
    }
    zenus_arch::interrupts::apic::eoi();

    SYS_TICKS.fetch_add(1, Ordering::Relaxed);
    zenus_arch::interrupts::pit::tick();
    // Keep the sysctl uptime counter in step with the scheduler tick; without
    // this call `kernel.uptime` never left 0.
    zenus_fs::sysctl::sysctl_tick();

    let cpu = current_cpu();
    if (cpu as usize) >= MAX_CPUS {
        return 0;
    }
    // Only the BSP owns the scheduler and the task set. An AP that takes a
    // stray IRQ0 (the PIT line is unmasked in the PIC for the BSP's LINT0
    // ExtINT route, and a PIC EOI from the wrong CPU re-aims the line) must
    // NOT enter a task frame: it would resume a task the BSP is running
    // right now, publish that half-finished frame as the task's saved
    // context and leave the AP executing garbage (observed: APs parked at
    // 0xfd0a9 while the shell stalled).
    if cpu != BSP_CPU_ID {
        return 0;
    }
    let count = TASK_COUNT.load(Ordering::Acquire);
    // Hanya satu task (idle) — tidak perlu schedul.
    if count <= 1 {
        return 0;
    }

    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    // Gunakan try_lock() untuk menghindari deadlock: jika scheduler::init()
    // sedang menahan TASKS.lock(), kita skip scheduling kali ini.
    // Timer interrupt bisa datang saat main code path memegang TASKS.lock()
    // (misalnya di scheduler::init()). Karena ISR berjalan dengan IF=0,
    // spinlock deadlock akan terjadi jika kita paksa lock().
    let mut tasks = match TASKS.try_lock() {
        Some(g) => g,
        None => return 0,
    };

    if current as usize >= MAX_TASKS {
        return 0;
    }
    if tasks.tasks[current as usize].is_none() {
        return 0;
    }

    let next = find_next_ready(&tasks, current, cpu);
    if next == current {
        return 0;
    }
    if next as usize >= MAX_TASKS {
        return 0;
    }
    if tasks.tasks[next as usize].is_none() {
        return 0;
    }

    // Migrate to this CPU if stolen from another
    migrate_task_to_cpu(&mut tasks, next, cpu);

    // Validate next task's stack (skip idle task index 0 — its stack is
    // managed separately via IDLE_RSP; saved RSP may point to boot stack).
    if next != 0 {
        if let Some(ref next_task) = tasks.tasks[next as usize] {
            if next_task.stack_alloc != 0 && next_task.stack_size > 0 {
                let rsp = next_task.rsp;
                let stack_bottom = next_task.stack_alloc;
                let stack_top = stack_bottom + next_task.stack_size as u64;
                let margin = 256u64;
                if rsp < stack_bottom + margin || rsp >= stack_top {
                    return 0;
                }
            }
        }
    }

    let current_cr3_raw = zenus_mem::paging::get_level4_addr().as_u64();

    {
        // TEMP DEBUG: compact switch trace (emergency UART waits for THR now).
        // Const-gated: this runs INSIDE the timer ISR with interrupts off and
        // writes ~100 bytes per tick over the polled UART — at the observed
        // tick rate that alone starved both tasks (neither could make any
        // progress between preemptions, so the shell never printed its
        // prompt). Set to true only while debugging the scheduler.
        const SCHED_TRACE: bool = false;
        if SCHED_TRACE {
        let n_rsp = tasks.tasks[next as usize].as_ref().map(|t| t.rsp).unwrap_or(0);
        emergency_str(b"\r\nT");
        emergency_hex(SYS_TICKS.load(Ordering::Relaxed));
        emergency_str(b" ");
        emergency_hex(current as u64);
        emergency_str(b"->");
        emergency_hex(next as u64);
        emergency_str(b" S=");
        emergency_hex(current_rsp);
        emergency_str(b" N=");
        emergency_hex(n_rsp);
        let cs = if n_rsp != 0 { unsafe { *(n_rsp as *const u64).add(16) } } else { 0 };
        emergency_str(b" cs=");
        emergency_hex(cs);
        emergency_str(b" ktop=");
        emergency_hex(
            tasks.tasks[next as usize]
                .as_ref()
                .map(|t| t.kernel_rsp_top)
                .unwrap_or(0),
        );
        }
    }

    // Save current task state
    if let Some(current_task) = tasks.tasks[current as usize].as_mut() {
        // Don't overwrite Terminated — task is exiting, skip scheduling it
        if current_task.state != TaskState::Terminated {
            current_task.state = TaskState::Ready;
        }
        current_task.rsp = current_rsp;
        current_task.cr3 = current_cr3_raw;
        // Stack-underflow tripwire. Task stacks are heap allocations, so a
        // task that blows past its stack bottom silently corrupts the heap
        // block in front of it and only surfaces much later as a mysterious
        // BADFRAME (observed: `dmesg` underflowed the shell stack by ~69 KB
        // and zeroed idle's saved frame). Say it out loud the moment the
        // saved RSP enters the bottom margin. Emergency UART only — we are
        // in the timer ISR with TASKS held.
        const STACK_UNDERFLOW_MARGIN: u64 = 512;
        // Task 0 is the idle task: it runs on the boot stack, which is not a
        // heap allocation, so `stack_alloc` does not describe its real bottom.
        // Comparing the two produced a tripwire hit on *every* tick of an idle
        // loop, and each hit writes ~80 bytes through the polled emergency
        // UART from inside the timer ISR — enough to starve the tasks the
        // scheduler is supposed to be running.
        if current != 0
            && current_task.stack_alloc != 0
            && current_rsp < current_task.stack_alloc + STACK_UNDERFLOW_MARGIN
        {
            emergency_str(b"\r\nSTACK UNDERFLOW task=");
            emergency_hex(current as u64);
            emergency_str(b" rsp=");
            emergency_hex(current_rsp);
            emergency_str(b" bottom=");
            emergency_hex(current_task.stack_alloc);
            emergency_str(b"\r\n");
        }
    }

    let (next_rsp, next_cr3) = match tasks.tasks[next as usize].as_mut() {
        Some(next_task) => {
            next_task.state = TaskState::Running;
            next_task.ticks_left = TIME_SLICE;
            (next_task.rsp, next_task.cr3)
        }
        None => return 0,
    };

    // Save user_rsp from PerCpu
    if let Some(current_task) = tasks.tasks[current as usize].as_mut() {
        current_task.user_rsp = zenus_arch::cpu::get_percpu_user_rsp(cpu);
    }

    CURRENT_TASK[cpu as usize].store(next, Ordering::Release);

    let next_kernel_rsp = tasks.tasks[next as usize]
        .as_ref()
        .map(|t| t.kernel_rsp_top)
        .unwrap_or(0);
    let next_user_rsp = tasks.tasks[next as usize]
        .as_ref()
        .map(|t| t.user_rsp)
        .unwrap_or(0);

    // Close IRQ window before releasing lock
    x86_64::instructions::interrupts::disable();
    drop(tasks);
    x86_64::instructions::interrupts::disable();

    // Restore next task's PerCpu state
    if next_user_rsp != 0 {
        zenus_arch::cpu::set_percpu_user_rsp(cpu, next_user_rsp);
    } else {
        zenus_arch::cpu::set_percpu_user_rsp(cpu, 0);
    }

    if next_cr3 != 0 && next_cr3 != current_cr3_raw {
        zenus_mem::paging::set_cr3(next_cr3);
    }

    if next_kernel_rsp != 0 {
        zenus_arch::cpu::set_percpu_kernel_rsp(cpu, next_kernel_rsp);
        zenus_arch::gdt::set_tss_stack(x86_64::VirtAddr::new(next_kernel_rsp));
    }

    unsafe {
        let percpu_addr = zenus_arch::cpu::percpu_virt_addr(cpu);
        zenus_arch::cpu::write_msr(0xC0000102, percpu_addr);
    }

    next_rsp
}

/// Called from the restore asm when a saved frame's CS is neither the ring 0
/// (0x08) nor the ring 3 (0x23) selector: the frame is corrupt, so dump it
/// and park the CPU instead of jumping into garbage.
#[no_mangle]
extern "C" fn frame_error_report(which: u32, frame: u64) -> ! {
    // Lock-free on purpose: this runs from the restore path with the
    // scheduler lock possibly still held by an interrupted `yield_now()`
    // (its TASKS guard lives on the stack we are switching away from), so
    // taking TASKS or the log lock here deadlocked the whole machine and
    // hid the very frame we need to see. Everything goes straight to the
    // UART.
    //
    // `frame` is passed in from the asm caller (RSP at the restore site,
    // pointing at the frame's RIP slot). Reading our own RSP instead used
    // to dump this function's prologue/locals — every RIP/CS/RSP value it
    // printed was garbage and hid the real corrupt word.
    let f = frame;
    let p = f as *const u64;
    let (rip, cs, rflags, frsp, ss) = unsafe {
        (
            p.read(),
            p.add(1).read(),
            p.add(2).read(),
            p.add(3).read(),
            p.add(4).read(),
        )
    };
    emergency_str(b"\r\nBADFRAME src=");
    emergency_hex(which as u64);
    emergency_str(b" frame=");
    emergency_hex(f);
    emergency_str(b" task=");
    emergency_hex(current_task_index() as u64);
    emergency_str(b" ticks=");
    emergency_hex(SYS_TICKS.load(Ordering::Relaxed));
    emergency_str(b"\r\n  RIP=");
    emergency_hex(rip);
    emergency_str(b" CS=");
    emergency_hex(cs);
    emergency_str(b" F=");
    emergency_hex(rflags);
    emergency_str(b" RSP=");
    emergency_hex(frsp);
    emergency_str(b" SS=");
    emergency_hex(ss);
    emergency_str(b"DUMP:\r\n");
    // Dump memory around the frame: tells us WHO wrote those words.
    // Dump the frame AND the words just below it: a timer_no_switch frame
    // is the CPU's own frame, so a wrong base shows up as the CS slot holding
    // something that is not 0x08/0x23 and the neighbouring slots as garbage.
    for i in -8isize..8 {
        let addr = (f as isize + i * 8) as u64;
        let word = unsafe { *(addr as *const u64) };
        emergency_str(b" ");
        if i == 0 {
            emergency_str(b">");
        } else {
            emergency_str(b" ");
        }
        emergency_hex(addr);
        emergency_str(b"=");
        emergency_hex(word);
        emergency_str(b"\r\n");
    }
    // Refuse to run the corrupt frame: park this CPU.
    x86_64::instructions::interrupts::disable();
    loop {
        x86_64::instructions::hlt()
    }
}

/// Emergency (lock-free) report of a frame whose CS is not a ring 0/3
/// selector, emitted *before* the switch that would restore it.
fn emergency_bad_frame(tag: &str, task: u32, rsp: u64) {
    let p = rsp as *const u64;
    let (rip, cs, fl, frsp, ss) = unsafe {
        (
            p.add(15).read(),
            p.add(16).read(),
            p.add(17).read(),
            p.add(18).read(),
            p.add(19).read(),
        )
    };
    emergency_str(b"\r\nBADFRAME(");
    emergency_str(tag.as_bytes());
    emergency_str(b") task=");
    emergency_hex(task as u64);
    emergency_str(b" rsp=");
    emergency_hex(rsp);
    emergency_str(b" ticks=");
    emergency_hex(SYS_TICKS.load(Ordering::Relaxed));
    emergency_str(b"\r\n  RIP=");
    emergency_hex(rip);
    emergency_str(b" CS=");
    emergency_hex(cs);
    emergency_str(b" F=");
    emergency_hex(fl);
    emergency_str(b" RSP=");
    emergency_hex(frsp);
    emergency_str(b" SS=");
    emergency_hex(ss);
    emergency_str(b"\r\n");
}

fn emergency_str(s: &[u8]) {
    for &b in s {
        zenus_console::serial::uart_write_byte_emergency(b);
    }
}

fn emergency_hex(mut v: u64) {
    // Exactly the 16 digits — the old code stamped a NUL terminator at
    // tmp[16] and then wrote the whole 17-byte buffer to the UART, so every
    // hex field in the emergency log was followed by a stray 0x00 byte
    // (serial logs came out as `T0000000000000001<NUL> ...`).
    let mut tmp = [0u8; 16];
    let mut i = 16;
    while i > 0 {
        i -= 1;
        tmp[i] = b"0123456789abcdef"[(v & 0xf) as usize];
        v >>= 4;
    }
    emergency_str(&tmp);
}

/// Dump the task table over the emergency UART (no locks held).
///
/// Diagnostic only — the boot path used to call this on every boot, which
/// pushed a raw task dump into the console log on every single start. Kept as
/// an API because it is the only way to inspect tasks while the CPU is wedged.
#[allow(dead_code)]
pub fn debug_dump_tasks_emergency() {
    let tasks = TASKS.lock();
    {
        emergency_str(b"TASKS:\r\n");
        for idx in 0..4usize {
            match tasks.tasks[idx].as_ref() {
                Some(t) => {
                    emergency_str(b"  t");
                    emergency_hex(idx as u64);
                    emergency_str(b" id=");
                    emergency_hex(t.id);
                    emergency_str(b" st=");
                    emergency_hex(t.state as u64);
                    emergency_str(b" cpu=");
                    emergency_hex(t.cpu as u64);
                    emergency_str(b" rsp=");
                    emergency_hex(t.rsp);
                    emergency_str(b" stk=");
                    emergency_hex(t.stack_alloc);
                    emergency_str(b" sz=");
                    emergency_hex(t.stack_size);
                    emergency_str(b" ur=");
                    emergency_hex(t.user_rsp);
                    emergency_str(b" ktop=");
                    emergency_hex(t.kernel_rsp_top);
                    emergency_str(b"\r\n");
                }
                None => {
                    emergency_str(b"  t");
                    emergency_hex(idx as u64);
                    emergency_str(b" none\r\n");
                }
            }
        }
    }
}

pub fn idle() -> ! {
    // Switch to the idle task's dedicated stack FIRST, then yield to the shell task.
    // This ensures the idle task's context is saved on its own stack.
    //
    // The yield goes through `idle_yield_bridge` instead of calling
    // `yield_now` directly: the bridge validates the target task's saved
    // frame before every switch, so a corrupted frame is reported with its
    // five slots instead of being jumped into (which showed up as a jump to
    // 0x202 — a RFLAGS word — and a bogus instruction fetch).
    unsafe {
        core::arch::asm!(
            "cli",
            "mov rsp, {idle_rsp}",
            "sti",
            "call {bridge}",
            // After the bridge returns (when the shell task yields back),
            // enter the HLT loop.
            // `sti` BEFORE every hlt: the switch path leaves IF=0 on the
            // resumed side, and an idle hlt with IF=0 never wakes →
            // nothing (not even the timer) could ever run this CPU again.
            "2:",
            "sti",
            "hlt",
            "jmp 2b",
            idle_rsp = sym IDLE_RSP,
            bridge = sym idle_yield_bridge,
            options(noreturn)
        );
    }
}

/// Idle loop with a watchdog predicate, mirroring [`idle()`] but able to
/// return.
///
/// `cond` runs on the idle task's own stack with interrupts enabled. While it
/// returns `false` the loop yields to the other tasks through
/// [`yield_now`]; the first `true` returns control to the caller.
///
/// The fuzzing harness needs this: a bare `yield_now()` from the boot task
/// keeps the boot task's frame on the boot stack instead of `IDLE_RSP`, and
/// the context switch never completes — the boot task simply spins and the
/// campaign task is never entered.
/// Spin on the idle stack until `cond` returns true, then run `finished`.
///
/// `finished` is a separate `fn() -> !` on purpose. This function switches
/// RSP to `IDLE_RSP`, so control can never come back to the caller's frame:
/// if `cond` returned true and the asm simply fell through, the following `ret`
/// would pop a "return address" out of the idle task's frame and jump to
/// garbage. That made `fuzz_runner`'s `abort("watchdog")` unreachable — the
/// watchdog path could never report a timeout.
pub fn idle_until(cond: fn() -> bool, finished: fn() -> !) {
    let check: fn() -> bool = cond;
    unsafe {
        core::arch::asm!(
            "cli",
            "mov rsp, {idle_rsp}",
            "sti",
            "3:",
            "call {check}",
            "test al, al",
            "jnz 4f",
            // `sti` before every hlt: the switch path leaves IF=0 on the
            // resumed side, and an hlt with IF=0 never wakes.
            "sti",
            "hlt",
            "jmp 3b",
            "4:",
            "call {done}",
            // `done` never returns; trap rather than fall through into
            // whatever follows.
            "ud2",
            idle_rsp = sym IDLE_RSP,
            check = in(reg) check,
            done = in(reg) finished,
            // The predicates are ordinary Rust `fn` pointers; the `call`s above
            // go through them, so nothing else needs to know it is Rust-ABI.
        );
    }
}

/// Called from `idle()`'s asm, already running on `IDLE_RSP`.
#[no_mangle]
extern "C" fn idle_yield_bridge() -> ! {
    loop {
        yield_now();
    }
}

pub fn ap_idle() -> ! {
    loop {
        unsafe {
            core::arch::asm!("sti", "hlt", options(nostack));
        }
    }
}

#[derive(Clone, Copy)]
pub struct TerminatedStack {
    pub base: u64,
    pub size: usize,
}

struct TerminatedStackList {
    stacks: [Option<TerminatedStack>; 64],
    count: usize,
}

const EMPTY_TERM_STACK: Option<TerminatedStack> = None;
const fn empty_term_array() -> [Option<TerminatedStack>; 64] {
    [EMPTY_TERM_STACK; 64]
}

static TERMINATED_STACKS: zenus_sync::spinlock::SpinLock<TerminatedStackList> =
    zenus_sync::spinlock::SpinLock::new(TerminatedStackList {
        stacks: empty_term_array(),
        count: 0,
    });

pub fn reap_terminated_stacks() {
    let mut list = TERMINATED_STACKS.lock();
    for i in 0..list.count {
        if let Some(ts) = list.stacks[i].take() {
            if let Ok(layout) = core::alloc::Layout::from_size_align(ts.size, 16) {
                unsafe {
                    alloc::alloc::dealloc(ts.base as *mut u8, layout);
                }
            }
        }
    }
    list.count = 0;
}

pub fn exit_current_task(code: u64) -> ! {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = TASKS.lock();

    // Mark task as Terminated and store zombie record.
    // Do NOT free the task slot or any resources yet — yield_now() needs
    // the slot to exist for context_switch_yield to save the current state.
    // The slot and resources will be freed later by the parent (via
    // reap_task()) when it reaps the zombie.
    let (task_id, parent_pid) = match tasks.tasks[idx as usize].as_ref() {
        Some(t) => (t.id, t.parent_pid),
        None => {
            drop(tasks);
            yield_now();
            loop {
                x86_64::instructions::hlt();
            }
        }
    };

    tasks.tasks[idx as usize].as_mut().unwrap().state = TaskState::Terminated;
    tasks.tasks[idx as usize].as_mut().unwrap().exit_code = code;

    // Set atomic child-exit notification (bypasses ZOMBIE_LIST issues)
    CHILD_EXIT_NOTIFY.store(
        (1u64 << 63) | ((code & 0xFFFF) << 32) | (task_id & 0xFFFFFFFF),
        Ordering::Release,
    );

    // Store zombie record so parent can reap
    {
        let mut zombies = ZOMBIE_LIST.lock();
        zombies.push(ZombieRecord {
            child_pid: task_id,
            parent_pid: parent_pid,
            exit_code: code,
        });
        // Zombie is stored; parent will call reap_task() to free resources
    }
    drop(tasks);

    // Deliver SIGCHLD to parent process
    signal_deliver(parent_pid, super::signal::SIGCHLD);

    // Write debug marker before yield (visible in serial/log)
    unsafe {
        core::arch::asm!("out 0xe9, al", in("al") b'X', options(nostack, preserves_flags));
    }

    // Switch to the next ready task via yield_now().
    // Since this task is Terminated, is_active() returns false and
    // find_next_ready will skip it.
    yield_now();

    // Never reached (should be scheduled over), but safe fallback:
    loop {
        x86_64::instructions::hlt();
    }
}

pub fn wait_for_child(parent_pid: u64, child_pid: u64, options: u64) -> Option<(u64, u64)> {
    let mut zombies = ZOMBIE_LIST.lock();
    let result = if child_pid == -1i64 as u64 || child_pid == 0 {
        if options == 1 {
            // WNOHANG — check but don't block
            Some(zombies.find_any_child(parent_pid))
        } else {
            // Block until a child exits
            Some(zombies.find_any_child(parent_pid))
        }
    } else {
        if options == 1 {
            Some(zombies.find_by_child(child_pid))
        } else {
            Some(zombies.find_by_child(child_pid))
        }
    };
    drop(zombies);

    match result {
        Some(Some(rec)) => Some((rec.child_pid, rec.exit_code)),
        Some(None) => None,
        None => None,
    }
}

/// Reap a terminated task: free its stack, address space, and task slot.
/// Called by the parent after wait_for_child() returns the zombie.
pub fn reap_task(task_id: u64) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        let task_info = {
            let t = &tasks.tasks[i];
            match t {
                Some(tc) if tc.id == task_id && tc.state == TaskState::Terminated => {
                    Some((tc.stack_alloc, tc.stack_size, tc.cpu, tc.cr3, tc.pid_ns))
                }
                _ => None,
            }
        };
        if let Some((stack_alloc, stack_size, task_cpu, cr3, pid_ns)) = task_info {
            // Free PID namespace registration
            if pid_ns != 0 {
                zenus_ns::pid::unregister_task(pid_ns, task_id);
            }
            // Free the task slot
            tasks.mark_freed(i);
            tasks.tasks[i] = None;
            drop(tasks);
            // Update CPU task count
            CPU_TASK_COUNT[task_cpu as usize].fetch_sub(1, Ordering::SeqCst);
            TASK_COUNT.fetch_sub(1, Ordering::Release);
            // Free user address space (switches to kernel CR3 if currently active)
            if cr3 != 0 {
                zenus_mem::paging::destroy_address_space(cr3);
            }
            // Free the task's kernel stack directly (NOT via TERMINATED_STACKS to avoid double-free)
            if stack_alloc != 0 && stack_size > 0 {
                if let Ok(layout) = core::alloc::Layout::from_size_align(stack_size as usize, 16) {
                    unsafe {
                        alloc::alloc::dealloc(stack_alloc as *mut u8, layout);
                    }
                }
            }
            return;
        }
    }
}

/// Check if any child has exited via the atomic notification flag.
/// Returns Some((child_pid, exit_code)) if a child exit was recorded.
/// Clears the flag — only returns each exit once.
pub fn check_child_exit_notify() -> Option<(u64, u64)> {
    let val = CHILD_EXIT_NOTIFY.swap(0, Ordering::AcqRel);
    if val & (1u64 << 63) != 0 {
        let child_pid = val & 0xFFFFFFFF;
        let exit_code = (val >> 32) & 0xFFFF;
        Some((child_pid, exit_code))
    } else {
        None
    }
}

pub fn reap_zombies_for_parent(parent_pid: u64) {
    let mut zombies = ZOMBIE_LIST.lock();
    zombies.reap_all_for_parent(parent_pid);
}

pub fn task_exit() {
    let cpu = current_cpu();
    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = TASKS.lock();
    let (stack_alloc, stack_size, task_cpu, task_cr3, task_pid_ns, task_id) = {
        let task = &tasks.tasks[current as usize];
        (
            task.as_ref().map(|t| t.stack_alloc),
            task.as_ref().map(|t| t.stack_size),
            task.as_ref().map(|t| t.cpu),
            task.as_ref().map(|t| t.cr3),
            task.as_ref().map(|t| t.pid_ns),
            task.as_ref().map(|t| t.id),
        )
    };
    tasks.mark_freed(current as usize);
    tasks.tasks[current as usize] = None;
    TASK_COUNT.fetch_sub(1, Ordering::Release);
    drop(tasks);
    if let (Some(sa), Some(ss), Some(tc)) = (stack_alloc, stack_size, task_cpu) {
        if sa != 0 && ss > 0 {
            let mut list = TERMINATED_STACKS.lock();
            let idx = list.count;
            if idx < 64 {
                list.stacks[idx] = Some(TerminatedStack {
                    base: sa,
                    size: ss as usize,
                });
                list.count = idx + 1;
            }
            drop(list);
        }
        CPU_TASK_COUNT[tc as usize].fetch_sub(1, Ordering::SeqCst);
    }
    if let (Some(pid_ns), Some(tid)) = (task_pid_ns, task_id) {
        if pid_ns != 0 {
            zenus_ns::pid::unregister_task(pid_ns, tid);
        }
    }
    // Free the user address space
    if let Some(cr3) = task_cr3 {
        if cr3 != 0 {
            zenus_mem::paging::destroy_address_space(cr3);
        }
    }
    loop {
        x86_64::instructions::hlt();
    }
}

pub fn kill_task(id: u64) -> bool {
    if id == 0 {
        return false;
    }
    let cpu = current_cpu();
    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = TASKS.lock();

    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == id && i as u32 == current {
                return false;
            }
        }
    }

    for i in 0..MAX_TASKS {
        let task_info = {
            let t = &tasks.tasks[i];
            match t {
                Some(tc) if tc.id == id && tc.is_active() => Some((
                    tc.cpu,
                    tc.stack_alloc,
                    tc.stack_size,
                    tc.cr3,
                    tc.state,
                    tc.pid_ns,
                    tc.id,
                )),
                _ => None,
            }
        };
        if let Some((task_cpu, stack_alloc, stack_size, cr3, state, pid_ns, tid)) = task_info {
            if pid_ns != 0 {
                zenus_ns::pid::unregister_task(pid_ns, tid);
            }
            tasks.mark_freed(i);
            if state == TaskState::Running {
                tasks.tasks[i] = None;
                CPU_TASK_COUNT[task_cpu as usize].fetch_sub(1, Ordering::SeqCst);
                if stack_alloc != 0 && stack_size > 0 {
                    unsafe {
                        if let Ok(layout) =
                            core::alloc::Layout::from_size_align(stack_size as usize, 16)
                        {
                            alloc::alloc::dealloc(stack_alloc as *mut u8, layout);
                        }
                    }
                }
                if cr3 != 0 {
                    zenus_mem::paging::destroy_address_space(cr3);
                }
                TASK_COUNT.fetch_sub(1, Ordering::Release);
                return true;
            }
            tasks.tasks[i] = None;
            CPU_TASK_COUNT[task_cpu as usize].fetch_sub(1, Ordering::SeqCst);
            if stack_alloc != 0 && stack_size > 0 {
                unsafe {
                    if let Ok(layout) =
                        core::alloc::Layout::from_size_align(stack_size as usize, 16)
                    {
                        alloc::alloc::dealloc(stack_alloc as *mut u8, layout);
                    }
                }
            }
            // Free the task's address space (user pages, page tables)
            if cr3 != 0 {
                zenus_mem::paging::destroy_address_space(cr3);
            }
            TASK_COUNT.fetch_sub(1, Ordering::Release);
            return true;
        }
    }
    false
}

// ── Signal helpers ──

pub fn get_task_id_by_pid(pid: u64) -> Option<u64> {
    let tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == pid {
                return Some(task.id);
            }
        }
    }
    None
}

pub fn signal_deliver(task_id: u64, sig: usize) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                task.deliver_signal(sig);
                if task.state == TaskState::Sleeping || task.state == TaskState::Waiting {
                    task.state = TaskState::Ready;
                }
                return;
            }
        }
    }
}

pub fn signal_force_kill(task_id: u64) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                task.state = TaskState::Terminated;
                task.exit_code = 9 << 8; // killed by signal 9
                return;
            }
        }
    }
}

pub fn signal_clear(task_id: u64, sig: usize) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                task.clear_signal(sig);
                return;
            }
        }
    }
}

pub fn task_set_state(task_id: u64, state: TaskState) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                task.state = state;
                return;
            }
        }
    }
}

pub fn get_signal_action(task_id: u64, sig: usize) -> super::task::SignalAction {
    let tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == task_id {
                return task.signal_actions[sig];
            }
        }
    }
    super::task::SignalAction::new()
}

pub fn set_signal_action(task_id: u64, sig: usize, action: super::task::SignalAction) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                task.signal_actions[sig] = action;
                return;
            }
        }
    }
}

pub fn get_signal_mask(task_id: u64) -> u64 {
    let tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == task_id {
                return task.signal_mask;
            }
        }
    }
    0
}

pub fn set_signal_mask(task_id: u64, mask: u64) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                task.signal_mask = mask;
                return;
            }
        }
    }
}

pub fn check_and_deliver_signal(
    user_rsp: u64,
    user_rip: u64,
    user_rflags: u64,
) -> Option<(u64, u64, u64)> {
    let task_id = current_task_id();
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                if let Some((sig, _disposition)) = super::signal::check_pending(task) {
                    let new_rsp = super::signal::setup_signal_frame(
                        task,
                        sig,
                        user_rsp,
                        user_rip,
                        user_rflags,
                    )?;
                    let handler = task.signal_actions[sig].handler_fn;
                    let new_rflags = user_rflags & !0x100; // clear TF
                    return Some((new_rsp, handler, new_rflags));
                }
                return None;
            }
        }
    }
    None
}

/// Called from syscall return path. Checks pending signals and modifies
/// kernel stack to redirect to signal handler if needed.
/// kernel_rsp points to [rflags, rip] on the kernel stack.
pub fn check_signal_for_sysret(kernel_rsp: u64) -> bool {
    let task_id = current_task_id();
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                if let Some((sig, _disposition)) = super::signal::check_pending(task) {
                    let saved_rip = unsafe { *(kernel_rsp as *const u64).add(1) };
                    let saved_rflags = unsafe { *(kernel_rsp as *const u64) };
                    let cpu = zenus_arch::smp::current_cpu();
                    let user_rsp = zenus_arch::cpu::get_percpu_user_rsp(cpu);

                    let new_rsp = match super::signal::setup_signal_frame(
                        task,
                        sig,
                        user_rsp,
                        saved_rip,
                        saved_rflags,
                    ) {
                        Some(r) => r,
                        None => return false,
                    };

                    let handler = task.signal_actions[sig].handler_fn;
                    let new_rflags = saved_rflags & !0x100;

                    unsafe {
                        *(kernel_rsp as *mut u64) = new_rflags;
                        *(kernel_rsp as *mut u64).add(1) = handler;
                        zenus_arch::cpu::set_percpu_user_rsp(cpu, new_rsp);
                    }
                    return true;
                }
                return false;
            }
        }
    }
    false
}

/// Handle rt_sigreturn syscall: restore user context from signal frame.
/// user_rsp points to the signal frame that was pushed by setup_signal_frame.
/// Returns (new_rsp, new_rip, new_rflags) to restore user execution.
pub fn rt_sigreturn_restore(user_rsp: u64) -> Option<(u64, u64, u64)> {
    if let Some((restored_rsp, saved_ctx)) = super::signal::restore_signal_frame(user_rsp) {
        let task_id = current_task_id();
        let mut tasks = TASKS.lock();
        for i in 0..MAX_TASKS {
            if let Some(ref mut task) = tasks.tasks[i] {
                if task.id == task_id {
                    // Update saved context for next context switch
                    task.saved_context = saved_ctx;
                    return Some((restored_rsp, saved_ctx.rip, saved_ctx.rflags));
                }
            }
        }
    }
    None
}

// ── VMA helpers ──

pub fn get_task_cr3(task_id: u64) -> Option<u64> {
    let tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == task_id {
                return Some(task.cr3);
            }
        }
    }
    None
}

pub fn with_vma<F, R>(task_id: u64, f: F) -> Option<R>
where
    F: FnOnce(&super::task::VmaTable) -> R,
{
    let tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == task_id {
                return Some(f(&task.vma));
            }
        }
    }
    None
}

pub fn with_vma_mut<F, R>(task_id: u64, f: F) -> Option<R>
where
    F: FnOnce(&mut super::task::VmaTable) -> R,
{
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                return Some(f(&mut task.vma));
            }
        }
    }
    None
}

// ── CWD helpers ──

pub fn get_task_cwd(task_id: u64) -> Option<[u8; 256]> {
    let tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == task_id {
                return Some(task.cwd);
            }
        }
    }
    None
}

pub fn set_task_cwd(task_id: u64, path: &str) {
    let mut tasks = TASKS.lock();
    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == task_id {
                let mut buf = [0u8; 256];
                let len = path.len().min(255);
                buf[..len].copy_from_slice(&path.as_bytes()[..len]);
                task.cwd = buf;
                return;
            }
        }
    }
}
