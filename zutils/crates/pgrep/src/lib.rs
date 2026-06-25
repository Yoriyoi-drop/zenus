#![no_std]

use zutils_common::{Args, Writer};
use zenus_sched::scheduler;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let pid_filter = args.get(1).and_then(|a| a.parse::<u64>().ok());
    let tasks = scheduler::list_tasks();
    let mut found = false;
    w.write_str("PID\tState\t\tCPU\tUID\tGID\r\n");
    for info in tasks.iter().flatten() {
        if let Some(pid) = pid_filter {
            if info.id != pid { continue; }
        }
        found = true;
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
    if !found {
        if let Some(pid) = pid_filter {
            w.write_str("No task with PID: ");
            w.write_u64(pid);
            w.write_str("\r\n");
        } else {
            w.write_str("No tasks\r\n");
        }
    }
}
