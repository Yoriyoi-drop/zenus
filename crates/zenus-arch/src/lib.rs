#![no_std]
#![feature(abi_x86_interrupt)]
#![allow(static_mut_refs)]
#![allow(bad_asm_style)]
// The host test harness needs std; the bare-metal build must not pull it in.
#[cfg(test)]
extern crate std;

extern crate alloc;

pub mod acpi;
pub mod ata;
pub mod cpu;
pub mod crash;
pub mod fuzz_guard;
pub mod gdt;
pub mod interrupts;
pub mod keyboard;
pub mod limine;
pub mod pci;
pub mod random;
pub mod rtc;
pub mod smp;
pub mod user;
pub mod watchdog;

/// Host-side unit tests (`cargo test --workspace`).
///
/// `zenus-arch` is almost entirely MMIO/IDT work, so the host layer covers
/// only the pure constants that both sides of the interrupt path have to agree
/// on — a disagreement there is silent at runtime: an interrupt routed onto a
/// vector with no handler is acknowledged and dropped, and nothing logs it.
#[cfg(test)]
mod host_tests {
    use crate::interrupts::{
        NIC_VECTOR, RESERVED_VECTORS, SERIAL_VECTOR, SPURIOUS_VECTOR, TIMER_VECTOR,
    };

    /// Regression: the NIC interrupt was routed to `32 + irq_line` (QEMU's
    /// rtl8139 reports IRQ 10 → vector 42) while its handler is installed at
    /// vector 43, so every RX interrupt landed on the stray handler and
    /// `set_nic_irq_handler` never ran.
    #[test]
    fn nic_vector_is_the_slot_with_the_handler() {
        assert_eq!(NIC_VECTOR, 43);
        assert!(NIC_VECTOR > 32, "must be above the exception range");
        assert!(NIC_VECTOR < 48, "must stay inside the legacy PIC window");
        assert!(RESERVED_VECTORS.contains(&NIC_VECTOR));
    }

    /// Two handlers on one vector means one of them is never called.
    #[test]
    fn reserved_vectors_are_distinct() {
        for (i, a) in RESERVED_VECTORS.iter().enumerate() {
            for b in RESERVED_VECTORS.iter().skip(i + 1) {
                assert_ne!(a, b, "vectors {a} and {b} collide");
            }
        }
        assert_eq!(TIMER_VECTOR, 32);
        assert_eq!(SPURIOUS_VECTOR, 39);
        // The serial vector is legacy IRQ 4, i.e. 32 + 4 — that is what
        // `apps::entry` routes (route_irq(4, ...)).
        assert_eq!(SERIAL_VECTOR, TIMER_VECTOR + 4);
    }

    /// The two device interrupts Zenus routes must land on reserved vectors.
    ///
    /// Regression detail: QEMU's rtl8139 reports IRQ line 10, so the old
    /// `32 + irq_line` routing produced vector 42 — not a reserved slot, hence
    /// swallowed by `interrupt_stray`. Vector 43 happens to be `32 + 11`, so
    /// the bug was invisible for IRQ 11 and only bit on other machines.
    #[test]
    fn routed_device_vectors_are_reserved() {
        assert_eq!(SERIAL_VECTOR, 32 + 4, "serial is legacy IRQ 4");
        assert!(RESERVED_VECTORS.contains(&SERIAL_VECTOR));
        assert!(RESERVED_VECTORS.contains(&NIC_VECTOR));

        // The naive mapping for the QEMU NIC lands nowhere useful.
        let naive_qemu = 32u8 + 10;
        assert!(
            !RESERVED_VECTORS.contains(&naive_qemu),
            "vector {naive_qemu} has no handler; routing there drops the IRQ"
        );
    }

    // ── the page-fault decision (BUG-042) ─────────────────────────────────

    use crate::interrupts::faults::{fault_verdict, FaultVerdict, LAZY_MAP_BASE, LAZY_MAP_END};

    /// Error-code bits as the handler reads them.
    const P: u64 = 0x1; // present
    const W: u64 = 0x2; // write
    const U: u64 = 0x4; // user
    const IF: u64 = 0x10; // instruction fetch

    const KERNEL_RIP: u64 = 0xFFFF_8000_0100_0000;
    const USER_RIP: u64 = 0x0040_0000;

    /// The fix in one test: a user task dereferencing a bad pointer is killed,
    /// not the kernel. Every "user, not lazily-mappable" fault is `KillUser`.
    #[test]
    fn a_user_fault_is_killed_not_panicked() {
        assert_eq!(
            fault_verdict(U, 0x0, USER_RIP),
            FaultVerdict::KillUser,
            "NULL"
        );
        assert_eq!(
            fault_verdict(U, 0xdead_beef, USER_RIP),
            FaultVerdict::KillUser,
            "wild pointer"
        );
        assert_eq!(
            fault_verdict(U | IF, 0x4000_0000, USER_RIP),
            FaultVerdict::KillUser,
            "instruction fetch outside the mapped image"
        );
    }

    /// A user *write* to a present read-only page is a protection violation
    /// (bit 0 set), and it must not be answered by mapping a fresh writable
    /// page: that is how `mprotect(PROT_READ)` used to be silently discarded.
    #[test]
    fn a_user_protection_violation_is_not_demand_paged() {
        assert_eq!(
            fault_verdict(U | W | P, LAZY_MAP_BASE + 0x1000, USER_RIP),
            FaultVerdict::KillUser,
            "present + write + user = protection violation, not demand paging"
        );
        // The same access on a *not-present* page is genuine demand paging.
        assert_eq!(
            fault_verdict(U | W, LAZY_MAP_BASE + 0x1000, USER_RIP),
            FaultVerdict::Recover
        );
    }

    /// Only a not-present fault inside the lazily-mapped window is recovered.
    #[test]
    fn only_the_lazy_window_is_demand_paged() {
        assert_eq!(
            fault_verdict(U, LAZY_MAP_BASE, USER_RIP),
            FaultVerdict::Recover,
            "the first byte of the window"
        );
        assert_eq!(
            fault_verdict(U, LAZY_MAP_END - 0x1000, USER_RIP),
            FaultVerdict::Recover,
            "the last page of the window"
        );
        assert_eq!(
            fault_verdict(U, LAZY_MAP_END, USER_RIP),
            FaultVerdict::KillUser,
            "one past the window is not mapped on demand"
        );
        assert_eq!(
            fault_verdict(U, LAZY_MAP_BASE - 1, USER_RIP),
            FaultVerdict::KillUser,
            "one below the window is not either"
        );
        // Below 0x1000 is the null page, never demand-paged.
        assert_eq!(fault_verdict(U, 0x800, USER_RIP), FaultVerdict::KillUser);
    }

    /// A supervisor fault is always fatal: the kernel touching memory it may not
    /// is a kernel bug, and "recovering" it would hide it.
    #[test]
    fn a_supervisor_fault_is_always_fatal() {
        assert_eq!(
            fault_verdict(0, 0xdead_beef, KERNEL_RIP),
            FaultVerdict::Panic
        );
        assert_eq!(
            fault_verdict(P, LAZY_MAP_BASE, KERNEL_RIP),
            FaultVerdict::Panic,
            "a supervisor protection violation is what SMAP raises"
        );
        // Even inside the window, the U/S bit decides.
        assert_eq!(
            fault_verdict(0, LAZY_MAP_BASE + 0x1000, KERNEL_RIP),
            FaultVerdict::Panic
        );
    }

    /// A fault with a kernel `RIP` is fatal even if the error code claims it came
    /// from user mode — that combination is a contradiction, not a recovery.
    #[test]
    fn a_user_fault_with_a_kernel_rip_is_fatal() {
        assert_eq!(fault_verdict(U, 0x1000, KERNEL_RIP), FaultVerdict::Panic);
        assert_eq!(
            fault_verdict(U, LAZY_MAP_BASE + 0x1000, KERNEL_RIP),
            FaultVerdict::Panic,
            "the window does not make a kernel-mode fault recoverable"
        );
    }
}
