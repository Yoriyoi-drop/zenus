use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering, AtomicU64};
use x86_64::instructions::interrupts;

#[repr(C)]
pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

static DEADLOCK_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct SpinLockGuard<'a, T> {
    lock: &'a SpinLock<T>,
    irq_was_enabled: bool,
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

    fn deadlock_warning(_locked: &AtomicBool) {
        let n = DEADLOCK_COUNTER.fetch_add(1, Ordering::Relaxed);
        if n > 3 {
            return;
        }
        unsafe {
            let mut lsr: u8;
            core::arch::asm!("in al, dx", out("al") lsr, in("dx") 0x3FDu16, options(nostack, preserves_flags));
            if lsr & 0x20 != 0 {
                let msg = b"SPINLOCK DEADLOCK\n";
                for &b in msg {
                    core::arch::asm!("out dx, al", in("dx") 0x3F8u16, in("al") b, options(nostack, preserves_flags));
                }
            }
        }
    }

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let irq_was_enabled = interrupts::are_enabled();
        if irq_was_enabled {
            interrupts::disable();
        }
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
            }
        }
        SpinLockGuard { lock: self, irq_was_enabled }
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
            }
        }
        SpinLockGuard { lock: self, irq_was_enabled: false }
    }

    pub fn try_lock(&self) -> Option<SpinLockGuard<'_, T>> {
        let irq_was_enabled = interrupts::are_enabled();
        if irq_was_enabled {
            interrupts::disable();
        }
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard { lock: self, irq_was_enabled })
        } else {
            if irq_was_enabled {
                interrupts::enable();
            }
            None
        }
    }

    pub fn try_lock_no_irq(&self) -> Option<SpinLockGuard<'_, T>> {
        if self
            .locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
        {
            Some(SpinLockGuard { lock: self, irq_was_enabled: false })
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
        if self.irq_was_enabled {
            interrupts::enable();
        }
    }
}
