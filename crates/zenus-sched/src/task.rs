use core::sync::atomic::{AtomicU32, Ordering};
use zenus_ns::NsId;

// VMA bookkeeping lives in `zenus-mem::vma` and is re-exported here so the
// existing `zenus_sched::task::VmaTable` / `MAP_*` / `PROT_*` paths keep
// working. This used to be a byte-for-byte *second* copy of the whole VMA
// table inside `task.rs`, and `mmap` (`sys_mmap` -> `scheduler::with_vma`)
// used that copy while `zenus-mem`'s was dead code — so every fix and every
// bug existed twice, and only one copy was reachable.
pub use zenus_mem::vma::{
    VmaRegion, VmaTable, MAP_ANONYMOUS, MAP_FIXED, MAP_NORESERVE, MAP_POPULATE, MAP_PRIVATE,
    MAP_SHARED, MAX_MAPPING_SIZE, MAX_VMAS, PAGE_NO_EXECUTE, PAGE_USER, PAGE_WRITABLE, PROT_EXEC,
    PROT_NONE, PROT_READ, PROT_WRITE,
};

static NEXT_UID: AtomicU32 = AtomicU32::new(0);
static NEXT_GID: AtomicU32 = AtomicU32::new(0);

pub fn alloc_uid() -> u32 {
    NEXT_UID.fetch_add(1, Ordering::SeqCst)
}

pub fn alloc_gid() -> u32 {
    NEXT_GID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(u8)]
pub enum TaskState {
    Ready,
    Running,
    Waiting,
    Sleeping,
    Terminated,
    Stopped,
}

impl TaskState {
    pub fn to_str(&self) -> &'static str {
        match self {
            TaskState::Ready => "Ready",
            TaskState::Running => "Running",
            TaskState::Waiting => "Waiting",
            TaskState::Sleeping => "Sleeping",
            TaskState::Terminated => "Terminated",
            TaskState::Stopped => "Stopped",
        }
    }
}

pub const SIG_MAX: usize = 64;

#[derive(Clone, Copy)]
pub struct SignalAction {
    pub handler_fn: u64,
    pub flags: u64,
    pub restorer: u64,
    pub mask: [u64; 2],
}

impl SignalAction {
    pub const fn new() -> Self {
        SignalAction {
            handler_fn: 0,
            flags: 0,
            restorer: 0,
            mask: [0; 2],
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct SavedUserContext {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rflags: u64,
    pub rip: u64,
    pub valid: bool,
}

impl SavedUserContext {
    pub const fn new() -> Self {
        SavedUserContext {
            rax: 0,
            rbx: 0,
            rcx: 0,
            rdx: 0,
            rsi: 0,
            rdi: 0,
            rbp: 0,
            rsp: 0,
            r8: 0,
            r9: 0,
            r10: 0,
            r11: 0,
            r12: 0,
            r13: 0,
            r14: 0,
            r15: 0,
            rflags: 0,
            rip: 0,
            valid: false,
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Task {
    pub id: u64,
    pub state: TaskState,
    pub priority: u8,
    pub rsp: u64,
    pub ticks_left: u64,
    pub stack_alloc: u64,
    pub stack_size: u64,
    pub kernel_rsp_top: u64,
    pub user_rsp: u64,
    pub cpu: u32,
    pub cr3: u64,
    pub heap_brk: u64,
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
    pub parent_pid: u64,
    pub exit_code: u64,
    pub uts_ns: NsId,
    pub pid_ns: NsId,
    pub mnt_ns: NsId,
    pub net_ns: NsId,
    pub user_ns: NsId,
    pub ipc_ns: NsId,
    pub name: [u8; 32],
    pub pending_signals: u64,
    pub signal_mask: u64,
    pub signal_actions: [SignalAction; SIG_MAX],
    pub saved_context: SavedUserContext,
    pub vma: VmaTable,
    pub cwd: [u8; 256],
}

impl Task {
    pub fn new(id: u64, stack: u64, name: &str) -> Self {
        let mut name_buf = [0u8; 32];
        let len = name.as_bytes().len().min(31);
        name_buf[..len].copy_from_slice(&name.as_bytes()[..len]);
        Task {
            id,
            state: TaskState::Ready,
            priority: 128,
            rsp: stack,
            ticks_left: 50,
            stack_alloc: 0,
            stack_size: 0,
            kernel_rsp_top: 0,
            user_rsp: 0,
            cpu: 0,
            cr3: 0,
            heap_brk: 0,
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
            parent_pid: 0,
            exit_code: 0,
            uts_ns: 0,
            pid_ns: 0,
            mnt_ns: 0,
            net_ns: 0,
            user_ns: 0,
            ipc_ns: 0,
            name: name_buf,
            pending_signals: 0,
            signal_mask: 0,
            signal_actions: [SignalAction::new(); SIG_MAX],
            saved_context: SavedUserContext::new(),
            vma: VmaTable::new(),
            cwd: {
                let mut buf = [0u8; 256];
                buf[0] = b'/';
                buf
            },
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, TaskState::Ready | TaskState::Running)
    }

    pub fn has_pending_signal(&self) -> bool {
        self.pending_signals & !self.signal_mask != 0
    }

    pub fn next_signal(&self) -> Option<usize> {
        let pending = self.pending_signals & !self.signal_mask;
        if pending == 0 {
            return None;
        }
        Some(pending.trailing_zeros() as usize)
    }

    pub fn deliver_signal(&mut self, sig: usize) {
        self.pending_signals |= 1u64 << sig;
    }

    pub fn clear_signal(&mut self, sig: usize) {
        self.pending_signals &= !(1u64 << sig);
    }
}

pub const MAX_TASKS: usize = 128;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct TaskInfo {
    pub id: u64,
    pub state: TaskState,
    pub cpu: u32,
    pub uid: u32,
    pub gid: u32,
    pub uts_ns: NsId,
    pub pid_ns: NsId,
    pub net_ns: NsId,
    pub user_ns: NsId,
    pub ipc_ns: NsId,
    pub name: [u8; 32],
}
