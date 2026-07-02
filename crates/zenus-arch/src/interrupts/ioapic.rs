use core::sync::atomic::AtomicBool;
use crate::limine;

const IOAPIC_PHYS_BASE: u64 = 0xFEC00000;

fn ioapic_select(reg: u8) {
    let hhdm = limine::hhdm_offset();
    let base = (IOAPIC_PHYS_BASE + hhdm) as *mut u32;
    unsafe {
        base.write_volatile(reg as u32);
    }
}

fn ioapic_read(reg: u8) -> u32 {
    ioapic_select(reg);
    let data_ptr = (IOAPIC_PHYS_BASE + limine::hhdm_offset() + 0x10) as *mut u32;
    unsafe { data_ptr.read_volatile() }
}

fn ioapic_write(reg: u8, val: u32) {
    ioapic_select(reg);
    let data_ptr = (IOAPIC_PHYS_BASE + limine::hhdm_offset() + 0x10) as *mut u32;
    unsafe { data_ptr.write_volatile(val); }
}

static IOAPIC_INITIALIZED: AtomicBool = AtomicBool::new(false);

pub fn init() {
    let ioapic_id = ioapic_read(0x00);
    let ioapic_version = ioapic_read(0x01);
    let max_redir_entries = ((ioapic_version >> 16) & 0xFF) as u8;
    zenus_console::kinfo!("IOAPIC ID={:#x} version={:#x} max_redir={}", ioapic_id, ioapic_version, max_redir_entries);
    core::sync::atomic::AtomicBool::store(&IOAPIC_INITIALIZED, true, core::sync::atomic::Ordering::Relaxed);
}

pub fn is_initialized() -> bool {
    IOAPIC_INITIALIZED.load(core::sync::atomic::Ordering::Relaxed)
}

pub fn route_irq(gsi: u8, vector: u8, apic_id: u8) -> bool {
    let rte_index = match (0x10u16).checked_add((gsi as u16) * 2) {
        Some(idx) if idx <= 0xFF => idx as u8,
        _ => return false,
    };
    // Low: vector | masked(1) during setup
    ioapic_write(rte_index, vector as u32 | (1 << 16));
    // High: destination APIC ID (physical mode)
    ioapic_write(rte_index + 1, (apic_id as u32) << 24);
    // Unmask: clear bit 16
    ioapic_write(rte_index, vector as u32);

    // Verify
    let verify = ioapic_read(rte_index);
    (verify & 0xFF) == vector as u32
}
