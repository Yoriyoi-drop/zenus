pub mod apic;
pub mod handler;
pub mod idt;
pub mod ioapic;
pub mod pit;

pub fn init() {
    idt::init();
    handler::init();
    ioapic::init();
}
