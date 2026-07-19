use core::sync::atomic::{AtomicU32, Ordering};
use zenus_ns::NsId;

pub const MAX_VMAS: usize = 64;

pub const MAP_SHARED: u64 = 0x01;
pub const MAP_PRIVATE: u64 = 0x02;
pub const MAP_FIXED: u64 = 0x10;
pub const MAP_ANONYMOUS: u64 = 0x20;
pub const MAP_POPULATE: u64 = 0x008000;
pub const MAP_NORESERVE: u64 = 0x04000;
pub const PROT_NONE: u64 = 0x0;
pub const PROT_READ: u64 = 0x1;
pub const PROT_WRITE: u64 = 0x2;
pub const PROT_EXEC: u64 = 0x4;
pub const PAGE_USER: u64 = 1 << 2;
pub const PAGE_WRITABLE: u64 = 1 << 1;
pub const PAGE_NO_EXECUTE: u64 = 1u64 << 63;

#[derive(Clone, Copy)]
pub struct VmaRegion {
    pub start: u64,
    pub end: u64,
    pub prot: u64,
    pub flags: u64,
    pub file_offset: u64,
    pub inode: u64,
    pub valid: bool,
}

impl VmaRegion {
    pub const fn new() -> Self {
        VmaRegion { start: 0, end: 0, prot: 0, flags: 0, file_offset: 0, inode: 0, valid: false }
    }
    pub fn contains(&self, addr: u64) -> bool {
        self.valid && addr >= self.start && addr < self.end
    }
}

#[derive(Clone, Copy)]
pub struct VmaTable {
    pub regions: [VmaRegion; MAX_VMAS],
    pub count: usize,
    pub mmap_base: u64,
}

impl VmaTable {
    pub const fn new() -> Self {
        VmaTable { regions: [VmaRegion::new(); MAX_VMAS], count: 0, mmap_base: 0x2000_0000_0000 }
    }
    pub fn insert(&mut self, start: u64, end: u64, prot: u64, flags: u64) -> Option<usize> {
        if self.count >= MAX_VMAS { return None; }
        let idx = self.count;
        self.regions[idx] = VmaRegion { start, end, prot, flags, file_offset: 0, inode: 0, valid: true };
        self.count += 1;
        Some(idx)
    }
    pub fn find(&self, addr: u64) -> Option<usize> {
        for i in 0..self.count {
            if self.regions[i].contains(addr) { return Some(i); }
        }
        None
    }
    pub fn remove(&mut self, idx: usize) -> bool {
        if idx >= self.count { return false; }
        self.regions[idx].valid = false;
        let mut write = 0;
        for read in 0..self.count {
            if self.regions[read].valid {
                if write != read { self.regions[write] = self.regions[read]; }
                write += 1;
            }
        }
        self.count = write;
        true
    }
    pub fn find_free(&self, size: u64, hint: u64) -> Option<u64> {
        let start = hint & !0xFFF;
        let end = start + size;
        if end > 0x7F00_0000_0000 { return None; }
        for i in 0..self.count {
            let r = &self.regions[i];
            if !r.valid { continue; }
            if start >= r.start && start < r.end { return self.find_free(size, r.end); }
            if end > r.start && end <= r.end { return self.find_free(size, r.end); }
        }
        Some(start)
    }
}

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
        SignalAction { handler_fn: 0, flags: 0, restorer: 0, mask: [0; 2] }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct SavedUserContext {
    pub rax: u64, pub rbx: u64, pub rcx: u64, pub rdx: u64,
    pub rsi: u64, pub rdi: u64, pub rbp: u64, pub rsp: u64,
    pub r8: u64,  pub r9: u64,  pub r10: u64, pub r11: u64,
    pub r12: u64, pub r13: u64, pub r14: u64, pub r15: u64,
    pub rflags: u64, pub rip: u64,
    pub valid: bool,
}

impl SavedUserContext {
    pub const fn new() -> Self {
        SavedUserContext {
            rax: 0, rbx: 0, rcx: 0, rdx: 0,
            rsi: 0, rdi: 0, rbp: 0, rsp: 0,
            r8: 0, r9: 0, r10: 0, r11: 0,
            r12: 0, r13: 0, r14: 0, r15: 0,
            rflags: 0, rip: 0, valid: false,
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
        if pending == 0 { return None; }
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
