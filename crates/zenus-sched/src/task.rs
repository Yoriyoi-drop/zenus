use core::sync::atomic::{AtomicU32, Ordering};

static NEXT_UID: AtomicU32 = AtomicU32::new(0);
static NEXT_GID: AtomicU32 = AtomicU32::new(0);

pub fn alloc_uid() -> u32 {
    NEXT_UID.fetch_add(1, Ordering::SeqCst)
}

pub fn alloc_gid() -> u32 {
    NEXT_GID.fetch_add(1, Ordering::SeqCst)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TaskState {
    Ready,
    Running,
    Waiting,
    Sleeping,
    Terminated,
}

impl TaskState {
    pub fn to_str(&self) -> &'static str {
        match self {
            TaskState::Ready => "Ready",
            TaskState::Running => "Running",
            TaskState::Waiting => "Waiting",
            TaskState::Sleeping => "Sleeping",
            TaskState::Terminated => "Terminated",
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
    pub cpu: u32,
    pub cr3: u64,
    pub heap_brk: u64,
    pub uid: u32,
    pub gid: u32,
    pub euid: u32,
    pub egid: u32,
}

impl Task {
    pub fn new(id: u64, stack: u64) -> Self {
        Task {
            id,
            state: TaskState::Ready,
            priority: 128,
            rsp: stack,
            ticks_left: 50,
            stack_alloc: 0,
            stack_size: 0,
            kernel_rsp_top: 0,
            cpu: 0,
            cr3: 0,
            heap_brk: 0,
            uid: 0,
            gid: 0,
            euid: 0,
            egid: 0,
        }
    }

    pub fn is_active(&self) -> bool {
        matches!(self.state, TaskState::Ready | TaskState::Running)
    }
}

pub const MAX_TASKS: usize = 128;

#[derive(Clone, Copy)]
pub struct TaskInfo {
    pub id: u64,
    pub state: TaskState,
    pub cpu: u32,
    pub uid: u32,
    pub gid: u32,
}
