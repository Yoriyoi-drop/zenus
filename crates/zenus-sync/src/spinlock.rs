use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
#[cfg(target_os = "none")]
use x86_64::instructions::interrupts;

#[repr(C)]
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

static DEADLOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Mask interrupts before taking the lock, returning the previous IF state.
///
/// Bare metal needs this: holding a spinlock across a context switch is what
/// wedges the machine. The host test build (`cargo test --target
/// x86_64-unknown-linux-gnu`) cannot execute `cli` from ring 3 — it takes
/// SIGSEGV — so the masking is compiled out there. The atomic CAS below still
/// provides mutual exclusion, which is all a single-threaded test needs.
#[cfg(target_os = "none")]
#[inline]
fn irq_disable() -> bool {
    let enabled = interrupts::are_enabled();
    if enabled {
        interrupts::disable();
    }
    enabled
}

/// Host test build: no interrupt masking (see the bare-metal twin above).
#[cfg(not(target_os = "none"))]
#[inline]
fn irq_disable() -> bool {
    false
}

/// Undo [`irq_disable`], restoring the IF state the caller had.
#[cfg(target_os = "none")]
#[inline]
fn irq_restore(enabled: bool) {
    if enabled {
        interrupts::enable();
    }
}

/// Host test build: nothing to restore.
#[cfg(not(target_os = "none"))]
#[inline]
fn irq_restore(_enabled: bool) {}

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    irq_was_enabled: bool,
}

/// RSP of whoever acquired the lock, so a long spin can name the stack that
/// holds it (the whole kernel hangs when a task is switched out while
/// holding a spinlock, and the waiting CPU has interrupts disabled).
static OWNER_RSP: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_os = "none")]
#[inline]
fn emit_emergency(msg: &[u8]) {
    unsafe {
        let mut lsr: u8;
        core::arch::asm!("in al, dx", out("al") lsr, in("dx") 0x3FDu16, options(nostack, preserves_flags));
        if lsr & 0x20 != 0 {
            for &b in msg {
                core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nostack, preserves_flags));
            }
        }
    }
}

/// Host test build: there is no UART behind us, so a stuck lock aborts the
/// test process (a silent spin would just hang the suite).
#[cfg(all(test, not(target_os = "none")))]
#[cold]
fn emit_emergency(_msg: &[u8]) {
    std::process::abort();
}

/// Host build outside the test harness: nothing to report to, keep spinning.
#[cfg(all(not(test), not(target_os = "none")))]
#[cold]
fn emit_emergency(_msg: &[u8]) {}

fn emit_hex(mut v: usize, out: &mut [u8]) {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for i in (0..16).rev() {
        out[i] = HEX[v & 0xf];
        v >>= 4;
    }
}

/// Read up to `N` return addresses above `rsp`, keeping only kernel-range ones.
///
/// Ring 0 only: on the host the "stack" is a thread stack, and `OWNER_RSP` is
/// 0 until some lock has been taken — dereferencing `0 + 8` there is a
/// guaranteed SIGSEGV that killed the whole test binary. Scanning is therefore
/// compiled out for the host build, which just gets the lock address.
#[cfg(target_os = "none")]
fn scan_kernel_returns(rsp: usize, out: &mut [usize; 6]) {
    if rsp == 0 {
        return;
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let value = unsafe { *((rsp + (i + 1) * 8) as *const usize) };
        if (0xffff_8000_0000..0xffff_ffff_ffff).contains(&value) {
            *slot = value;
        }
    }
}

/// Host twin: nothing to scan.
#[cfg(not(target_os = "none"))]
fn scan_kernel_returns(_rsp: usize, _out: &mut [usize; 6]) {}

/// Called from a spin loop: after a few million spins, dump who holds it.
#[cold]
fn long_spin_report(lock: usize, spinning_at: usize) {
    const REPORT_CAP: usize = 320;
    let owner = OWNER_RSP.load(Ordering::Relaxed);
    let mut calls = [0usize; 6];
    let mut holds = [0usize; 6];
    scan_kernel_returns(spinning_at, &mut calls);
    if owner != 0 {
        scan_kernel_returns(owner, &mut holds);
    }

    let mut buf = [0u8; REPORT_CAP];
    // Every append goes through `push_*`, which stops at the end of the
    // buffer. The previous hand-rolled `p3 += ...` arithmetic could walk past
    // 256 bytes and panic with interrupts disabled once 4+ scanned words were
    // kernel addresses.
    let mut used = 0usize;
    macro_rules! push {
        ($bytes:expr) => {{
            for &b in $bytes {
                if used < REPORT_CAP {
                    buf[used] = b;
                    used += 1;
                }
            }
        }};
    }
    macro_rules! push_hex {
        ($value:expr) => {{
            let mut hex = [0u8; 16];
            emit_hex($value, &mut hex);
            push!(&hex);
        }};
    }

    push!(b"SPIN: lock=");
    push_hex!(lock);
    push!(b" owner_rsp=");
    push_hex!(owner);
    push!(b" spin_rsp=");
    push_hex!(spinning_at);
    for c in holds.iter().filter(|c| **c != 0) {
        push!(b" h=");
        push_hex!(*c);
    }
    for c in calls.iter().filter(|c| **c != 0) {
        push!(b" c=");
        push_hex!(*c);
    }
    push!(b"\n");
    emit_emergency(&buf[..used]);
}

unsafe impl<T: Send> Send for SpinLock<T> {}
unsafe impl<T: Send> Sync for SpinLock<T> {}

impl<T> SpinLock<T> {
    pub const fn new(data: T) -> Self {
        SpinLock {
            locked: AtomicBool::new(false),
            data: UnsafeCell::new(data),
        }
    }

    #[cfg(target_os = "none")]
    fn deadlock_warning(_locked: &AtomicBool) {
        let n = DEADLOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
        if n > 3 {
            return;
        }
        emit_emergency(b"SPINLOCK DEADLOCK\n");
    }

    /// Host test build: a lock held for 100M spins means the test deadlocked;
    /// fail loudly instead of spinning forever.
    #[cfg(not(target_os = "none"))]
    #[cold]
    fn deadlock_warning(_locked: &AtomicBool) {
        let n = DEADLOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let _ = n;
        emit_emergency(b"SPINLOCK DEADLOCK\n");
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let irq_was_enabled = irq_disable();
        let mut backoff = 1u32;
        let mut spins = 0u64;
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.locked.load(Ordering::Relaxed) {
                for _ in 0..backoff {
                    core::hint::spin_loop();
                }
                backoff = backoff.saturating_mul(2).min(256);
                spins += 1;
                if spins > 100_000_000 {
                    Self::deadlock_warning(&self.locked);
                    spins = 0;
                }
                if spins == 50_000 || spins == 400_000 {
                    let here = current_rsp();
                    long_spin_report(self as *const Self as usize, here);
                }
            }
        }
        OWNER_RSP.store(current_rsp(), Ordering::Relaxed);
        SpinLockGuard {
            lock: self,
            irq_was_enabled,
        }
    }

    pub fn lock_no_irq(&self) -> SpinLockGuard<'_, T> {
        let mut backoff = 1u32;
        let mut spins = 0u64;
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.locked.load(Ordering::Relaxed) {
                for _ in 0..backoff {
                    core::hint::spin_loop();
                }
                backoff = backoff.saturating_mul(2).min(256);
                spins += 1;
                if spins > 100_000_000 {
                    Self::deadlock_warning(&self.locked);
                    spins = 0;
                }
                if spins == 50_000 || spins == 400_000 {
                    let here = current_rsp();
                    long_spin_report(self as *const Self as usize, here);
                }
            }
        }
        OWNER_RSP.store(current_rsp(), Ordering::Relaxed);
        SpinLockGuard {
            lock: self,
            irq_was_enabled: false,
        }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let irq_was_enabled = irq_disable();
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard {
                lock: self,
                irq_was_enabled,
            })
        } else {
            irq_restore(irq_was_enabled);
            None
        }
    }

    pub fn try_lock_no_irq(&self) -> Option<SpinLockGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard {
                lock: self,
                irq_was_enabled: false,
            })
        } else {
            None
        }
    }
}

impl<'a, T> Deref for SpinLockGuard<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        unsafe { &*self.lock.data.get() }
    }
}

impl<'a, T> DerefMut for SpinLockGuard<'a, T> {
    fn deref_mut(&mut self) -> &mut T {
        unsafe { &mut *self.lock.data.get() }
    }
}

impl<'a, T> Drop for SpinLockGuard<'a, T> {
    fn drop(&mut self) {
        self.lock.locked.store(false, Ordering::Release);
        irq_restore(self.irq_was_enabled);
    }
}

#[inline]
fn current_rsp() -> usize {
    let r: usize;
    unsafe { core::arch::asm!("mov {}, rsp", out(reg) r, options(nostack, preserves_flags)) };
    r
}
