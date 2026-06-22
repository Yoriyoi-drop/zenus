use core::sync::atomic::{AtomicU64, AtomicU32, Ordering};
use zenus_console::serial::SerialPort;
use zenus_sync::spinlock::SpinLock;
use super::task::{Task, TaskInfo, TaskState, MAX_TASKS};
use zenus_mem::allocator::ALLOCATOR;

const TIME_SLICE: u64 = 50;
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

#[allow(static_mut_refs)]
static mut TASKS: SpinLock<TaskArray> = SpinLock::new(TaskArray::new());

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
    "context_switch_yield:",
    "  cli",
    "  pop rax",
    "  pushfq",
    "  push 0x08",
    "  push rax",
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
    "  or qword ptr [rsp + 136], 0x200",
    "  mov rax, rsp",
    "  add rax, 144",
    "  mov [rdi], rsp",
    "  mov rsp, rsi",
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
    "  pop rax",
    "  add rsp, 8",
    "  popfq",
    "  jmp rax",
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
    "  jz 1f",
    "  mov rsp, rax",
    "1:",
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
  "  jnz 3f",
  "  pop rax",
  "  add rsp, 8",
  "  popfq",
  "  jmp rax",
    "3:",
    "  mov qword ptr [rsp + 32], 0x1b",
    "  iretq",
    ".att_syntax prefix",
);

pub fn init() {
    let mut tasks = unsafe { TASKS.lock() };
    let idle = Task::new(0, 0);
    tasks.tasks[0] = Some(idle);
    TASK_COUNT.store(1, Ordering::Release);

    let mut s = SerialPort::new(0x3F8);
    s.write_str("[OK] Scheduler initialized\n");
}

fn current_cpu() -> u32 {
    zenus_arch::smp::current_cpu()
}

fn current_cpu_id() -> u32 {
    let cpu = current_cpu();
    CURRENT_TASK[cpu as usize].load(Ordering::Acquire)
}

fn set_current_cpu_id(cpu: u32, idx: u32) {
    CURRENT_TASK[cpu as usize].store(idx, Ordering::Release);
}

macro_rules! wb {
    ($b:expr) => {
        #[allow(unused_unsafe)]
        unsafe { core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") $b) }
    };
}

pub fn create_user_task(entry: u64, stack_size: usize, user_rsp: u64, cr3: u64) -> u64 {
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

    let heap_brk = zenus_arch::random::get_random_page_aligned(
        0x6000_0000_0000u64, 0x6000_0020_0000u64,
    );

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
        task.cpu = cpu;
        task.cr3 = cr3;
        task.heap_brk = heap_brk;

        let mut tasks = TASKS.lock();
        let mut placed = false;
        for i in 0..MAX_TASKS {
            if tasks.tasks[i].is_none() {
                tasks.tasks[i] = Some(task);
                placed = true;
                let new_count = (i + 1) as u32;
                if new_count > TASK_COUNT.load(Ordering::Relaxed) {
                    TASK_COUNT.store(new_count, Ordering::Release);
                }
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
    wb!(b'A');
    let (stack_base, _stack_layout) = unsafe { alloc_stack(stack_size) };
    if stack_base == 0 {
        return 0;
    }
    let id = NEXT_TASK_ID.fetch_add(1, Ordering::SeqCst);
    wb!(b'B');
    let stack_top = stack_base + stack_size as u64;
    wb!(b'C');

    let cpu = 0u32;
    wb!(b'D');

    unsafe {
        let mut sp = stack_top as *mut u64;
        sp = sp.sub(1); sp.write(0x202u64);
        sp = sp.sub(1); sp.write(0x08u64);
        sp = sp.sub(1); sp.write(entry as u64);
        for _ in 0..15 {
            sp = sp.sub(1); sp.write(0u64);
        }
        let initial_rsp = sp as u64;
        wb!(b'E');

        let mut task = Task::new(id, initial_rsp);
        wb!(b'F');
        task.rsp = initial_rsp;
        task.stack_alloc = stack_base;
        task.stack_size = stack_size as u64;
        task.cpu = cpu;
        wb!(b'G');

        let mut tasks = TASKS.lock();
        wb!(b'H');
        let mut placed = false;
        for i in 0..MAX_TASKS {
            if tasks.tasks[i].is_none() {
                tasks.tasks[i] = Some(task);
                placed = true;
                let new_count = (i + 1) as u32;
                if new_count > TASK_COUNT.load(Ordering::Relaxed) {
                    TASK_COUNT.store(new_count, Ordering::Release);
                }
                break;
            }
        }
        if !placed {
            dealloc_stack(stack_base, stack_size);
            return 0;
        }
    }
    wb!(b'I');

    wb!(b'J');
    CPU_TASK_COUNT[cpu as usize].fetch_add(1, Ordering::SeqCst);
    wb!(b'K');
    id
}

unsafe fn dealloc_stack(base: u64, size: usize) {
    if base == 0 { return; }
    let layout = core::alloc::Layout::from_size_align(size, 16).unwrap();
    alloc::alloc::dealloc(base as *mut u8, layout);
}

unsafe fn alloc_stack(size: usize) -> (u64, core::alloc::Layout) {
    use core::alloc::Layout;
    let layout = Layout::from_size_align(size, 16).unwrap();
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
    let mut tasks = unsafe { TASKS.lock() };

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
            if rsp < stack_bottom || rsp > stack_top {
                drop(tasks);
                return;
            }
        }
    }

    tasks.tasks[current as usize].as_mut().unwrap().state = TaskState::Ready;
    tasks.tasks[next as usize].as_mut().unwrap().state = TaskState::Running;
    tasks.tasks[next as usize].as_mut().unwrap().ticks_left = TIME_SLICE;

    CURRENT_TASK[cpu as usize].store(next, Ordering::Release);

    let save_rsp: *mut u64 = unsafe {
        &raw mut tasks.tasks[current as usize].as_mut().unwrap_unchecked().rsp
    };
    let new_rsp = tasks.tasks[next as usize].as_ref().unwrap().rsp;
    drop(tasks);

    unsafe { context_switch_yield(save_rsp, new_rsp); }
}

pub fn check_yield() {
    yield_now();
}

fn find_next_ready(tasks: &TaskArray, current: u32, cpu: u32) -> u32 {
    let count = (TASK_COUNT.load(Ordering::Acquire) as usize).min(MAX_TASKS) as u32;
    if count == 0 { return 0; }
    for i in 1..count {
        let idx = (current + i) % count;
        if let Some(ref task) = tasks.tasks[idx as usize] {
            if task.is_active() && task.cpu == cpu { return idx; }
        }
    }
    current
}

pub fn current_task_id() -> u64 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = unsafe { TASKS.lock() };
    tasks.tasks[idx as usize].as_ref().map(|t| t.id).unwrap_or(0)
}

pub fn list_tasks() -> [Option<TaskInfo>; MAX_TASKS] {
    let tasks = unsafe { TASKS.lock() };
    let mut result: [Option<TaskInfo>; MAX_TASKS] = [None; MAX_TASKS];
    for (i, t) in tasks.tasks.iter().enumerate() {
        if let Some(ref task) = t {
            result[i] = Some(TaskInfo { id: task.id, state: task.state, cpu: task.cpu, uid: task.uid, gid: task.gid });
        }
    }
    result
}

pub fn current_uid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = unsafe { TASKS.lock() };
    tasks.tasks[idx as usize].as_ref().map(|t| t.uid).unwrap_or(0)
}

pub fn current_gid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = unsafe { TASKS.lock() };
    tasks.tasks[idx as usize].as_ref().map(|t| t.gid).unwrap_or(0)
}

pub fn current_euid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = unsafe { TASKS.lock() };
    tasks.tasks[idx as usize].as_ref().map(|t| t.euid).unwrap_or(0)
}

pub fn current_egid() -> u32 {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let tasks = unsafe { TASKS.lock() };
    tasks.tasks[idx as usize].as_ref().map(|t| t.egid).unwrap_or(0)
}

pub fn set_current_uid(uid: u32) -> bool {
    let cpu = current_cpu();
    let idx = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = unsafe { TASKS.lock() };
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
    let mut tasks = unsafe { TASKS.lock() };
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
    let tasks = unsafe { TASKS.lock() };
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
    let mut tasks = unsafe { TASKS.lock() };
    for t in tasks.tasks.iter_mut() {
        if let Some(ref mut task) = t {
            if task.id == id {
                task.heap_brk = brk;
                return;
            }
        }
    }
}

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
    let count = TASK_COUNT.load(Ordering::Acquire);
    if count <= 1 {
        return 0;
    }

    let current = CURRENT_TASK[cpu as usize].load(Ordering::Relaxed);
    let mut tasks = match unsafe { TASKS.try_lock() } {
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

    tasks.tasks[current as usize].as_mut().unwrap().rsp = current_rsp;
    tasks.tasks[current as usize].as_mut().unwrap().cr3 = zenus_mem::paging::get_level4_addr().as_u64();

    let next = find_next_ready(&tasks, current, cpu);
    if next == current {
        return 0;
    }

    if tasks.tasks[next as usize].is_none() {
        return 0;
    }

    if let Some(ref next_task) = tasks.tasks[next as usize] {
        if next_task.stack_alloc != 0 && next_task.stack_size > 0 {
            let rsp = next_task.rsp;
            let stack_bottom = next_task.stack_alloc;
            let stack_top = stack_bottom + next_task.stack_size as u64;
            if rsp < stack_bottom || rsp > stack_top {
                return 0;
            }
        }
    }

    tasks.tasks[current as usize].as_mut().unwrap().state = TaskState::Ready;
    tasks.tasks[next as usize].as_mut().unwrap().state = TaskState::Running;
    tasks.tasks[next as usize].as_mut().unwrap().ticks_left = TIME_SLICE;

    let current_cr3_raw = zenus_mem::paging::get_level4_addr().as_u64();
    tasks.tasks[current as usize].as_mut().unwrap().cr3 = current_cr3_raw;

    let next_cr3 = tasks.tasks[next as usize].as_ref().unwrap().cr3;
    if next_cr3 != 0 && next_cr3 != current_cr3_raw {
        zenus_mem::paging::set_cr3(next_cr3);
    }

    CURRENT_TASK[cpu as usize].store(next, Ordering::Release);

    let new_rsp = tasks.tasks[next as usize].as_ref().unwrap().rsp;
    drop(tasks);

    new_rsp
}

pub fn idle() -> ! {
    loop {
        x86_64::instructions::interrupts::enable();
        x86_64::instructions::hlt();
        x86_64::instructions::interrupts::disable();
        yield_now();
    }
}

pub fn ap_idle() -> ! {
    loop {
        x86_64::instructions::hlt();
        yield_now();
    }
}

pub struct TerminatedStack {
    pub base: u64,
    pub size: usize,
}

static mut TERMINATED_STACKS: [Option<TerminatedStack>; 8] = [None; 8];
static mut TERMINATED_STACK_COUNT: usize = 0;

pub fn reap_terminated_stacks() {
    unsafe {
        for i in 0..TERMINATED_STACK_COUNT {
            if let Some(ts) = TERMINATED_STACKS[i] {
                let layout = core::alloc::Layout::from_size_align(ts.size, 16).unwrap();
                alloc::alloc::dealloc(ts.base as *mut u8, layout);
            }
        }
        TERMINATED_STACK_COUNT = 0;
    }
}

pub fn task_exit() {
    let cpu = current_cpu();
    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = unsafe { TASKS.lock() };
    if let Some(ref task) = tasks.tasks[current as usize] {
        // Save stack for reaping (can't free while still running on it)
        if task.stack_alloc != 0 && task.stack_size > 0 {
            unsafe {
                if TERMINATED_STACK_COUNT < 8 {
                    TERMINATED_STACKS[TERMINATED_STACK_COUNT] = Some(TerminatedStack {
                        base: task.stack_alloc,
                        size: task.stack_size as usize,
                    });
                    TERMINATED_STACK_COUNT += 1;
                }
            }
        }
        CPU_TASK_COUNT[task.cpu as usize].fetch_sub(1, Ordering::SeqCst);
        tasks.tasks[current as usize] = None;
    }
    drop(tasks);
    loop { x86_64::instructions::hlt(); }
}

pub fn kill_task(id: u64) -> bool {
    if id == 0 { return false; }
    let cpu = current_cpu();
    let current = CURRENT_TASK[cpu as usize].load(Ordering::Acquire);
    let mut tasks = unsafe { TASKS.lock() };

    for i in 0..MAX_TASKS {
        if let Some(ref task) = tasks.tasks[i] {
            if task.id == id && i as u32 == current {
                return false;
            }
        }
    }

    for i in 0..MAX_TASKS {
        if let Some(ref mut task) = tasks.tasks[i] {
            if task.id == id && task.is_active() {
                if task.state == TaskState::Running {
                    task.state = TaskState::Terminated;
                    tasks.tasks[i] = None;
                    CPU_TASK_COUNT[task.cpu as usize].fetch_sub(1, Ordering::SeqCst);
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
                if task.stack_alloc != 0 && task.stack_size > 0 {
                    unsafe {
                        let layout = core::alloc::Layout::from_size_align(
                            task.stack_size as usize, 16,
                        ).unwrap();
                        alloc::alloc::dealloc(task.stack_alloc as *mut u8, layout);
                    }
                }
                CPU_TASK_COUNT[task.cpu as usize].fetch_sub(1, Ordering::SeqCst);
                task.stack_alloc = 0;
                task.stack_size = 0;
                task.state = TaskState::Terminated;
                tasks.tasks[i] = None;
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
    }
    false
}
