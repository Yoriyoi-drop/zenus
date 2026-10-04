pub mod apic;
pub mod handler;
pub mod idt;
pub mod ioapic;
pub mod pit;

/// Preemption timer vector (LAPIC timer, armed by `apps::entry`).
pub const TIMER_VECTOR: u8 = 32;

/// Spurious interrupt vector (PIC IRQ 7/15, ACPI).
pub const SPURIOUS_VECTOR: u8 = 39;

/// Fixed vector for the NIC interrupt.
///
/// The handler lives at this slot (`idt::init` installs `interrupt_nic` here)
/// so both sides agree. It used to be computed as `32 + irq_line`, which for
/// QEMU's rtl8139 (IRQ line 10) produced vector 42 — a slot the stray
/// ack-and-return handler now claims, so `set_nic_irq_handler` never ran and
/// every RX interrupt was silently dropped.
pub const NIC_VECTOR: u8 = 43;

/// Fixed vector for the serial (COM1) interrupt: legacy IRQ 4 is 32 + 4.
pub const SERIAL_VECTOR: u8 = 36;

/// Every vector that carries a real handler, not the stray ack-and-return.
///
/// The IDT install loop skips exactly these, so a device routed onto any other
/// vector in 34..=255 lands on `interrupt_stray` and is acknowledged without
/// ever running its handler.
pub const RESERVED_VECTORS: [u8; 4] = [TIMER_VECTOR, SERIAL_VECTOR, NIC_VECTOR, SPURIOUS_VECTOR];

pub fn init() {
    idt::init();
    handler::init();
    ioapic::init();
}
