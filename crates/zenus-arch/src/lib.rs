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
}
