#![no_std]

use zenus_mem::allocator::ALLOCATOR;
use zenus_sched::scheduler;
use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let tasks = scheduler::list_tasks();
    let uptime_ticks = scheduler::uptime_ticks();
    let uptime_secs = uptime_ticks / 100;
    let total = ALLOCATOR.total_size();
    let free = ALLOCATOR.free_size();
    let used = total - free;
    w.write_str("Zenus OS - Task Monitor\r\n");
    w.write_str("Uptime: ");
    w.write_u64(uptime_secs);
    w.write_str("s  Tasks: ");
    w.write_u64(tasks.iter().flatten().count() as u64);
    w.write_str("  Heap: ");
    w.write_u64(used as u64);
    w.write_str("/");
    w.write_u64(total as u64);
    let pct = if total > 0 { used * 100 / total } else { 0 };
    w.write_str(" (");
    w.write_u64(pct as u64);
    w.write_str("%)\r\n");
    w.write_str("PID\tState\t\tCPU\tName\t\t\tUID\tGID\r\n");
    for info in tasks.iter().flatten() {
        w.write_u64(info.id);
        w.write_str("\t");
        w.write_str(info.state.to_str());
        for _ in info.state.to_str().len()..16 {
            w.write_byte(b' ');
        }
        w.write_u64(info.cpu as u64);
        w.write_str("\t");
        let name_len = info.name.iter().position(|&b| b == 0).unwrap_or(32);
        if name_len > 0 {
            w.write_str(core::str::from_utf8(&info.name[..name_len]).unwrap_or("?"));
        } else {
            w.write_str("-");
        }
        for _ in name_len..16 {
            w.write_byte(b' ');
        }
        w.write_u64(info.uid as u64);
        w.write_str("\t");
        w.write_u64(info.gid as u64);
        w.write_str("\r\n");
    }
}
