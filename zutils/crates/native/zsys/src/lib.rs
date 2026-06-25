#![no_std]

use zutils_common::{Args, Writer};
use zenus_arch::interrupts::pit::get_ticks;
use zenus_sched::scheduler;
use zenus_sched::task::TaskState;
use zenus_arch::acpi;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let subcmd = args.get(1).unwrap_or("status");
    match subcmd {
        "status" => {
            w.write_str("System status:\r\n");
            let seconds = get_ticks() / 100;
            w.write_str("  Uptime:  ");
            w.write_u64(seconds);
            w.write_str("s\r\n");
            let tasks = scheduler::list_tasks();
            let active = tasks.iter().flatten().filter(|t| t.state == TaskState::Ready || t.state == TaskState::Running).count();
            w.write_str("  Tasks:   ");
            w.write_u64(active as u64);
            w.write_str(" active\r\n");
            w.write_str("  Version: Zenus OS v0.1.0\r\n");
        }
        "halt" | "shutdown" => {
            w.write_str("System halting...\r\n");
            acpi::shutdown_via_acpi();
        }
        "reboot" => {
            w.write_str("System rebooting...\r\n");
            acpi::reboot_via_keyboard();
        }
        "uptime" => {
            let seconds = get_ticks() / 100;
            let minutes = seconds / 60;
            let hours = minutes / 60;
            let days = hours / 24;
            w.write_str("Uptime: ");
            w.write_u64(days);
            w.write_str("d ");
            w.write_u64(hours % 24);
            w.write_str("h ");
            w.write_u64(minutes % 60);
            w.write_str("m ");
            w.write_u64(seconds % 60);
            w.write_str("s\r\n");
        }
        _ => {
            w.write_str("Usage: zsys <status|halt|reboot|uptime>\r\n");
        }
    }
}
