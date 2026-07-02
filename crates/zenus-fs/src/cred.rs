/// Current-task credential store for VFS permission checks.
///
/// This module breaks the would-be cyclic dependency between zenus-fs and
/// zenus-sched.  Instead of importing zenus-sched from zenus-fs, the scheduler
/// calls `cred::set_current(uid, gid, euid, egid)` on every context switch, and
/// the VFS reads the values back with `cred::current_euid()` / `cred::current_egid()`.
///
/// On SMP each logical CPU stores its own credentials in a per-CPU slot so that
/// concurrent tasks running on different cores do not interfere with each other.
/// The CPU index is obtained via the APIC ID (capped to MAX_CPUS).

use core::sync::atomic::{AtomicU32, Ordering};

/// Maximum number of CPUs we track.  Must match zenus-sched::scheduler::MAX_CPUS.
const MAX_CPUS: usize = 8;

// Root (uid=0) is the safe default — the kernel itself runs as root before any
// task context is established.
static EUID: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];
static EGID: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];
static UID: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];
static GID: [AtomicU32; MAX_CPUS] = [
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
    AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0),
];

/// Read the current APIC ID and map it to a slot index.
/// Falls back to slot 0 if the APIC is not yet initialised.
#[inline(always)]
fn cpu_slot() -> usize {
    // Read IA32_TSC_AUX (MSR 0xC0000103) which is set by the scheduler to the
    // logical CPU index.  If not available we fall back to 0.
    // Using RDTSCP: stores TSC in EDX:EAX and CPU index in ECX.
    #[cfg(target_arch = "x86_64")]
    {
        let cpu_id: u32;
        unsafe {
            core::arch::asm!(
                "rdtscp",
                out("ecx") cpu_id,
                out("eax") _,
                out("edx") _,
                options(nostack, preserves_flags, nomem),
            );
        }
        (cpu_id as usize) % MAX_CPUS
    }
    #[cfg(not(target_arch = "x86_64"))]
    { 0 }
}

/// Called by the scheduler on every context switch to update credentials for
/// the incoming task.
#[inline]
pub fn set_current(uid: u32, gid: u32, euid: u32, egid: u32) {
    let s = cpu_slot();
    UID[s].store(uid, Ordering::Relaxed);
    GID[s].store(gid, Ordering::Relaxed);
    EUID[s].store(euid, Ordering::Relaxed);
    EGID[s].store(egid, Ordering::Relaxed);
}

/// Effective user-ID of the currently executing task on this CPU.
#[inline]
pub fn current_euid() -> u32 {
    EUID[cpu_slot()].load(Ordering::Relaxed)
}

/// Effective group-ID of the currently executing task on this CPU.
#[inline]
pub fn current_egid() -> u32 {
    EGID[cpu_slot()].load(Ordering::Relaxed)
}

/// Real user-ID.
#[inline]
pub fn current_uid() -> u32 {
    UID[cpu_slot()].load(Ordering::Relaxed)
}

/// Real group-ID.
#[inline]
pub fn current_gid() -> u32 {
    GID[cpu_slot()].load(Ordering::Relaxed)
}
