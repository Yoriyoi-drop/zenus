#![no_std]

use zutils_common::{Args, Writer};
use zenus_mem::allocator::ALLOCATOR;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    let hhdm = zenus_arch::limine::hhdm_offset();
    let total = ALLOCATOR.total_size();
    let free = ALLOCATOR.free_size();
    let used = total - free;

    w.write_str("=== Zenus Diagnostics ===\r\n");
    w.write_str("System ticks: "); w.write_u64(ticks); w.write_str("\r\n");
    w.write_str("HHDM offset:  0x"); w.write_hex(hhdm); w.write_str("\r\n");
    w.write_str("Heap total:   "); w.write_u64(total as u64); w.write_str(" bytes\r\n");
    w.write_str("Heap used:    "); w.write_u64(used as u64); w.write_str(" bytes\r\n");
    w.write_str("Heap free:    "); w.write_u64(free as u64); w.write_str(" bytes\r\n");

    if zenus_arch::watchdog::watchdog_is_active() {
        w.write_str("Watchdog:     ACTIVE (");
        w.write_u64(zenus_arch::watchdog::watchdog_get_remaining() as u64);
        w.write_str("s remaining)\r\n");
    } else {
        w.write_str("Watchdog:     INACTIVE\r\n");
    }

    let cpu_count = zenus_arch::smp::cpu_count();
    w.write_str("CPUs:         "); w.write_u64(cpu_count as u64); w.write_str("\r\n");
}
