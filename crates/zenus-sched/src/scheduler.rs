use core::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;
use super::task::{Task, TaskInfo, TaskState, MAX_TASKS};
use zenus_mem::allocator::ALLOCATOR;
use zenus_ns::{NsId, NS_ROOT};

static IDLE_RSP: AtomicU64 = AtomicU64::new(0);

pub const TIME_SLICE: u64 = 5;
const MAX_CPUS: usize = 8;

static CURRENT_TASK: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];
static CPU_TASK_COUNT: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(1), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];
static TASK_COUNT: AtomicU32 = AtomicU32::new(0);
static NEXT_TASK_ID: AtomicU64 = AtomicU64::new(1);

static TASKS: SpinLock<TaskArray> = SpinLock::new(TaskArray::new());

struct TaskArray {
    tasks: [Option<Task>; MAX_TASKS],
}

impl TaskArray {
    const fn new() -> Self {
        TaskArray { tasks: [None; MAX_TASKS] }
    }
}

// Unified frame format for all Ring 0 context switches:
// Stack layout (from RSP upward):
//   15 GP registers (R15..RAX)
//   interrupt frame (RFLAGS, CS, RIP)
// Exit via pop rax; add rsp, 8; popfq; jmp rax
// Ring 3 (user) tasks use 5-item frame (SS, RSP, RFLAGS, CS, RIP) + iretq.

// Called from yield_now() — pops return addr, saves 15 regs + 3-item frame
extern "C" {
    fn context_switch_yield(save_rsp: *mut u64, new_rsp: u64);
}

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl context_switch_yield",
    // Entry: rdi = &save_rsp (destination), rsi = new_rsp (target stack)
    // Saves 15 GP registers + 3-item interrupt frame (RFLAGS, CS, RIP)
    // Stack layout at switch point (top→bottom):
    //   RFLAGS, CS, RIP      ← 3-item frame (Ring 0→Ring 0)
    //   R15..RAX             ← 15 GP registers (ALL preserved)
    // Total frame: 18 × 8 = 144 bytes
    //
    // Fix: push all 15 GP registers FIRST (preserving RAX),
    // then write the 3-item frame above them using RAX as temp
    // (RAX is already saved at [rsp+112], so using it as temp is safe).
    //
    // Restore: pops 15 GP regs, then checks CS.RPL:
    //   RPL=0 → kernel frame (3 items): pop rax(add rsp,8) popfq jmp rax
    //   RPL=3 → user frame (5 items): fix SS, iretq
    "context_switch_yield:",
    "  cli",
    // Reserve 16 bytes so the 3-item frame (24 bytes at [RSP+120..143])
    // does NOT overwrite the caller's stack frame above the return address.
    // Net: after restore, RSP = caller's RSP (no +16 displacement).
    "  sub rsp, 16",
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
    // Stack: [R15..RAX][16 reserved][return_addr]
    // return_addr is at [rsp + 136] (15 items × 8 + 16 reserved)
    // Read return_addr into rax (RAX is safe — saved at [rsp+112])
    "  mov rax, [rsp + 136]",
    // Write 3-item interrupt frame ABOVE the 15 regs,
    // overwriting return_addr and two caller-slots (no longer needed)
    // Layout: [rsp+120]=RIP, [rsp+128]=CS, [rsp+136]=RFLAGS
    "  mov [rsp + 120], rax",
    "  mov qword ptr [rsp + 128], 0x08",
    "  pushfq",
    "  pop rax",
    "  mov [rsp + 136], rax",
    "  or qword ptr [rsp + 136], 0x200",
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
    // Frame type detection via CS.RPL
    // [rsp] = RIP, [rsp+8] = CS, [rsp+16] = RFLAGS
    // For user: [rsp+24] = user_RSP, [rsp+32] = SS
    // NOTE: use memory TEST to avoid corrupting restored RCX
    "  test byte ptr [rsp + 8], 3",
    "  jnz 3f",
    // Kernel task (3-item frame: RIP, CS, RFLAGS)
    "  pop rax",
    "  add rsp, 8",
    "  popfq",
    "  jmp rax",
    // User task (5-item frame: RIP, CS, RFLAGS, RSP, SS)
    // KERNEL_GS_BASE was set to PerCpu by Rust caller.
    // Zero GS_BASE so user mode can't access kernel memory via GS segment.
    "3:",
    "  xor eax, eax",
    "  xor edx, edx",
    "  mov ecx, 0xC0000101",
    "  wrmsr",
    "  mov qword ptr [rsp + 32], 0x1b",
    "  iretq",
    ".att_syntax prefix",
);

core::arch::global_asm!(
    ".intel_syntax noprefix",
    ".globl apic_timer_isr_stub",
    "apic_timer_isr_stub:",
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
    "  mov rdi, rsp",
    "  call schedule_tick",
    "  test rax, rax",
    "  jz timer_no_switch",
    "  mov rsp, rax",
    "timer_no_switch:",
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
    "  test byte ptr [rsp + 8], 3",
    "  jnz timer_user_ret",
    // Kernel return — use pop+popf+jmp to avoid iretq's 5-item frame
    "  pop rax",
    "  add rsp, 8",
    "  popfq",
    "  jmp rax",
    "timer_user_ret:",
    "  xor eax, eax",
    "  xor edx, edx",
    "  mov ecx, 0xC0000101",
    "  wrmsr",
    "  mov qword ptr [rsp + 32], 0x1b",
    "  iretq",
    ".att_syntax prefix",
);

pub fn init() {
    let mut tasks = TASKS.lock();

    // Idle task: allocate a 16K kernel stack and construct a 3-item kernel frame
    // so the scheduler can switch to idle() with a valid RSP.
    let (idle_stack_base, _idle_layout) = unsafe { alloc_stack(16384) };
    let idle_stack_top = idle_stack_base.wrapping_add(16384);
    let mut idle_sp = idle_stack_top as *mut u64;
    unsafe {
        idle_sp = idle_sp.sub(1);
        idle_sp.write(0x202u64);                   // RFLAGS (IF set)
        idle_sp = idle_sp.sub(1);
        idle_sp.write(0x08u64);                    // CS (kernel)
        idle_sp = idle_sp.sub(1);
        idle_sp.write(idle as *const () as usize as u64);
        for _ in 0..15 {
            idle_sp = idle_sp.sub(1);
            idle_sp.write(0u64);                   // zeroed GP registers
        }
        // Zero the rest of the stack to prevent stale data from being
        // misinterpreted as interrupt frames or return addresses.
        let clear_start = idle_stack_base as *mut u64;
        let clear_end = idle_sp;
        let mut clear_ptr = clear_end;
        while clear_ptr > clear_start {
            clear_ptr = clear_ptr.sub(1);
            clear_ptr.write(0u64);
        }
    }
    let idle_initial_rsp = idle_sp as u64;

    let mut idle_task = Task::new(0, idle_initial_rsp);
    idle_task.rsp = idle_initial_rsp;
    idle_task.stack_alloc = idle_stack_base;
    idle_task.stack_size = 16384;
    tasks.tasks[0] = Some(idle_task);
    IDLE_RSP.store(idle_stack_top, Ordering::Release);
    TASK_COUNT.store(1, Ordering::Release);

    let s = SerialPort::new(0x3F8);
    s.write_str("[OK] Scheduler initialized\n");
}

fn current_cpu() -> u32 {
    zenus_arch::smp::current_cpu()
}

fn current_cpu_id() -> u32 {
    let cpu = current_cpu() as usize % MAX_CPUS;
    CURRENT_TASK[cpu].load(Ordering::Acquire)
}

fn set_current_cpu_id(cpu: u32, idx: u32) {
    CURRENT_TASK[cpu as usize % MAX_CPUS].store(idx, Ordering::Release);
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

    unsafe {
        let mut sp = stack_top as *mut u64;
        sp = sp.sub(1); sp.write(0x1bu64);
        sp = sp.sub(1); sp.write(user_rsp);
        sp = sp.sub(1); sp.write(0x202u64);
        sp = sp.sub(1); sp.write(0x23u64);
        sp = sp.sub(1); sp.write(entry);
        for _ in 0..15 {
            sp = sp.sub(1); sp.write(0u64);
        }
        let initial_rsp = sp as u64;

        let mut task = Task::new(id, initial_rsp);
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.kernel_rsp_top = stack_top;
        task.user_rsp = user_rsp;
        task.cpu = cpu;
        task.cr3 = cr3;
        task.heap_brk = heap_brk;
        task.uid = parent.uid;
        task.gid = parent.gid;
        task.euid = parent.euid;
        task.egid = parent.egid;
        task.uts_ns = new_uts_ns;
        task.pid_ns = new_pid_ns;
        task.mnt_ns = new_mnt_ns;

        let mut tasks = TASKS.lock();
        let mut placed = false;
        for i in 0..MAX_TASKS {
            if tasks.tasks[i].is_none() {
                tasks.tasks[i] = Some(task);
                placed = true;
                TASK_COUNT.fetch_add(1, Ordering::Release);
                break;
            }
        }
        if !placed {
            dealloc_stack(stack_base, stack_size);
            return 0;
        }
    }

    // Register in PID namespace if it's a new or existing non-root NS
    if new_pid_ns != NS_ROOT {
        zenus_ns::pid::register_task(new_pid_ns, id);
    }

    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    id
}

pub fn create_user_task(entry: u64, stack_size: usize, user_rsp: u64, cr3: u64, heap_base: u64) -> u64 {
    // Validate entry point: must be a canonical user-space address.
    // Entry values in the 1-16MB range likely indicate a physical address
    // was accidentally passed as the virtual entry point.
    if entry < 0x1000 || entry >= 0x0000_8000_0000_0000 {
        return 0;
    }
    let (stack_base, _stack_layout) = unsafe { alloc_stack(stack_size) };
    if stack_base == 0 {
        return 0;
    }
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::SeqCst);
    let stack_top = stack_base + stack_size as u64;
    let cpu = 0u32;

    let aslr_user_rsp = if user_rsp == 0 {
        let slide = zenus_arch::random::get_random_page_aligned(0, 0x2000_0000u64);
        let rsp = 0x7FFF_FFFF_F000u64.saturating_sub(slide);
        if rsp < 0x1000 { 0x7FFF_FFFF_F000u64 } else { rsp }
    } else {
        user_rsp
    };

    let heap_brk = if heap_base != 0 {
        heap_base
    } else {
        zenus_arch::random::get_random_page_aligned(
            0x6000_0000_0000u64, 0x6000_0020_0000u64,
        )
    };

    unsafe {
        let mut sp = stack_top as *mut u64;
        sp = sp.sub(1); sp.write(0x1bu64);
        sp = sp.sub(1); sp.write(aslr_user_rsp);
        sp = sp.sub(1); sp.write(0x202u64);
        sp = sp.sub(1); sp.write(0x23u64);
        sp = sp.sub(1); sp.write(entry);
        for _ in 0..15 {
            sp = sp.sub(1); sp.write(0u64);
        }
        let initial_rsp = sp as u64;

        let mut task = Task::new(id, initial_rsp);
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.kernel_rsp_top = stack_top;
        task.user_rsp = aslr_user_rsp;
        task.cpu = cpu;
        task.cr3 = cr3;
        task.heap_brk = heap_brk;

        let mut tasks = TASKS.lock();
        let mut placed = false;
        for i in 0..MAX_TASKS {
            if tasks.tasks[i].is_none() {
                tasks.tasks[i] = Some(task);
                placed = true;
                TASK_COUNT.fetch_add(1, Ordering::Release);
                break;
            }
        }
        if !placed {
            dealloc_stack(stack_base, stack_size);
            return 0;
        }
    }

    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    id
}

pub fn create_task(entry: fn(), stack_size: usize) -> u64 {
    let (stack_base, _stack_layout) = unsafe { alloc_stack(stack_size) };
    if stack_base == 0 {
        return 0;
    }
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::SeqCst);
    let stack_top = stack_base + stack_size as u64;

    let cpu = 0u32;

    unsafe {
        let mut sp = stack_top as *mut u64;
        // 3-item kernel frame: RFLAGS, CS, RIP
        sp = sp.sub(1); sp.write(0x202u64);
        sp = sp.sub(1); sp.write(0x08u64);
        sp = sp.sub(1); sp.write(entry as u64);
        for _ in 0..15 {
            sp = sp.sub(1); sp.write(0u64);
        }
        let initial_rsp = sp as u64;

        let mut task = Task::new(id, initial_rsp);
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.kernel_rsp_top = stack_top;
        task.cpu = cpu;

        let mut tasks = TASKS.lock();
        let mut placed = false;
        for i in 0..MAX_TASKS {
            if tasks.tasks[i].is_none() {
                tasks.tasks[i] = Some(task);
                placed = true;
                TASK_COUNT.fetch_add(1, Ordering::Release);
                break;
            }
        }
        if !placed {
            dealloc_stack(stack_base, stack_size);
            return 0;
        }
    }

    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    id
}

unsafe fn dealloc_stack(base: u64, size: usize) {
    if base == 0 || size == 0 { return; }
    let Ok(layout) = core::alloc::Layout::from_size_align(size, 16) else { return; };
    alloc::alloc::dealloc(base as *mut u8, layout);
}

unsafe fn alloc_stack(size: usize) -> (u64, core::alloc::Layout) {
    use core::alloc::Layout;
    let Ok(layout) = Layout::from_size_align(size, 16) else { return (0, Layout::new::<u8>()); };
    let ptr = {
        use core::alloc::GlobalAlloc;
        ALLOCATOR.alloc(layout)
    };
    if ptr.is_null() {
        return (0, layout);
    }
    (ptr as u64, layout)
}

pub fn yield_now() {
    let cpu = current_cpu();
    if (cpu as usize) >= MAX_CPUS {
        x86_64::instructions::hlt();
        return;
    }
    let count = TASK_COUNT.load(Ordering::Acquire);
    if count <= 1 {
        x86_64::instructions::hlt();
        return;
    }

    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = TASKS.lock();

    if tasks.tasks[current as usize].is_none() {
        drop(tasks);
        return;
    }

    let next = find_next_ready(&tasks, current, cpu);
    if next == current {
        drop(tasks);
        x86_64::instructions::hlt();
        return;
    }

    if tasks.tasks[next as usize].is_none() {
        drop(tasks);
        return;
    }

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
        current_task.state = TaskState::Ready;
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

    let next_kernel_rsp = tasks.tasks[next as usize].as_ref()
        .map(|t| t.kernel_rsp_top).unwrap_or(0);
    let next_user_rsp = tasks.tasks[next as usize].as_ref()
        .map(|t| t.user_rsp).unwrap_or(0);
    let save_rsp: *mut u64 = unsafe {
        &raw mut tasks.tasks[current as usize].as_mut().unwrap_unchecked().rsp
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

    unsafe { context_switch_yield(save_rsp, next_rsp); }
}

pub fn check_yield() {
    yield_now();
}

fn find_next_ready(tasks: &TaskArray, current: u32, cpu: u32) -> u32 {
    for i in 1..MAX_TASKS as u32 {
        let idx = (current + i) % MAX_TASKS as u32;
        if let Some(ref task) = tasks.tasks[idx as usize] {
            if task.is_active() && task.cpu == cpu { return idx; }
        }
    }
    current
}

pub fn current_task_id() -> u64 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize].as_ref().map(|t| t.id).unwrap_or(0)
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
    tasks.tasks[idx as usize].as_ref().map(|t| t.pid_ns).unwrap_or(0)
}

/// Get the mount namespace of the current task.
pub fn current_mnt_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize].as_ref().map(|t| t.mnt_ns).unwrap_or(0)
}

/// Get the UTS namespace of the current task.
pub fn current_uts_ns() -> NsId {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize].as_ref().map(|t| t.uts_ns).unwrap_or(0)
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
    tasks.tasks[idx as usize].as_ref().map(|t| t.uid).unwrap_or(0)
}

pub fn current_gid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize].as_ref().map(|t| t.gid).unwrap_or(0)
}

pub fn current_euid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize].as_ref().map(|t| t.euid).unwrap_or(0)
}

pub fn current_egid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = TASKS.lock();
    tasks.tasks[idx as usize].as_ref().map(|t| t.egid).unwrap_or(0)
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

static TICK_COUNT: AtomicU64 = AtomicU64::new(0);

/// Called from APIC timer ISR. Interrupts disabled (interrupt gate).
/// Sends EOI, tries to acquire TASKS lock (returns 0 if contended),
/// saves current RSP, finds next task on this CPU, returns its RSP.
#[no_mangle]
pub extern "C" fn schedule_tick(current_rsp: u64) -> u64 {
    extern "C" { fn apic_timer_eoi(); }
    unsafe { apic_timer_eoi(); }

    let cpu = current_cpu();
    if (cpu as usize) >= MAX_CPUS {
        return 0;
    }

    TICK_COUNT.fetch_add(1, Ordering::Relaxed);

    zenus_arch::watchdog::watchdog_tick();
    zenus_arch::watchdog::watchdog_pet();

    // Guard: if running on the boot stack (not yet in the idle task),
    // skip context switching. This prevents the timer from trying to
    // save/restore the idle task's RSP while it's still on the boot stack.
    if cpu == 0 {
        let current = CURRENT_TASK[0].load(Ordering::Relaxed);
        if current == 0 {
            let tasks = TASKS.lock();
            if let Some(ref idle_task) = tasks.tasks[0] {
                let base = idle_task.stack_alloc;
                let size = idle_task.stack_size;
                if size > 0 && (current_rsp < base || current_rsp >= base.wrapping_add(size as u64)) {
                    drop(tasks);
                    return 0;
                }
            }
            drop(tasks);
        }
    }

    let count = TASK_COUNT.load(Ordering::Acquire);
    if count <= 1 {
        return 0;
    }

    let current = CURRENT_TASK[cpu as usize].load(Ordering::Relaxed);
    let mut tasks = match TASKS.try_lock() {
        Some(t) => t,
        None => return 0,
    };

    if tasks.tasks[current as usize].is_none() {
        return 0;
    }

    let expired = if let Some(ref mut task) = tasks.tasks[current as usize] {
        task.ticks_left = task.ticks_left.saturating_sub(1);
        task.ticks_left == 0
    } else {
        false
    };

    if !expired {
        return 0;
    }

    let current_cr3_raw = zenus_mem::paging::get_level4_addr().as_u64();
    if let Some(current_task) = tasks.tasks[current as usize].as_mut() {
        current_task.rsp = current_rsp;
        current_task.cr3 = current_cr3_raw;
        let cs_on_stack = unsafe { core::ptr::read_volatile((current_rsp + 128) as *const u64) };
        if (cs_on_stack & 3) == 3 {
            current_task.user_rsp = unsafe { core::ptr::read_volatile((current_rsp + 144) as *const u64) };
        } else {
            current_task.user_rsp = zenus_arch::cpu::get_percpu_user_rsp(cpu);
        }
    } else {
        return 0;
    }

    let next = find_next_ready(&tasks, current, cpu);
    if next == current {
        return 0;
    }

    if tasks.tasks[next as usize].is_none() {
        return 0;
    }

    if let Some(current_task) = tasks.tasks[current as usize].as_mut() {
        current_task.state = TaskState::Ready;
        current_task.cr3 = current_cr3_raw;
    } else {
        return 0;
    }

    let new_rsp = match tasks.tasks[next as usize].as_mut() {
        Some(nt) => {
            nt.state = TaskState::Running;
            nt.ticks_left = TIME_SLICE;

            if nt.user_rsp != 0 {
                zenus_arch::cpu::set_percpu_user_rsp(cpu, nt.user_rsp);
            } else {
                zenus_arch::cpu::set_percpu_user_rsp(cpu, 0);
            }

            let next_cr3 = nt.cr3;
            if next_cr3 != 0 && next_cr3 != current_cr3_raw {
                zenus_mem::paging::set_cr3(next_cr3);
            }

            CURRENT_TASK[cpu as usize].store(next, Ordering::Release);

            if nt.kernel_rsp_top != 0 {
                zenus_arch::cpu::set_percpu_kernel_rsp(cpu, nt.kernel_rsp_top);
            }

            if nt.kernel_rsp_top != 0 {
                zenus_arch::gdt::set_tss_stack(x86_64::VirtAddr::new(nt.kernel_rsp_top));
            }

            nt.rsp
        }
        None => return 0,
    };

    drop(tasks);
    new_rsp
}

core::arch::global_asm!(
    ".section .text._sched_idle_asm, \"ax\"",
    ".globl _sched_idle_asm",
    ".type _sched_idle_asm, @function",
    "_sched_idle_asm:",
    "cli",
    "mov rax, qword ptr [rip + {idle}]",
    "mov rsp, rax",
    "sti",
    "hlt",
    "0:",
    "sti",
    "hlt",
    "jmp 0b",
    idle = sym IDLE_RSP,
);

extern "C" {
    fn _sched_idle_asm() -> !;
}

/// Opaque reference that ThinLTO cannot devirtualize.
/// Stored as a raw pointer so the compiler sees an indirect call.
struct SyncPtr(*const ());
unsafe impl Sync for SyncPtr {}

#[used]
#[link_section = ".text._sched_idle_ref"]
static SCHED_IDLE_KEEP: SyncPtr = SyncPtr(_sched_idle_asm as *const () as *const ());

pub fn idle() -> ! {
    let p = unsafe { core::ptr::read_volatile(&SCHED_IDLE_KEEP).0 };
    let f: unsafe extern "C" fn() -> ! = unsafe { core::mem::transmute(p) };
    unsafe { f() }
}

pub fn ap_idle() -> ! {
    loop {
        unsafe { core::arch::asm!("sti", "hlt", options(nostack)); }
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
                unsafe { alloc::alloc::dealloc(ts.base as *mut u8, layout); }
            }
        }
    }
    list.count = 0;
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
    tasks.tasks[current as usize] = None;
    let mut new_count = 1u32;
    for scan in (1..MAX_TASKS).rev() {
        if tasks.tasks[scan].is_some() {
            new_count = (scan + 1) as u32;
            break;
        }
    }
    TASK_COUNT.store(new_count, Ordering::Release);
    drop(tasks);
    loop { x86_64::instructions::hlt(); }
}

pub fn kill_task(id: u64) -> bool {
    if id == 0 { return false; }
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
                Some(tc) if tc.id == id && tc.is_active() => Some((tc.cpu, tc.stack_alloc, tc.stack_size, tc.cr3, tc.state, tc.pid_ns, tc.id)),
                _ => None,
            }
        };
        if let Some((task_cpu, stack_alloc, stack_size, cr3, state, pid_ns, tid)) = task_info {
            if pid_ns != 0 {
                zenus_ns::pid::unregister_task(pid_ns, tid);
            }
            if state == TaskState::Running {
                tasks.tasks[i] = None;
                CPU_TASK_COUNT[task_cpu as usize].fetch_sub(1, Ordering::SeqCst);
                if stack_alloc != 0 && stack_size > 0 {
                    unsafe {
                        if let Ok(layout) = core::alloc::Layout::from_size_align(
                            stack_size as usize, 16,
                        ) {
                            alloc::alloc::dealloc(stack_alloc as *mut u8, layout);
                        }
                    }
                }
                if cr3 != 0 {
                    zenus_mem::paging::destroy_address_space(cr3);
                }
                let mut new_count = 1u32;
                for scan in (1..MAX_TASKS).rev() {
                    if tasks.tasks[scan].is_some() {
                        new_count = (scan + 1) as u32;
                        break;
                    }
                }
                TASK_COUNT.store(new_count, Ordering::Release);
                return true;
            }
            tasks.tasks[i] = None;
            CPU_TASK_COUNT[task_cpu as usize].fetch_sub(1, Ordering::SeqCst);
            if stack_alloc != 0 && stack_size > 0 {
                unsafe {
                    if let Ok(layout) = core::alloc::Layout::from_size_align(
                        stack_size as usize, 16,
                    ) {
                        alloc::alloc::dealloc(stack_alloc as *mut u8, layout);
                    }
                }
            }
            // Free the task's address space (user pages, page tables)
            if cr3 != 0 {
                zenus_mem::paging::destroy_address_space(cr3);
            }
            let mut new_count = 1u32;
            for scan in (1..MAX_TASKS).rev() {
                if tasks.tasks[scan].is_some() {
                    new_count = (scan + 1) as u32;
                    break;
                }
            }
            TASK_COUNT.store(new_count, Ordering::Release);
            return true;
        }
    }
    false
}
