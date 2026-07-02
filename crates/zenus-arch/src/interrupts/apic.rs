use core::sync::atomic::{AtomicU64, Ordering};

pub static LAPIC_VIRT_BASE: AtomicU64 = AtomicU64::new(0);

fn lapic_base() -> *mut u32 {
    LAPIC_VIRT_BASE.load(Ordering::Relaxed) as *mut u32
}

fn lapic_read(reg: u32) -> u32 {
    unsafe {
        let addr = (lapic_base() as usize).wrapping_add(reg as usize);
        (addr as *const u32).read_volatile()
    }
}

fn lapic_write(reg: u32, val: u32) {
    unsafe {
        let addr = (lapic_base() as usize).wrapping_add(reg as usize);
        (addr as *mut u32).write_volatile(val);
    }
}

pub fn init_with_virt(virt: u64) {
    LAPIC_VIRT_BASE.store(virt, Ordering::Relaxed);
    remap_pic();

    enable_lapic();
}

pub fn init_ap(virt: u64) {
    LAPIC_VIRT_BASE.store(virt, Ordering::Relaxed);
    enable_lapic();
}

pub fn current_apic_id() -> u32 {
    lapic_read(0x20) >> 24
}

fn enable_lapic() {
    // Ensure xAPIC mode (not x2APIC): set EN bit 10, clear EXTD bit 11
    let base_raw = unsafe { crate::cpu::read_msr(0x1B) };
    if (base_raw & (1 << 11)) != 0 {
        zenus_console::kinfo!("Switching from x2APIC to xAPIC mode");
    }
    if (base_raw & (1 << 10)) == 0 {
        zenus_console::kinfo!("Enabling APIC (IA32_APIC_BASE.EN)");
    }
    unsafe { crate::cpu::write_msr(0x1B, (base_raw | (1 << 10)) & !(1 << 11)); }

    let val = lapic_read(0xF0);
    let apic_id = lapic_read(0x20) >> 24;
    zenus_console::kinfo!("APIC SVR={:#x} APIC ID={:#x}", val, apic_id);
    // Keep APIC enabled, set spurious vector to 39 (our handler)
    lapic_write(0xF0, (val | 0x100) & !0xFF | 39);
    let svr2 = lapic_read(0xF0);
    zenus_console::kinfo!("APIC SVR after enable={:#x}", svr2);
    // Mask all LVT entries (APs call this too — keep LINT0 masked for them)
    lapic_write(0x2F0, 0x0100FF);      // CMCI: masked
    lapic_write(0x320, 0x00010000);    // Timer: masked
    lapic_write(0x330, 0x0100FF);      // Thermal: masked
    lapic_write(0x340, 0x0100FF);      // Performance Counter: masked
    lapic_write(0x350, 0x0100FF);      // LINT0: masked by default; BSP calls enable_pic_lint0()
    lapic_write(0x360, 0x0100FF);      // LINT1: masked (bit 16), vector 0xFF
    lapic_write(0x370, 0x0100FF);      // Error: masked
    lapic_write(0x380, 0);             // Timer initial count = 0 (no fire)
}

/// Enable LINT0 in ExtINT mode to accept PIC interrupts.
/// Only call on BSP; APs keep LINT0 masked.
pub fn enable_pic_lint0() {
    lapic_write(0x350, 0x700 | 32);    // LINT0: ExtINT mode, unmasked
}

pub fn lapic_read_reg(reg: u32) -> u32 {
    lapic_read(reg)
}

pub fn eoi() {
    lapic_write(0xB0, 0);
}

pub fn lapic_write_reg(reg: u32, val: u32) {
    lapic_write(reg, val);
}

#[no_mangle]
pub extern "C" fn apic_timer_eoi() {
    lapic_write(0xB0, 0);
}

pub fn init_timer(vector: u8) {
    lapic_write(0x3E0, 0xB);          // divide by 1
    // Use a count large enough that the timer NEVER fires during the ISR.
    // On QEMU KVM the APIC timer runs at TSC frequency (~2 GHz), so each
    // tick is 50 μs with INITCNT=100_000 — shorter than the ISR execution
    // time. This causes a nested timer interrupt between popfq and jmp rax
    // in the ISR return path, corrupting the target task's saved RIP.
    // With 50_000_000 ticks: 25 ms at 2 GHz, 500 ms at 100 MHz.
    // TIME_SLICE=5 → every task runs for ~125 ms, which is still snappy.
    lapic_write(0x380, 1_000_000);
    lapic_write(0x320, vector as u32 | 0x20000); // periodic mode, unmasked
}

pub fn init_timer_ap(vector: u8) {
    lapic_write(0x3E0, 0xB);
    lapic_write(0x380, 0);
    lapic_write(0x320, 0x00010000);
    lapic_write(0x380, 100_000);
    // Keep timer MASKED initially — BSP will broadcast IPI to start AP timers
    // after all APs have signaled readiness.
    lapic_write(0x320, vector as u32 | 0x20000 | 0x10000);
}

pub fn send_ipi(cpu_id: u8, vector: u8) {
    let icr = (cpu_id as u32) << 24 | vector as u32;
    lapic_write(0x300, icr);
    // Wait for delivery
    while (lapic_read(0x300) & (1 << 12)) != 0 {
        core::hint::spin_loop();
    }
}

fn remap_pic() {
    unsafe {
        // Master PIC
        core::arch::asm!("out 0x20, al", in("al") 0x11u8);
        core::arch::asm!("out 0x21, al", in("al") 0x20u8);
        core::arch::asm!("out 0x21, al", in("al") 0x04u8);
        core::arch::asm!("out 0x21, al", in("al") 0x01u8);

        // Slave PIC
        core::arch::asm!("out 0xA0, al", in("al") 0x11u8);
        core::arch::asm!("out 0xA1, al", in("al") 0x28u8);
        core::arch::asm!("out 0xA1, al", in("al") 0x02u8);
        core::arch::asm!("out 0xA1, al", in("al") 0x01u8);

        // Keep IRQ0 (PIT) unmasked; mask everything else
        core::arch::asm!("out 0x21, al", in("al") 0xFEu8);
        core::arch::asm!("out 0xA1, al", in("al") 0xFFu8);
    }
}
