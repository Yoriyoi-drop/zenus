use core::sync::atomic::{AtomicU64, Ordering};
use x86_64::instructions::interrupts;

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
    if !interrupts::are_enabled() {
        remap_pic();
    }

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
    let val = lapic_read(0xF0);
    let apic_id = lapic_read(0x20) >> 24;
    let s = zenus_console::serial::SerialPort::new(0x3F8);
    s.write_str("[APIC] SVR=0x");
    s.write_hex(val as u64);
    s.write_str(" APIC ID=0x");
    s.write_hex(apic_id as u64);
    s.write_str("\n");
    // Keep APIC enabled, set spurious vector to 39 (our handler)
    lapic_write(0xF0, (val | 0x100) & !0xFF | 39);
    let svr2 = lapic_read(0xF0);
    s.write_str("[APIC] SVR after enable=0x");
    s.write_hex(svr2 as u64);
    s.write_str("\n");
    lapic_write(0x80, 0);              // TPR = 0: allow all interrupt priorities
    lapic_write(0x60, 0x0100FF);       // LINT0: masked (bit 16), vector 0xFF
}

pub fn eoi() {
    lapic_write(0xB0, 0);
}

#[no_mangle]
pub extern "C" fn apic_timer_eoi() {
    lapic_write(0xB0, 0);
}

pub fn init_timer(vector: u8) {
    lapic_write(0x3E0, 0xB);          // divide by 1
    lapic_write(0x380, 10_000_000);   // count = 10M, ~100ms at 100MHz bus
    lapic_write(0x320, vector as u32 | 0x20000); // periodic mode
}

pub fn init_timer_ap(vector: u8) {
    lapic_write(0x3E0, 0xB);
    lapic_write(0x380, 10_000_000);
    lapic_write(0x320, vector as u32 | 0x20000);
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

        // Mask all IRQs — we use APIC timer, not PIC
        core::arch::asm!("out 0x21, al", in("al") 0xFFu8);
        core::arch::asm!("out 0xA1, al", in("al") 0xFFu8);
    }
}
