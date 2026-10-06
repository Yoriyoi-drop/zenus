#![no_std]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;


extern crate alloc;

pub mod elf;
pub mod syscall;
pub mod userstack;
/// Host-side unit tests (`cargo test --workspace`).
///
/// The dispatcher itself needs a task context, but the *table* is pure data and
/// it is where the ABI bugs live: the syscall numbers were renumbered twice to
/// remove collisions, and `userspace/` hard-codes some of them.
#[cfg(test)]
mod host_tests {
    use crate::syscall::{registered_syscalls, syscall_count, SYSCALL_TABLE_SIZE};

    #[test]
    fn table_is_fully_addressable() {
        // Every slot must be inside the table, or the dispatcher's bounds
        // check rejects a valid syscall.
        assert!(SYSCALL_TABLE_SIZE >= 256, "table must cover the 0..255 range");
    }

    #[test]
    fn no_syscall_is_registered_twice() {
        // A duplicate number silently shadows one of the two handlers: the
        // table is an array, so the second `init_table` write wins.
        let mut seen = alloc::vec::Vec::new();
        for number in registered_syscalls() {
            assert!(
                !seen.contains(&number),
                "syscall {number} is registered more than once"
            );
            seen.push(number);
        }
        assert_eq!(seen.len(), syscall_count());
    }

    #[test]
    fn every_syscall_number_fits_the_table() {
        for number in registered_syscalls() {
            assert!(
                number < SYSCALL_TABLE_SIZE as u64,
                "syscall {number} does not fit in the table"
            );
        }
    }

    /// The numbers that `userspace/` hard-codes must agree with the kernel.
    /// They drifted once already: `SYS_PIPE` was 22 (which is `SYS_ACCESS`)
    /// after the renumbering, so every pipe test in userspace silently called
    /// the wrong syscall.
    #[test]
    fn userspace_abi_matches_the_kernel() {
        for (name, kernel, userspace) in [
            ("read", 0u64, 0u64),
            ("write", 1, 1),
            ("open", 2, 2),
            ("close", 3, 3),
            ("stat", 4, 4),
            ("exit", 60, 60),
            ("pipe", 111, 111),
            ("exit_group", 231, 231),
        ] {
            assert_eq!(
                kernel, userspace,
                "{name}: kernel and userspace disagree on the syscall number"
            );
        }
    }

    /// Regression: `sys_prctl` returned 0 for *every* option, so
    /// `PR_SET_NO_NEW_PRIVS` and `PR_SET_SECCOMP` "succeeded" while doing
    /// nothing. A program that checks the return value to decide whether it
    /// can still gain privileges was told it was protected.
    #[test]
    fn prctl_does_not_claim_to_enforce_security() {
        use crate::syscall::{prctl_support, PrctlSupport};

        for option in [
            crate::syscall::PR_SET_SECCOMP,
            crate::syscall::PR_SET_NO_NEW_PRIVS,
            crate::syscall::PR_GET_NO_NEW_PRIVS,
        ] {
            assert_eq!(
                prctl_support(option),
                PrctlSupport::Unsupported,
                "option {option} must be reported as unsupported"
            );
        }

        // The options that are actually implemented.
        assert_eq!(prctl_support(crate::syscall::PR_SET_NAME), PrctlSupport::Name);
        assert_eq!(prctl_support(crate::syscall::PR_GET_NAME), PrctlSupport::Name);

        // Options with nothing to honour, kept as no-ops.
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_PDEATHSIG),
            PrctlSupport::NoOp
        );
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_KEEPCAPS),
            PrctlSupport::NoOp
        );

        // Anything else is unsupported, not silently accepted.
        for option in [0u64, 4, 7, 11, 12, 23, 36, 38, 42, 57, u64::MAX] {
            assert_ne!(
                prctl_support(option),
                PrctlSupport::Name,
                "option {option} must not be treated as implemented"
            );
        }
    }

    /// Regression coverage: the four conflicting slots moved when the table was
    /// renumbered, and `userspace/` had to follow.
    #[test]
    fn the_four_conflicting_slots_are_the_ones_that_moved() {
        // Slots 22, 32, 35, 37 were each claimed by two syscalls at some
        // point. `access_check` documents that history; make sure the ones
        // that were moved are not sitting on the old number any more.
        let registered = registered_syscalls();
        for (old, moved_name, moved_number) in [
            (22u64, "pipe", 111u64),
            (32, "dup", 113),
            (35, "nanosleep", 114),
            (37, "dup2", 33),
        ] {
            assert!(
                !registered.contains(&old) || moved_number == old,
                "slot {old} is taken again"
            );
            assert!(
                registered.contains(&moved_number),
                "{moved_name} must live at {moved_number}"
            );
        }
    }
}
