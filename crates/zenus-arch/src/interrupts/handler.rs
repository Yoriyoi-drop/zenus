use x86_64::structures::idt::InterruptStackFrame;
use core::sync::atomic::AtomicUsize;

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
    // PIC EOI (master)
    unsafe { core::arch::asm!("out 0x20, al", in("al") 0x20u8); }
    // APIC EOI (for ExtINT via LINT0)
    crate::interrupts::apic::eoi();
    crate::interrupts::pit::tick();
    // Flush serial output buffer so shell output appears in real time
    zenus_console::serial::flush_output();
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_keyboard(_frame: InterruptStackFrame) {
    crate::keyboard::handle_irq1();
    // PIC EOI (master) — required if IRQ1 ever touches the PIC path
    unsafe { core::arch::asm!("out 0x20, al", in("al") 0x20u8); }
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
pub extern "x86-interrupt" fn interrupt_serial(_frame: InterruptStackFrame) {
    // Read all available bytes from UART and push into interrupt buffer.
    // NOTE: LSR (Line Status Register) must be RELOADED each iteration
    // because reading the data port (0x3F8) clears the LSR's DR bit.
    loop {
        let lsr: u8;
        unsafe { core::arch::asm!("in al, dx", out("al") lsr, in("dx") 0x3FDu16, options(nostack, preserves_flags)); }
        if lsr & 0x01 == 0 { break; }
        zenus_console::serial::irq_handler_serial();
    }
    crate::interrupts::apic::eoi();
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_nic(_frame: InterruptStackFrame) {
    let ptr = NIC_IRQ_HANDLER.load(core::sync::atomic::Ordering::Acquire);
    if ptr != 0 && ptr_in_text(ptr) {
        let handler: fn() = unsafe { core::mem::transmute_copy(&ptr) };
        handler();
    }
    crate::interrupts::apic::eoi();
}

pub fn init() {
    zenus_console::kinfo!("Interrupt handlers installed");
}
