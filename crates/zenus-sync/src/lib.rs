#![no_std]

// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

pub mod irq_guard;
pub mod lockdep;
pub mod spinlock;

/// Host-side unit tests (`cargo test --target x86_64-unknown-linux-gnu`).
///
/// The kernel crate also carries the `#[cfg(feature = "testing")]` tests that
/// `apps/src/test_runner.rs` runs inside QEMU. Both layers exist on purpose:
/// the in-kernel ones exercise real MMIO/interrupt paths, the host ones give
/// instant feedback on the pure logic underneath.
///
/// Naming: the in-kernel harness calls `pub fn test_*() -> Result<(), &str>`,
/// so host modules are called `host_tests` and use `#[test]` directly. Never
/// put privileged instructions in them — `cli`, `in`/`out`, CR3/CR8 reads and
/// MSR access all fault in ring 3.
#[cfg(test)]
mod host_tests {
    use super::irq_guard::IrqGuard;
    use super::lockdep;
    use super::spinlock::{SpinLock, SpinLockGuard};

    /// Serialises the tests that touch the process-wide `LOCKDEP` state.
    /// `cargo test` runs tests on parallel threads, so without this the
    /// suites clear each other's counters mid-assertion.
    static SERIAL: SpinLock<()> = SpinLock::new(());

    fn serial() -> SpinLockGuard<'static, ()> {
        SERIAL.lock()
    }

    #[test]
    fn spinlock_locks_and_unlocks() {
        static LOCK: SpinLock<u32> = SpinLock::new(7);
        {
            let mut guard = LOCK.lock();
            assert_eq!(*guard, 7);
            *guard = 9;
        }
        assert_eq!(*LOCK.lock(), 9);
    }

    #[test]
    fn spinlock_try_lock_reports_contention() {
        static LOCK: SpinLock<u32> = SpinLock::new(0);
        let held = LOCK.lock();
        // `try_lock` must refuse while the guard is alive.
        assert!(LOCK.try_lock().is_none());
        drop(held);
        assert!(LOCK.try_lock().is_some());
    }

    #[test]
    fn spinlock_lock_no_irq_mutates() {
        static LOCK: SpinLock<u32> = SpinLock::new(1);
        *LOCK.lock_no_irq() += 41;
        assert_eq!(*LOCK.lock_no_irq(), 42);
    }

    /// `IrqGuard` masks interrupts on bare metal and compiles to nothing on the
    /// host (`cli` faults in ring 3), so there is nothing to observe here. The
    /// test that used to "cover" it only incremented its own counter, which
    /// passed whether or not the guard did anything.
    ///
    /// What *is* checkable off-target is that the type is usable as a scope
    /// guard and does not panic on construction or drop.
    #[test]
    fn irq_guard_is_a_scope_guard_on_host() {
        let mut nested: Option<IrqGuard> = None;
        {
            let _outer = IrqGuard::new();
            nested = Some(IrqGuard::new());
        }
        drop(nested.take());
    }

    #[test]
    fn lockdep_detects_reverse_edge() {
        let _serial = serial();
        lockdep::lockdep_init();
        lockdep::lockdep_clear();
        lockdep::lockdep_enable(true);

        let a = lockdep::lockdep_register("host_test_a");
        let b = lockdep::lockdep_register("host_test_b");
        assert_ne!(a, 0);
        assert_ne!(b, 0);

        // a -> b
        assert!(lockdep::lockdep_acquire(a, "test"));
        assert!(lockdep::lockdep_acquire(b, "test"));
        lockdep::lockdep_release(b);
        lockdep::lockdep_release(a);
        assert_eq!(lockdep::lockdep_status().violations, 0);

        // b -> a is the reverse edge: that is a lock-order inversion.
        assert!(lockdep::lockdep_acquire(b, "test"));
        assert!(!lockdep::lockdep_acquire(a, "test"));
        assert_eq!(lockdep::lockdep_status().violations, 1);

        lockdep::lockdep_release(a);
        lockdep::lockdep_release(b);
        lockdep::lockdep_clear();
    }

    #[test]
    fn lockdep_same_lock_twice_is_not_a_violation() {
        let _serial = serial();
        lockdep::lockdep_init();
        lockdep::lockdep_clear();
        lockdep::lockdep_enable(true);

        let a = lockdep::lockdep_register("host_test_reentrant");
        assert!(lockdep::lockdep_acquire(a, "test"));
        assert!(lockdep::lockdep_acquire(a, "test"));
        lockdep::lockdep_release(a);
        lockdep::lockdep_release(a);
        assert_eq!(lockdep::lockdep_status().violations, 0);

        lockdep::lockdep_clear();
    }

    #[test]
    fn lockdep_register_is_idempotent_and_reports_names() {
        let _serial = serial();
        lockdep::lockdep_init();
        lockdep::lockdep_clear();
        lockdep::lockdep_enable(true);

        let first = lockdep::lockdep_register("host_test_unique");
        let again = lockdep::lockdep_register("host_test_unique");
        assert_eq!(first, again);
        // IDs are 1-based: 0 is the "not registered" sentinel, so the very
        // first class must still be visible to `lockdep_acquire`.
        assert!(first > 0);
        let snapshot = lockdep::lockdep_status();
        assert_eq!(snapshot.classes[first], "host_test_unique");
        assert_eq!(snapshot.classes[0], "");

        lockdep::lockdep_clear();
    }

    #[test]
    fn lockdep_disabled_accepts_everything() {
        let _serial = serial();
        lockdep::lockdep_init();
        lockdep::lockdep_clear();
        lockdep::lockdep_enable(false);
        assert!(!lockdep::lockdep_is_enabled());

        // With lockdep off, `register` returns 0 and `acquire` waves everything
        // through — that is the whole point of the kill switch.
        let a = lockdep::lockdep_register("host_test_disabled");
        assert_eq!(a, 0);
        assert!(lockdep::lockdep_acquire(a, "test"));
        lockdep::lockdep_release(a);

        lockdep::lockdep_enable(true);
        lockdep::lockdep_clear();
    }
}
