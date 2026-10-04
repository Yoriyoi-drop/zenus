use core::sync::atomic::AtomicUsize;
use x86_64::structures::idt::InterruptStackFrame;

static NIC_IRQ_HANDLER: AtomicUsize = AtomicUsize::new(0);

// Kernel text bounds — defined by linker.ld
extern "C" {
    static __text_start: u8;
    static __text_end: u8;
}

/// EOI for the 8259 pair — only the BSP may send it.
///
/// A PIC EOI issued by an AP re-aims the next delivery of an unacked IRQ line
/// to that AP. IRQ0 (the PIT) is deliberately unmasked so the BSP can pick
/// it up through LINT0/ExtINT; an AP EOI therefore made the AP receive IRQ0
/// and enter the scheduler on a task the BSP was running.
fn pic_eoi_if_bsp() {
    if crate::smp::current_cpu() == 0 {
        unsafe {
            core::arch::asm!("out 0x20, al", in("al") 0x20u8, options(nostack, preserves_flags));
        }
    }
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
    pic_eoi_if_bsp();
    // APIC EOI (for ExtINT via LINT0)
    crate::interrupts::apic::eoi();
    crate::interrupts::pit::tick();
    // Flush serial output buffer so shell output appears in real time
    zenus_console::serial::flush_output();
}

#[no_mangle]
pub extern "x86-interrupt" fn interrupt_keyboard(_frame: InterruptStackFrame) {
    crate::keyboard::handle_irq1();
    pic_eoi_if_bsp();
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
        unsafe {
            core::arch::asm!("in al, dx", out("al") lsr, in("dx") 0x3FDu16, options(nostack, preserves_flags));
        }
        if lsr & 0x01 == 0 {
            break;
        }
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

/// Catch-all for vectors nobody claimed (34..=255 minus the ones above).
///
/// Leaving an IDT slot zeroed is not harmless: the CPU jumps to address 0 on
/// the first stray interrupt (QEMU raises IRQ7 spuriously, the cascade line
/// can be re-aimed, an APIC can deliver a bogus vector) and the machine dies
/// in a #PF/#DF cascade. Ack the interrupt (so it cannot storm) and return.
#[no_mangle]
pub extern "x86-interrupt" fn interrupt_stray(_frame: InterruptStackFrame) {
    pic_eoi_if_bsp();
    // Known limitation: this handler is installed on every vector from 34 to
    // 255 that has no owner, and it acknowledges unconditionally. For a vector
    // that did *not* arrive through the LAPIC (a legacy PIC IRQ, a stray `int`)
    // there is no LAPIC ISR bit to clear, and the EOI clears whichever vector
    // is in service on this CPU instead — which can swallow an unrelated
    // interrupt.
    //
    // Gating it properly needs the vector number, and
    // `x86_interrupt::InterruptStackFrame` does not expose it. Reading it back
    // out of the frame by hand would depend on the gate type and stack width,
    // so this stays documented rather than guessed.
    crate::interrupts::apic::eoi();
}
