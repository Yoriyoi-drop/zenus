use x86_64::structures::idt::InterruptStackFrame;
use core::sync::atomic::AtomicUsize;
use zenus_console::serial::SerialPort;

static NIC_IRQ_HANDLER: AtomicUsize = AtomicUsize::new(0);

// Kernel text bounds — defined by linker.ld
extern "C" {
    static __text_start: u8;
    static __text_end: u8;
}

fn ptr_in_text(ptr: usize) -> bool {
    let start = unsafe { &__text_start as *const u8 as usize };
    let end = unsafe { &__text_end as *const u8 as usize };
    ptr >= start && ptr < end
}

pub fn set_nic_irq_handler(handler: fn()) {
    NIC_IRQ_HANDLER.store(handler as usize, core::sync::atomic::Ordering::Relaxed);
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_timer(_frame: InterruptStackFrame) {
    crate::interrupts::apic::eoi();
    crate::interrupts::pit::tick();
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_keyboard(_frame: InterruptStackFrame) {
    crate::keyboard::handle_irq1();
    crate::interrupts::apic::eoi();
}

pub fn get_timer_tick() -> u64 {
    crate::interrupts::pit::get_ticks()
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_spurious(_frame: InterruptStackFrame) {
    core::sync::atomic::compiler_fence(core::sync::atomic::Ordering::SeqCst);
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_nic(_frame: InterruptStackFrame) {
    let ptr = NIC_IRQ_HANDLER.load(core::sync::atomic::Ordering::Relaxed);
    if ptr != 0 && ptr_in_text(ptr) {
        let handler: fn() = unsafe { core::mem::transmute_copy(&ptr) };
        handler();
    }
    crate::interrupts::apic::eoi();
}

pub fn init() {
    SerialPort::new(0x3F8).write_str("[OK] Interrupt handlers installed\n");
}
