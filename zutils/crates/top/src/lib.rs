#![no_std]

use zutils_common::{Args, Writer};
use zenus_sched::scheduler;
use zenus_mem::allocator::ALLOCATOR;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let tasks = scheduler::list_tasks();
    let total = ALLOCATOR.total_size();
    let free = ALLOCATOR.free_size();
    let used = total - free;
    w.write_str("Zenus OS - Task Monitor\r\n");
    w.write_str("Tasks: ");
    w.write_u64(tasks.iter().flatten().count() as u64);
    w.write_str("  Heap: ");
    w.write_u64(used);
    w.write_str("/");
    w.write_u64(total);
    w.write_str("\r\n\r\n");
    w.write_str("PID\tState\t\tCPU\tUID\tGID\r\n");
    for info in tasks.iter().flatten() {
        w.write_u64(info.id);
        w.write_str("\t");
        w.write_str(info.state.to_str());
        for _ in info.state.to_str().len()..16 { w.write_byte(b' '); }
        w.write_u64(info.cpu as u64);
        w.write_str("\t");
        w.write_u64(info.uid as u64);
        w.write_str("\t");
        w.write_u64(info.gid as u64);
        w.write_str("\r\n");
    }
}
