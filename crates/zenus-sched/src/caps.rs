//! A per-task capability set.
//!
//! Before this, "privileged" meant `euid == 0`, and every task starts at euid 0,
//! so `sys_mount`, `sys_umount2` and `sys_sethostname` accepted every caller:
//! the check was there, but nothing could ever fail it. A task that wants to
//! shed privilege now drops a capability, and the check is against this set.
//!
//! There is **one** set per task, not Linux's permitted/effective/inheritable
//! split, and it acts as both the bounding set and the effective set. That is a
//! deliberate simplification rather than an oversight: it is why
//! `PR_CAPBSET_DROP` is the only way to shrink it, and why a future `capset`
//! would be the same operation under a different name.

/// Number of capabilities the kernel knows about.
pub const CAP_COUNT: u8 = 17;

pub const CAP_CHOWN: u8 = 0;
pub const CAP_DAC_OVERRIDE: u8 = 1;
pub const CAP_DAC_READ_SEARCH: u8 = 2;
pub const CAP_FOWNER: u8 = 3;
pub const CAP_FSETID: u8 = 4;
pub const CAP_KILL: u8 = 5;
pub const CAP_SETGID: u8 = 6;
pub const CAP_SETUID: u8 = 7;
pub const CAP_SETPCAP: u8 = 8;
pub const CAP_NET_BIND_SERVICE: u8 = 9;
pub const CAP_NET_RAW: u8 = 10;
pub const CAP_SYS_CHROOT: u8 = 11;
pub const CAP_SYS_PTRACE: u8 = 12;
pub const CAP_SYS_ADMIN: u8 = 13;
pub const CAP_SYS_BOOT: u8 = 14;
pub const CAP_SYS_TIME: u8 = 15;
pub const CAP_SYS_TTY_CONFIG: u8 = 16;

/// A bitmask of capabilities.
///
/// Copy, not shared state: the scheduler stores one per task, and dropping a
/// capability produces a new value rather than mutating the old one, so a
/// caller cannot be surprised by a set changing underneath it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CapSet(u64);

impl CapSet {
    /// Every capability. What a task created by the kernel starts with.
    pub const fn all() -> Self {
        CapSet((1u64 << CAP_COUNT) - 1)
    }

    /// No capabilities at all.
    pub const fn none() -> Self {
        CapSet(0)
    }

    /// Build a set from raw bits, discarding any bit past `CAP_COUNT`.
    ///
    /// Masking rather than rejecting keeps `from_bits` total, so a corrupted
    /// or future-width value cannot grant a capability that does not exist.
    pub const fn from_bits(bits: u64) -> Self {
        CapSet(bits & ((1u64 << CAP_COUNT) - 1))
    }

    pub const fn bits(self) -> u64 {
        self.0
    }

    /// Whether `cap` is in the set. Unknown capabilities are never held.
    pub const fn has(self, cap: u8) -> bool {
        cap < CAP_COUNT && (self.0 >> cap) & 1 == 1
    }

    /// The same set without `cap`. Dropping is idempotent.
    pub const fn without(self, cap: u8) -> Self {
        if cap < CAP_COUNT {
            CapSet(self.0 & !(1u64 << cap))
        } else {
            self
        }
    }
}

#[cfg(test)]
mod host_tests {
    #[test]
    fn all_holds_every_known_capability_and_none_holds_none() {
        let all = super::CapSet::all();
        for cap in 0..super::CAP_COUNT {
            assert!(all.has(cap), "all() must hold capability {}", cap);
        }
        assert_eq!(all.bits().count_ones(), super::CAP_COUNT as u32);

        let none = super::CapSet::none();
        for cap in 0..super::CAP_COUNT {
            assert!(!none.has(cap));
        }
    }

    #[test]
    fn dropping_a_capability_removes_exactly_that_one() {
        let all = super::CapSet::all();
        let no_admin = all.without(super::CAP_SYS_ADMIN);

        assert!(!no_admin.has(super::CAP_SYS_ADMIN));
        for cap in 0..super::CAP_COUNT {
            if cap != super::CAP_SYS_ADMIN {
                assert!(no_admin.has(cap), "capability {} must survive", cap);
            }
        }

        // Idempotent, and it does not resurrect anything.
        assert_eq!(no_admin.without(super::CAP_SYS_ADMIN), no_admin);
    }

    #[test]
    fn an_unknown_capability_is_never_held() {
        let all = super::CapSet::all();
        assert!(!all.has(super::CAP_COUNT));
        assert!(!all.has(200));
        // Dropping an unknown capability is a no-op, not a wrap-around.
        assert_eq!(all.without(200), all);
    }

    #[test]
    fn from_bits_cannot_grant_a_capability_past_the_known_range() {
        let set = super::CapSet::from_bits(u64::MAX);
        assert_eq!(set, super::CapSet::all());
        assert!(!set.has(super::CAP_COUNT));
        assert_eq!(super::CapSet::from_bits(0), super::CapSet::none());
    }
}
