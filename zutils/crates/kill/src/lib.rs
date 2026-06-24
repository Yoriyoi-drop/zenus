#![no_std]

use zutils_common::{Args, Writer};
use zenus_sched::scheduler;

pub fn execute<W: Writer + ?Sized>(args: &Args, w: &mut W) {
    let pid_str = match args.args().iter().find(|a| !a.is_empty()) {
        Some(p) => p,
        None => {
            w.write_str("kill: missing pid\r\n");
            return;
        }
    };

    let pid = match pid_str.parse::<u64>() {
        Ok(p) => p,
        Err(_) => {
            w.write_str("kill: invalid pid\r\n");
            return;
        }
    };

    if pid == 0 {
        w.write_str("kill: cannot kill idle process\r\n");
        return;
    }

    let current_pid = scheduler::current_task_id();
    if pid == current_pid {
        w.write_str("kill: cannot kill the shell itself\r\n");
        return;
    }

    if scheduler::kill_task(pid) {
        w.write_str("killed: ");
        w.write_u64(pid);
        w.write_str("\r\n");
    } else {
        w.write_str("kill: not found: ");
        w.write_u64(pid);
        w.write_str("\r\n");
    }
}
