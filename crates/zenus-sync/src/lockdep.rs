use core::sync::atomic::{AtomicBool, Ordering};
use crate::spinlock::SpinLock;

fn lockdep_serial(msg: &str) {
    for &b in msg.as_bytes() {
        unsafe {
            loop {
                let mut lsr: u8;
                core::arch::asm!("in al, dx", out("al") lsr, in("dx") 0x3FDu16, options(nostack, preserves_flags));
                if lsr & 0x20 != 0 { break; }
            }
            core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nostack, preserves_flags));
        }
    }
}

const MAX_LOCKS: usize = 64;
const MAX_DEPTH: usize = 8;
const MAX_EDGES: usize = 256;
const MAX_CPUS: usize = 8;

#[derive(Clone, Copy)]
struct LockClass {
    name: &'static str,
    registered: bool,
}

#[derive(Clone, Copy)]
struct LockEdge {
    from: usize,
    to: usize,
}

struct LockdepState {
    classes: [LockClass; MAX_LOCKS],
    class_count: usize,
    edges: [LockEdge; MAX_EDGES],
    edge_count: usize,
    per_cpu_stack: [[usize; MAX_DEPTH]; MAX_CPUS],
    per_cpu_depth: [usize; MAX_CPUS],
    violations: u64,
}

impl LockdepState {
    const fn new() -> Self {
        const EMPTY_CLASS: LockClass = LockClass { name: "", registered: false };
        const EMPTY_EDGE: LockEdge = LockEdge { from: 0, to: 0 };
        LockdepState {
            classes: [EMPTY_CLASS; MAX_LOCKS],
            class_count: 0,
            edges: [EMPTY_EDGE; MAX_EDGES],
            edge_count: 0,
            per_cpu_stack: [[0; MAX_DEPTH]; MAX_CPUS],
            per_cpu_depth: [0; MAX_CPUS],
            violations: 0,
        }
    }
}

static LOCKDEP: SpinLock<LockdepState> = SpinLock::new(LockdepState::new());
static LOCKDEP_INIT: AtomicBool = AtomicBool::new(false);
static LOCKDEP_ENABLED: AtomicBool = AtomicBool::new(true);

fn current_cpu() -> usize {
    let cpu: u64;
    unsafe { core::arch::asm!("mov {}, cr8", out(reg) cpu); }
    cpu as usize % MAX_CPUS
}

pub fn lockdep_init() {
    LOCKDEP_INIT.store(true, Ordering::Release);
}

pub fn lockdep_register(name: &'static str) -> usize {
    if !LOCKDEP_INIT.load(Ordering::Acquire) || !LOCKDEP_ENABLED.load(Ordering::Acquire) {
        return 0;
    }
    let mut state = LOCKDEP.lock();
    for i in 0..state.class_count {
        if state.classes[i].name == name {
            return i;
        }
    }
    if state.class_count >= MAX_LOCKS {
        return 0;
    }
    let id = state.class_count;
    state.classes[id] = LockClass { name, registered: true };
    state.class_count += 1;
    id
}

pub fn lockdep_acquire(lock_id: usize, caller: &'static str) -> bool {
    if !LOCKDEP_INIT.load(Ordering::Acquire) || !LOCKDEP_ENABLED.load(Ordering::Acquire) {
        return true;
    }
    if lock_id == 0 || lock_id >= MAX_LOCKS {
        return true;
    }
    let cpu = current_cpu();
    let mut state = LOCKDEP.lock();

    let depth = state.per_cpu_depth[cpu];
    for i in 0..depth {
        let held = state.per_cpu_stack[cpu][i];
        if held == lock_id {
            return true;
        }
        let ec = state.edge_count;
        let already_recorded = state.edges[..ec]
            .iter()
            .any(|e| e.from == held && e.to == lock_id);
        if !already_recorded && ec < MAX_EDGES {
            state.edges[ec] = LockEdge { from: held, to: lock_id };
            state.edge_count = ec + 1;
        }

        let ec2 = state.edge_count;
        let reverse = state.edges[..ec2]
            .iter()
            .any(|e| e.from == lock_id && e.to == held);
        if reverse {
            state.violations += 1;
            lockdep_serial("[LOCKDEP] Potential deadlock: ");
            lockdep_serial(state.classes[lock_id].name);
            lockdep_serial(" -> ");
            lockdep_serial(state.classes[held].name);
            lockdep_serial(" (caller: ");
            lockdep_serial(caller);
            lockdep_serial(")\n");
            return false;
        }
    }

    if depth < MAX_DEPTH {
        state.per_cpu_stack[cpu][depth] = lock_id;
        state.per_cpu_depth[cpu] = depth + 1;
    }
    true
}

pub fn lockdep_release(lock_id: usize) {
    if !LOCKDEP_INIT.load(Ordering::Acquire) || !LOCKDEP_ENABLED.load(Ordering::Acquire) {
        return;
    }
    if lock_id == 0 || lock_id >= MAX_LOCKS {
        return;
    }
    let cpu = current_cpu();
    let mut state = LOCKDEP.lock();
    let depth = state.per_cpu_depth[cpu];
    if depth == 0 {
        return;
    }
    let top = state.per_cpu_stack[cpu][depth - 1];
    if top == lock_id {
        state.per_cpu_depth[cpu] = depth - 1;
    }
}

pub fn lockdep_status() -> LockdepSnapshot {
    let state = LOCKDEP.lock();
    let mut snapshot = LockdepSnapshot {
        violations: state.violations,
        class_count: state.class_count,
        edge_count: state.edge_count,
        classes: [""; MAX_LOCKS],
        edges: [(0, 0); MAX_EDGES],
    };
    for i in 0..state.class_count {
        snapshot.classes[i] = state.classes[i].name;
    }
    for i in 0..state.edge_count {
        snapshot.edges[i] = (state.edges[i].from, state.edges[i].to);
    }
    snapshot
}

pub fn lockdep_clear() {
    let mut state = LOCKDEP.lock();
    state.violations = 0;
    state.edge_count = 0;
    for cpu in 0..MAX_CPUS {
        state.per_cpu_depth[cpu] = 0;
    }
}

pub fn lockdep_enable(enabled: bool) {
    LOCKDEP_ENABLED.store(enabled, Ordering::Release);
}

pub fn lockdep_is_enabled() -> bool {
    LOCKDEP_ENABLED.load(Ordering::Acquire)
}

pub struct LockdepSnapshot {
    pub violations: u64,
    pub class_count: usize,
    pub edge_count: usize,
    pub classes: [&'static str; MAX_LOCKS],
    pub edges: [(usize, usize); MAX_EDGES],
}
