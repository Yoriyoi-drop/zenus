pub mod apic;
pub mod pit;
pub mod idt;
pub mod handler;
pub mod ioapic;

pub fn init() {
    idt::init();
    handler::init();
    ioapic::init();
}
