#![no_std]

use zutils_common::{Args, Writer};
use zenus_sched::scheduler;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("PID\tUID\tGID\tState\tPidNS\tUtsNS\r\n");
    let tasks = scheduler::list_tasks();
    for info in tasks.iter().flatten() {
        w.write_u64(info.id);
        w.write_str("\t");
        w.write_u64(info.uid as u64);
        w.write_str("\t");
        w.write_u64(info.gid as u64);
        w.write_str("\t");
        w.write_str(info.state.to_str());
        w.write_str("\t");
        w.write_u64(info.pid_ns as u64);
        w.write_str("\t");
        w.write_u64(info.uts_ns as u64);
        if info.id == scheduler::current_task_id() {
            w.write_str(" (current)");
        }
        w.write_str("\r\n");
    }
}
