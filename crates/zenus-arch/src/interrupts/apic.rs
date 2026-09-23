use core::sync::atomic::{AtomicBool, AtomicU64, Ordering};

pub static LAPIC_VIRT_BASE: AtomicU64 = AtomicU64::new(0);
static X2APIC_MODE: AtomicBool = AtomicBool::new(false);

fn lapic_base() -> *mut u32 {
    LAPIC_VIRT_BASE.load(Ordering::Relaxed) as *mut u32
}

fn lapic_read(reg: u32) -> u32 {
    if X2APIC_MODE.load(Ordering::Relaxed) {
        // x2APIC: use MSR-based access. MSR = 0x800 + (reg >> 4)
        let msr = 0x800u32 + (reg >> 4);
        unsafe { crate::cpu::read_msr(msr as u32) as u32 }
    } else {
        // xAPIC: use memory-mapped access
        unsafe {
            let addr = (lapic_base() as usize).wrapping_add(reg as usize);
            (addr as *const u32).read_volatile()
        }
    }
}

fn lapic_write(reg: u32, val: u32) {
    if X2APIC_MODE.load(Ordering::Relaxed) {
        // x2APIC: use MSR-based access. MSR = 0x800 + (reg >> 4)
        let msr = 0x800u32 + (reg >> 4);
        unsafe {
            crate::cpu::write_msr(msr as u32, val as u64);
        }
    } else {
        // xAPIC: use memory-mapped access
        unsafe {
            let addr = (lapic_base() as usize).wrapping_add(reg as usize);
            (addr as *mut u32).write_volatile(val);
        }
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
    if X2APIC_MODE.load(Ordering::Relaxed) {
        (unsafe { crate::cpu::read_msr(0x802) } >> 24) as u32
    } else {
        lapic_read(0x20) >> 24
    }
}

fn enable_lapic() {
    // IA32_APIC_BASE: bit 10 = EN, bit 11 = EXTD (x2APIC enable)
    // Intel SDM: Once bit 11 is set to 1, it cannot be cleared (requires reset).
    // So we must DETECT x2APIC and use MSR-based access if already enabled.
    let base_raw = unsafe { crate::cpu::read_msr(0x1B) };
    let x2apic = (base_raw & (1 << 11)) != 0;
    let apic_was_disabled = (base_raw & (1 << 10)) == 0;

    if x2apic {
        zenus_console::kinfo!("x2APIC mode detected (EXTD=1) — using MSR-based APIC access");
        X2APIC_MODE.store(true, Ordering::Relaxed);
    } else {
        zenus_console::kinfo!("xAPIC mode (EXTD=0) — using memory-mapped APIC access");
        X2APIC_MODE.store(false, Ordering::Relaxed);
    }

    if apic_was_disabled {
        zenus_console::kinfo!("Enabling APIC (IA32_APIC_BASE.EN)");
        // Just enable APIC, don't touch x2APIC bit (may be locked)
        unsafe {
            crate::cpu::write_msr(0x1B, base_raw | (1 << 10));
        }
    }

    let val = lapic_read(0xF0);
    let apic_id = current_apic_id();
    zenus_console::kinfo!("APIC SVR={:#x} APIC ID={:#x}", val, apic_id);
    // Keep APIC enabled, set spurious vector to 39 (our handler)
    lapic_write(0xF0, (val | 0x100) & !0xFF | 39);
    let svr2 = lapic_read(0xF0);
    zenus_console::kinfo!("APIC SVR after enable={:#x}", svr2);
    // Mask all LVT entries (APs call this too — keep LINT0 masked for them)
    if !X2APIC_MODE.load(Ordering::Relaxed) {
        // CMCI MSR (0x82F) may not be accessible in x2APIC mode on some CPUs (KVM).
        // Skip it to avoid #GP.
        lapic_write(0x2F0, 0x0100FF); // CMCI: masked
    }
    lapic_write(0x320, 0x00010000); // Timer: masked
    lapic_write(0x330, 0x0100FF); // Thermal: masked
    lapic_write(0x340, 0x0100FF); // Performance Counter: masked
    lapic_write(0x350, 0x0100FF); // LINT0: masked by default; BSP calls enable_pic_lint0()
    lapic_write(0x360, 0x0100FF); // LINT1: masked (bit 16), vector 0xFF
    lapic_write(0x370, 0x0100FF); // Error: masked
    lapic_write(0x380, 0); // Timer initial count = 0 (no fire)
}

/// Enable LINT0 in ExtINT mode to accept PIC interrupts.
/// Only call on BSP; APs keep LINT0 masked.
/// In x2APIC mode, ExtINT delivery is NOT supported (Intel SDM: invalid).
/// We route PIT via IOAPIC or keep PIC interrupt disabled instead.
pub fn enable_pic_lint0() {
    if X2APIC_MODE.load(Ordering::Relaxed) {
        // x2APIC: ExtINT is not valid on LINT0 per Intel SDM.
        // Instead, route PIT IRQ0 directly through the IOAPIC.
        zenus_console::kinfo!("LINT0: routing PIT through IOAPIC (x2APIC mode)");
        crate::interrupts::ioapic::route_irq(0, 32, 0); // PIT IRQ0 → vector 32
    } else {
        lapic_write(0x350, 0x700 | 32); // LINT0: ExtINT mode, unmasked
        zenus_console::kinfo!("LINT0: ExtINT mode enabled");
    }
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
    lapic_write(0x3E0, 0xB); // divide by 1
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

        // Keep IRQ0 (PIT) and IRQ1 (keyboard) unmasked; mask everything else
        core::arch::asm!("out 0x21, al", in("al") 0xFCu8);
        core::arch::asm!("out 0xA1, al", in("al") 0xFFu8);
    }
}
