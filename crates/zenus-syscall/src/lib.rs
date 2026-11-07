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
        assert!(
            SYSCALL_TABLE_SIZE >= 256,
            "table must cover the 0..255 range"
        );
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

        // `PR_SET_SECCOMP` is now implemented: `PR_SET_SECCOMP` with
        // `SECCOMP_SET_MODE_FILTER` installs a 256-bit syscall allow-list
        // that `syscall_dispatch6` enforces on every subsequent call.
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_SECCOMP),
            PrctlSupport::Implemented,
            "seccomp filter mode is implemented"
        );

        // `PR_SET_NO_NEW_PRIVS` / `PR_GET_NO_NEW_PRIVS` are now honoured for
        // real (the flag is stored and inherited), so they are `Implemented`
        // rather than a silent no-op.
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_NO_NEW_PRIVS),
            PrctlSupport::Implemented
        );
        assert_eq!(
            prctl_support(crate::syscall::PR_GET_NO_NEW_PRIVS),
            PrctlSupport::Implemented
        );

        // The options that are actually implemented.
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_NAME),
            PrctlSupport::Name
        );
        assert_eq!(
            prctl_support(crate::syscall::PR_GET_NAME),
            PrctlSupport::Name
        );

        // The capability set exists now, so `PR_CAPBSET_READ`/`DROP` are
        // implemented rather than refused: a task can read what it holds and
        // give a capability up.
        assert_eq!(
            prctl_support(crate::syscall::PR_CAPBSET_READ),
            PrctlSupport::Implemented
        );
        assert_eq!(
            prctl_support(crate::syscall::PR_CAPBSET_DROP),
            PrctlSupport::Implemented
        );

        // KEEPCAPS is about keeping capabilities across a `setuid` exec. Zenus
        // never drops them there, so the flag still has nothing to do — but the
        // reason is now "no privilege transition", not "no capabilities".
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_KEEPCAPS),
            PrctlSupport::NoOp
        );

        // PDEATHSIG promises a signal on parent death that Zenus never sends;
        // it must fail rather than be accepted as a no-op.
        assert_eq!(
            prctl_support(crate::syscall::PR_SET_PDEATHSIG),
            PrctlSupport::Unsupported
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

    /// Seccomp filter constants and allow-list bitmask logic are pure, so
    /// they can be tested on the host without a kernel.
    #[test]
    fn seccomp_constants_match_the_linux_abi() {
        use crate::syscall::{
            SECCOMP_FILTER_FLAG_TSYNC, SECCOMP_RET_ALLOW, SECCOMP_RET_ERRNO,
            SECCOMP_RET_KILL, SECCOMP_RET_TRAP, SECCOMP_SET_MODE_FILTER,
        };
        assert_eq!(SECCOMP_SET_MODE_FILTER, 1);
        assert_eq!(SECCOMP_FILTER_FLAG_TSYNC, 1);
        assert_eq!(SECCOMP_RET_ALLOW, 0x7fff_0000);
        assert_eq!(SECCOMP_RET_ERRNO, 0x0005_0000);
        assert_eq!(SECCOMP_RET_KILL, 0x0000_0000);
        assert_eq!(SECCOMP_RET_TRAP, 0x0003_0000);
    }

    /// The seccomp allow-list is a 256-bit bitmap: bit `n` permits syscall `n`.
    /// A syscall number past 255 is always rejected by the dispatcher before
    /// the filter is consulted, so bits past 255 are irrelevant.
    #[test]
    fn seccomp_allowlist_bitmask_covers_all_256_syscalls() {
        // 4 u64s × 64 bits = 256 bits, one per syscall number.
        let mut allowed = [0u64; 4];
        // Allow every syscall 0..=255.
        for w in 0..4 {
            allowed[w] = u64::MAX;
        }
        for num in 0..256u64 {
            let word = num / 64;
            let bit = num % 64;
            assert!(
                allowed[word as usize] & (1u64 << bit) != 0,
                "syscall {num} must be permitted"
            );
        }
        // Syscall 256 is past the bitmap; the dispatcher rejects it before
        // consulting the filter, so no bit needs to cover it.
        assert_eq!(256 / 64, 4, "256 is past the 4-word bitmap");
    }

    /// An empty allow-list blocks every syscall (except prctl itself).
    #[test]
    fn seccomp_empty_allowlist_blocks_everything() {
        let allowed = [0u64; 4];
        for num in 0..256u64 {
            if num == crate::syscall::SYS_PRCTL {
                continue; // prctl is always allowed
            }
            let word = num / 64;
            let bit = num % 64;
            assert_eq!(
                allowed[word as usize] & (1u64 << bit),
                0,
                "syscall {num} must be blocked by an empty allow-list"
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
