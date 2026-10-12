//! What an unrecoverable CPU exception should do (BUG-042).
//!
//! The page-fault handler used to end in a diagnostic dump followed by
//! `hlt` — for *every* fault, including one a user program caused by
//! dereferencing NULL. A single bad pointer in a ring-3 program therefore took
//! the whole machine down: the fault could not be recovered, so the kernel
//! stopped instead of killing the task. `SECURITY.md` and `DEVLOG.md` listed
//! that as an open audit finding ("no process kill on a user-mode page fault").
//!
//! The decision is a pure function — [`fault_verdict`] — so it can be tested on
//! the host. The *action* for [`FaultVerdict::KillUser`] needs the scheduler,
//! which this crate cannot depend on (it is the layer below), so the handler
//! calls a function pointer registered at boot with
//! [`register_user_fault_handler`]. With no handler registered the handler keeps
//! the old behaviour, which is why this is safe to bring up incrementally.

use core::sync::atomic::{AtomicUsize, Ordering};

/// Kernel addresses start here; anything below is user space.
pub const KERNEL_BASE: u64 = 0xFFFF_8000_0000_0000;

/// The lazily-mapped window: `brk` and `mmap` hand out pages here, and a
/// not-present fault inside it is filled in on demand rather than being fatal.
pub const LAZY_MAP_BASE: u64 = 0x6000_0000_0000;
pub const LAZY_MAP_END: u64 = 0x8000_0000_0000;

pub fn is_kernel_addr(addr: u64) -> bool {
    addr >= KERNEL_BASE
}

/// What the exception handler should do with a fault it received.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum FaultVerdict {
    /// Demand paging: map the missing page and retry the faulting instruction.
    Recover,
    /// A user-mode task faulted on something it may not touch. Kill it with
    /// `SIGSEGV` instead of taking the kernel down with it.
    KillUser,
    /// A supervisor fault, or a user fault whose `RIP` is in the kernel: the
    /// kernel itself is broken. Dump and halt.
    Panic,
}

/// Decide what to do with a page fault, from the values the CPU supplied.
///
/// `code_bits` is the page-fault error code: bit 0 present, bit 1 write, bit 2
/// user, bit 3 reserved-write, bit 4 instruction fetch. `addr` is `CR2`, `rip`
/// the faulting instruction pointer.
///
/// The order matters, and each rule is a bug that was fixed:
///
/// * **A fault whose `RIP` is in the kernel is always fatal.** `U/S` in the
///   error code says which mode the *access* came from, but a user-mode fault
///   whose `RIP` is a kernel address is a contradiction the kernel must not try
///   to survive.
/// * **A `U/S = 0` (supervisor) fault is always fatal.** It is the kernel
///   touching memory it may not, and "recovering" it by mapping a page would
///   turn a kernel bug into silent corruption.
/// * **Only a *not-present* fault is demand-paged.** The old recovery path
///   checked only the address window, so a user *write* to a present read-only
///   page — a protection violation — was treated as demand paging and answered
///   by mapping a fresh writable page over it. That silently discarded
///   `mprotect`'s `PROT_READ`, i.e. it defeated the protection instead of
///   raising `SIGSEGV`. Bit 0 set means protection violation, never recovery.
pub fn fault_verdict(code_bits: u64, addr: u64, rip: u64) -> FaultVerdict {
    let from_user = (code_bits & 0x4) != 0;
    if !from_user || is_kernel_addr(rip) {
        return FaultVerdict::Panic;
    }
    let not_present = (code_bits & 0x1) == 0;
    let in_lazy_window = (LAZY_MAP_BASE..LAZY_MAP_END).contains(&addr);
    if not_present && in_lazy_window && addr >= 0x1000 {
        return FaultVerdict::Recover;
    }
    FaultVerdict::KillUser
}

/// The task-killing action, provided by the scheduler layer at boot.
///
/// `extern "C" fn(addr, rip) -> !`: it delivers the fault, terminates the
/// current task and switches away, and never returns to the faulting
/// instruction.
pub type UserFaultHandler = extern "C" fn(u64, u64) -> !;

static USER_FAULT_HANDLER: AtomicUsize = AtomicUsize::new(0);

/// Register the handler the fault path calls to kill a user task.
///
/// Called once from `apps::entry` after the scheduler exists. Until then
/// [`user_fault_handler`] returns `None` and the handler falls back to the old
/// dump-and-halt, which is the safe direction for a boot that has no tasks yet.
pub fn register_user_fault_handler(f: UserFaultHandler) {
    USER_FAULT_HANDLER.store(f as usize, Ordering::Release);
}

/// The registered handler, if any.
pub fn user_fault_handler() -> Option<UserFaultHandler> {
    match USER_FAULT_HANDLER.load(Ordering::Acquire) {
        0 => None,
        // SAFETY: the only store to this atomic is `register_user_fault_handler`,
        // which stores a `UserFaultHandler as usize`. Reconstructing that exact
        // type is the inverse, and the value is non-zero here.
        a => Some(unsafe { core::mem::transmute::<usize, UserFaultHandler>(a) }),
    }
}
