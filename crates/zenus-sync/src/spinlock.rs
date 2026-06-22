use core::cell::UnsafeCell;
use core::ops::{Deref, DerefMut};
use core::sync::atomic::{AtomicBool, Ordering};
use x86_64::instructions::interrupts;

pub struct SpinLock<T> {
    locked: AtomicBool,
    data: UnsafeCell<T>,
}

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

    pub fn lock(&self) -> SpinLockGuard<'_, T> {
        let irq_was_enabled = interrupts::are_enabled();
        if irq_was_enabled {
            interrupts::disable();
        }
        while self
            .locked
            .compare_exchange_weak(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_err()
        {
            while self.locked.load(Ordering::Relaxed) {
                if irq_was_enabled {
                    interrupts::enable();
                    core::hint::spin_loop();
                    interrupts::disable();
                } else {
                    core::hint::spin_loop();
                }
            }
        }
        SpinLockGuard { lock: self, irq_was_enabled }
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
