use super::task::{SavedUserContext, Task, SIG_MAX};

/// The CR3 the calling CPU is running with.
///
/// Signal delivery happens from the timer handler on the interrupted task's
/// CR3, so this is the address space the user stack lives in — not the
/// kernel's. A stale or wrong CR3 here means the mapping checks below validate
/// the wrong address space.
#[inline]
fn current_cr3() -> u64 {
    let cr3: u64;
    unsafe {
        core::arch::asm!("mov {}, cr3", out(reg) cr3, options(nostack, preserves_flags));
    }
    cr3
}

// Linux signal numbers
pub const SIGHUP: usize = 1;
pub const SIGINT: usize = 2;
pub const SIGQUIT: usize = 3;
pub const SIGILL: usize = 4;
pub const SIGTRAP: usize = 5;
pub const SIGABRT: usize = 6;
pub const SIGBUS: usize = 7;
pub const SIGFPE: usize = 8;
pub const SIGKILL: usize = 9;
pub const SIGUSR1: usize = 10;
pub const SIGSEGV: usize = 11;
pub const SIGUSR2: usize = 12;
pub const SIGPIPE: usize = 13;
pub const SIGALRM: usize = 14;
pub const SIGTERM: usize = 15;
pub const SIGCHLD: usize = 17;
pub const SIGCONT: usize = 18;
pub const SIGSTOP: usize = 19;
pub const SIGTSTP: usize = 20;
pub const SIGTTIN: usize = 21;
pub const SIGTTOU: usize = 22;

// sigprocmask how
pub const SIG_BLOCK: u64 = 0;
pub const SIG_UNBLOCK: u64 = 1;
pub const SIG_SETMASK: u64 = 2;

// sigaction flags
pub const SA_RESTORER: u64 = 0x04000000;
pub const SA_RESTART: u64 = 0x10000000;
pub const SA_SIGINFO: u64 = 0x00000004;

// Default actions per signal
pub fn default_action(sig: usize) -> SignalDisposition {
    match sig {
        SIGHUP | SIGINT | SIGKILL | SIGPIPE | SIGALRM | SIGTERM => SignalDisposition::Terminate,
        SIGQUIT | SIGILL | SIGTRAP | SIGABRT | SIGBUS | SIGFPE | SIGSEGV => SignalDisposition::Core,
        SIGSTOP | SIGTSTP | SIGTTIN | SIGTTOU => SignalDisposition::Stop,
        SIGCONT => SignalDisposition::Continue,
        _ => SignalDisposition::Ignore,
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum SignalDisposition {
    Terminate,
    Core,
    Stop,
    Continue,
    Ignore,
    Handler,
}

/// Check if a task has pending signals and should be delivered.
/// Returns (signal_number, disposition) if signal should be delivered.
pub fn check_pending(task: &Task) -> Option<(usize, SignalDisposition)> {
    let pending = task.pending_signals & !task.signal_mask;
    if pending == 0 {
        return None;
    }

    let sig_num = pending.trailing_zeros() as usize;
    if sig_num >= SIG_MAX {
        return None;
    }

    let action = &task.signal_actions[sig_num];
    let disposition = if action.handler_fn != 0 {
        SignalDisposition::Handler
    } else {
        default_action(sig_num)
    };

    Some((sig_num, disposition))
}

/// Signal frame pushed onto user stack before entering signal handler.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct SignalFrame {
    pub rip: u64,
    pub rflags: u64,
    pub rsp: u64,
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub signal_number: u64,
    pub handler_fn: u64,
    pub restorer: u64,
}

/// Set up signal frame on user stack and redirect execution to handler.
/// Returns new user RSP after frame is pushed.
pub fn setup_signal_frame(
    task: &mut Task,
    sig: usize,
    user_rsp: u64,
    user_rip: u64,
    user_rflags: u64,
) -> Option<u64> {
    let action = &task.signal_actions[sig];
    let handler = action.handler_fn;
    let restorer = action.restorer;

    if handler == 0 {
        return None;
    }

    let frame_size = core::mem::size_of::<SignalFrame>() as u64;
    let new_rsp = (user_rsp - frame_size) & !0xF; // 16-byte align

    let frame = SignalFrame {
        rip: user_rip,
        rflags: user_rflags,
        rsp: user_rsp,
        rax: task.saved_context.rax,
        rbx: task.saved_context.rbx,
        rcx: task.saved_context.rcx,
        rdx: task.saved_context.rdx,
        rsi: task.saved_context.rsi,
        rdi: task.saved_context.rdi,
        rbp: task.saved_context.rbp,
        r8: task.saved_context.r8,
        r9: task.saved_context.r9,
        r10: task.saved_context.r10,
        r11: task.saved_context.r11,
        r12: task.saved_context.r12,
        r13: task.saved_context.r13,
        r14: task.saved_context.r14,
        r15: task.saved_context.r15,
        signal_number: sig as u64,
        handler_fn: handler,
        restorer,
    };

    // Write the frame to the user stack.
    //
    // Not a plain store: the page's U/S bit is 1, so with SMAP on a supervisor
    // write to it faults even though the page is present. `stac`/`clac` open
    // and close that window, and the compiler treats both as memory-opaque, so
    // the store cannot be hoisted out of it.
    //
    // `new_rsp` is not checked for a mapping either. An unmapped frame lands as
    // a ring-0 fault inside the scheduler, and the timer handler is the one
    // asking, so the machine wedges; a `None` here makes delivery fail and the
    // signal stay pending instead.
    if zenus_mem::paging::virt_to_phys_raw(current_cr3(), new_rsp).is_none() {
        return None;
    }
    unsafe {
        zenus_arch::cpu::stac();
        core::ptr::copy_nonoverlapping(
            &frame as *const SignalFrame,
            new_rsp as *mut SignalFrame,
            1,
        );
        zenus_arch::cpu::clac();
    }

    // Clear pending signal
    task.clear_signal(sig);

    Some(new_rsp)
}

/// Restore user context from signal frame.
/// Called from rt_sigreturn syscall to recover pre-signal state.
pub fn restore_signal_frame(user_rsp: u64) -> Option<(u64, SavedUserContext)> {
    // `rt_sigreturn` hands us a user pointer, so the same two rules as
    // `setup_signal_frame` apply in the other direction: the frame has to be
    // mapped, and the read has to happen inside `stac`/`clac` or SMAP faults it.
    // Without the translation check a bogus `rt_sigreturn` argument was a
    // ring-0 page fault on the way into restoring the context.
    if zenus_mem::paging::virt_to_phys_raw(current_cr3(), user_rsp).is_none() {
        return None;
    }
    let frame = unsafe {
        zenus_arch::cpu::stac();
        let f = core::ptr::read_volatile(user_rsp as *const SignalFrame);
        zenus_arch::cpu::clac();
        f
    };

    // Restore user context
    let saved_context = SavedUserContext {
        rax: frame.rax,
        rbx: frame.rbx,
        rcx: frame.rcx,
        rdx: frame.rdx,
        rsi: frame.rsi,
        rdi: frame.rdi,
        rbp: frame.rbp,
        rsp: frame.rsp,
        r8: frame.r8,
        r9: frame.r9,
        r10: frame.r10,
        r11: frame.r11,
        r12: frame.r12,
        r13: frame.r13,
        r14: frame.r14,
        r15: frame.r15,
        rflags: frame.rflags,
        rip: frame.rip,
        valid: true,
    };

    // Return restored RSP and context
    Some((frame.rsp, saved_context))
}
