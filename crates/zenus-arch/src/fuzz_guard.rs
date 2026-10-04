//! Fault containment for in-kernel fuzzing.
//!
//! A fuzzer that executes fuzzed syscall arguments in ring 0 will, sooner or
//! later, hit a page fault / #GP / #UD. Without containment the first such
//! input kills the VM and the campaign finds exactly one bug per boot.
//!
//! Instead the fuzzer arms a *checkpoint* right before each test case. Any CPU
//! exception raised while the checkpoint is armed is recorded (vector, faulting
//! RIP, fault address, error code) and control is transferred back to the
//! checkpoint with an `iretq` frame, so the campaign keeps running and can
//! report every distinct crash it found.
//!
//! This is the same "longjmp out of a fault handler" technique used by
//! in-kernel fuzzers; the difference is that the recovery point is a kernel
//! stack pointer + instruction pointer rather than a userspace sigjmp_buf.
//!
//! Rules for callers inside exception handlers:
//!   1. call [`should_recover`] first — it returns false when no checkpoint is
//!      armed *or* when we are already inside a fault (re-entrancy), in which
//!      case the handler must fall through to its normal panic path;
//!   2. on true, call [`recover_to_checkpoint`] which never returns;
//!   3. on the recovery path only atomics and raw serial writes are allowed —
//!      the fault may have been raised with a spinlock held.

use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// GDT selectors used to build the ring0 -> ring0 IRETQ frame.
const KERNEL_CS: u64 = 0x08; // GDT index 1 (kernel code)
const KERNEL_SS: u64 = 0x10; // GDT index 2 (kernel data)
/// RFLAGS for the recovered context: IF=1 (interrupts back on) and bit 1
/// reserved bit which is architecturally required to be 1.
const RECOVERY_RFLAGS: u64 = 0x202;

static ARMED: AtomicBool = AtomicBool::new(false);
static IN_FAULT: AtomicBool = AtomicBool::new(false);
/// Set by [`recover_to_checkpoint`] and cleared by [`take_resumed`].
///
/// The checkpoint resumes in the *middle* of the code that armed it, so the
/// resumed code has to know not to re-run the operation that faulted —
/// otherwise it faults again with the checkpoint disarmed and the campaign
/// dies on the second occurrence instead of recording the first.
static RESUMED: AtomicBool = AtomicBool::new(false);
static POINT_RSP: AtomicU64 = AtomicU64::new(0);
static POINT_RIP: AtomicU64 = AtomicU64::new(0);

/// Number of faults contained since the last [`reset`].
pub static FAULTS: AtomicU64 = AtomicU64::new(0);
/// CPU exception vector of the most recently contained fault.
pub static LAST_VECTOR: AtomicU64 = AtomicU64::new(u64::MAX);
/// Faulting instruction pointer of the most recently contained fault.
pub static LAST_RIP: AtomicU64 = AtomicU64::new(0);
/// Fault address (CR2 for #PF) of the most recently contained fault.
pub static LAST_ADDR: AtomicU64 = AtomicU64::new(0);
/// Exception error code of the most recently contained fault.
pub static LAST_ERR: AtomicU64 = AtomicU64::new(0);
/// Contained faults that could not be attributed to a checkpoint (nested).
pub static UNRECOVERED: AtomicU64 = AtomicU64::new(0);

/// Arm a recovery checkpoint. `rsp`/`rip` must describe the state the fuzzer
/// wants to resume at.
///
/// Callers should normally pass a placeholder here and publish the real point
/// with [`set_point`] right after capturing `rsp`/`rip`: with `#[inline]` `arm`
/// is a handful of stores, so a recovery that lands on them would re-arm and
/// re-run the test case instead of returning to the campaign loop.
#[inline]
pub fn arm(rsp: u64, rip: u64) {
    POINT_RSP.store(rsp, Ordering::Release);
    POINT_RIP.store(rip, Ordering::Release);
    IN_FAULT.store(false, Ordering::Release);
    RESUMED.store(false, Ordering::Release);
    ARMED.store(true, Ordering::Release);
}

/// True when the code that just ran was resumed by fault containment.
///
/// Consumes the flag, so the next test case starts clean.
#[inline]
pub fn take_resumed() -> bool {
    RESUMED.swap(false, Ordering::AcqRel)
}

/// Non-consuming peek, for diagnostics.
#[inline]
pub fn resumed() -> bool {
    RESUMED.load(Ordering::Acquire)
}

/// Update the recovery point without (re-)arming the checkpoint.
///
/// Callers that capture `rsp`/`rip` *after* [`arm`] need this: arming with a
/// placeholder and publishing the real point afterwards means a recovery lands
/// on the instruction following the capture, i.e. once, in the caller's own
/// code — not back inside `arm()` itself, which would re-arm and loop forever.
#[inline]
pub fn set_point(rsp: u64, rip: u64) {
    POINT_RSP.store(rsp, Ordering::Release);
    POINT_RIP.store(rip, Ordering::Release);
}

/// Clear the checkpoint. Call after a test case completes without a fault.
#[inline]
pub fn disarm() {
    ARMED.store(false, Ordering::Release);
    IN_FAULT.store(false, Ordering::Release);
}

#[inline]
pub fn is_armed() -> bool {
    ARMED.load(Ordering::Acquire)
}

/// True when a fault happened while a checkpoint was armed.
#[inline]
pub fn faulted() -> bool {
    FAULTS.load(Ordering::Acquire) != 0
}

/// Consume the "a fault just happened" flag for the current test case.
#[inline]
pub fn take_fault() -> bool {
    FAULTS.swap(0, Ordering::AcqRel) != 0
}

/// Guard called at the top of an exception handler. Returns true when the
/// handler must hand over to [`recover_to_checkpoint`] instead of panicking.
#[inline]
pub fn should_recover() -> bool {
    if !ARMED.load(Ordering::Acquire) {
        return false;
    }
    // The checkpoint is a single (rsp, rip) pair, so it is only valid on the
    // CPU that armed it. Recovering an application-processor fault would jump
    // that core onto the boot stack, so APs always take the normal panic path.
    if crate::smp::current_cpu() != 0 {
        return false;
    }
    // A fault raised while we are already recovering cannot be contained:
    // unwinding again would loop forever, so let the normal path panic.
    if IN_FAULT.swap(true, Ordering::AcqRel) {
        UNRECOVERED.fetch_add(1, Ordering::Relaxed);
        return false;
    }
    true
}

/// Record a contained fault and resume the fuzz loop at the armed checkpoint.
///
/// Never returns. Deliberately allocation-free and lock-free: the faulting
/// code may have died holding the heap or a subsystem spinlock.
#[inline(never)]
pub unsafe fn recover_to_checkpoint(vector: u64, rip: u64, addr: u64, err: u64) -> ! {
    use core::arch::asm;

    LAST_VECTOR.store(vector, Ordering::Relaxed);
    LAST_RIP.store(rip, Ordering::Relaxed);
    LAST_ADDR.store(addr, Ordering::Relaxed);
    LAST_ERR.store(err, Ordering::Relaxed);
    FAULTS.fetch_add(1, Ordering::Relaxed);

    let target_rsp = POINT_RSP.load(Ordering::Acquire);
    let target_rip = POINT_RIP.load(Ordering::Acquire);
    ARMED.store(false, Ordering::Release);
    IN_FAULT.store(false, Ordering::Release);
    RESUMED.store(true, Ordering::Release);

    // Build the IRETQ frame for a ring0 -> ring0 return.
    //
    // `iretq` pops RIP/CS/RFLAGS/RSP/SS from the *current* RSP, so the frame
    // has to be sitting on the stack we are about to switch to. It cannot be
    // left on the fault stack: switching RSP first would leave `iretq` reading
    // whatever happens to be at the top of the checkpoint stack, which faults
    // with #GP (error code 0x20) and kills the campaign.
    //
    // Sequence:
    //   1. push the frame on the fault stack (memory is fine there);
    //   2. stash the target stack pointer below the frame so it survives the
    //      register wipe;
    //   3. wipe the general registers through that scratch space — the frame
    //      itself is never in the way, and the wipe cannot destroy the
    //      operands because they now live in memory;
    //   4. copy the frame to `target_rsp - 40`, switch RSP there, `iretq`.
    asm!(
        // 1. frame on the fault stack, in iretq pop order
        "push {ss}",
        "push {rsp_val}",
        "push {rflags}",
        "push {cs}",
        "push {rip_val}",
        // 2. stash the checkpoint stack pointer just below the frame
        "push {rsp_val}",
        // 3. wipe the general registers (scratch space below RSP)
        "push 0",
        "pop rax",
        "push 0",
        "pop rbx",
        "push 0",
        "pop rcx",
        "push 0",
        "pop rdx",
        "push 0",
        "pop rsi",
        "push 0",
        "pop rdi",
        "push 0",
        "pop rbp",
        "push 0",
        "pop r8",
        "push 0",
        "pop r9",
        "push 0",
        "pop r10",
        "push 0",
        "pop r11",
        "push 0",
        "pop r12",
        "push 0",
        "pop r13",
        "push 0",
        "pop r14",
        "push 0",
        "pop r15",
        // 4. frame lives at [rsp + 8, rsp + 48); target stack pointer at [rsp]
        "mov rcx, [rsp]",
        "mov rax, [rsp + 8]",
        "mov [rcx - 40], rax", // RIP
        "mov rax, [rsp + 16]",
        "mov [rcx - 32], rax", // CS
        "mov rax, [rsp + 24]",
        "mov [rcx - 24], rax", // RFLAGS
        "mov rax, [rsp + 32]",
        "mov [rcx - 16], rax", // RSP
        "mov rax, [rsp + 40]",
        "mov [rcx - 8], rax",  // SS
        "sub rcx, 40",
        "mov rsp, rcx",
        "iretq",
        ss = in(reg) KERNEL_SS,
        rsp_val = in(reg) target_rsp,
        rflags = in(reg) RECOVERY_RFLAGS,
        cs = in(reg) KERNEL_CS,
        rip_val = in(reg) target_rip,
        options(noreturn),
    )
}

/// Human-readable name for a CPU exception vector.
pub fn vector_name(vector: u64) -> &'static str {
    match vector {
        0 => "#DE divide-error",
        1 => "#DB debug",
        2 => "NMI",
        3 => "#BP breakpoint",
        4 => "#OF overflow",
        5 => "#BR bound-range",
        6 => "#UD invalid-opcode",
        7 => "#NM device-not-available",
        8 => "#DF double-fault",
        10 => "#TS invalid-tss",
        11 => "#NP segment-not-present",
        12 => "#SS stack-segment",
        13 => "#GP general-protection",
        14 => "#PF page-fault",
        _ => "unknown-vector",
    }
}

/// Reset counters between campaigns. The checkpoint is cleared too.
pub fn reset() {
    ARMED.store(false, Ordering::Release);
    IN_FAULT.store(false, Ordering::Release);
    RESUMED.store(false, Ordering::Release);
    FAULTS.store(0, Ordering::Release);
    UNRECOVERED.store(0, Ordering::Release);
    LAST_VECTOR.store(u64::MAX, Ordering::Release);
    LAST_RIP.store(0, Ordering::Release);
    LAST_ADDR.store(0, Ordering::Release);
    LAST_ERR.store(0, Ordering::Release);
}
