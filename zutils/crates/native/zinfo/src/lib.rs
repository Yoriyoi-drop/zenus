#![no_std]

use zutils_common::{Args, Writer};
use zenus_mem::allocator::ALLOCATOR;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let hhdm = zenus_arch::limine::hhdm_offset();
    let boot_time = zenus_arch::rtc::boot_time();
    let ticks = zenus_arch::interrupts::pit::get_ticks();
    let total = ALLOCATOR.total_size();
    let free = ALLOCATOR.free_size();
    let used = total - free;
    let cpu_count = zenus_arch::smp::cpu_count();

    w.write_str("Zenus OS v0.1.0\r\n");
    w.write_str("========================\r\n");
    w.write_str("Arch:         x86_64\r\n");
    w.write_str("CPUs:         ");
    w.write_u64(cpu_count as u64);
    w.write_str("\r\n");
    w.write_str("HHDM:         0x");
    w.write_hex(hhdm);
    w.write_str("\r\n");
    w.write_str("Heap:         ");
    w.write_u64(used);
    w.write_str("/");
    w.write_u64(total);
    w.write_str("\r\n");
    w.write_str("System ticks: ");
    w.write_u64(ticks);
    w.write_str(" (");
    w.write_u64(ticks / 100);
    w.write_str("s uptime)\r\n");

    if let Some(bt) = boot_time {
        let mut buf = [0u8; 20];
        let len = zenus_arch::rtc::format_time(&bt, &mut buf);
        w.write_str("Boot time:    ");
        w.write_str(core::str::from_utf8(&buf[..len]).unwrap_or("?"));
        w.write_str("\r\n");
    }

    w.write_str("Bootloader:   Limine\r\n");
    w.write_str("Target:       Production Server OS\r\n");
}
