#![no_std]

use zutils_common::{Args, Writer};
use zenus_sched::scheduler;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("PID\tState\t\tName\t\t\t\tUID\tGID\r\n");
    w.write_str("---\t-----\t\t----\t\t\t\t---\t---\r\n");
    let tasks = scheduler::list_tasks();
    for info in tasks.iter().flatten() {
        w.write_u64(info.id);
        w.write_str("\t");
        w.write_str(info.state.to_str());
        let pad = 16usize.saturating_sub(info.state.to_str().len());
        for _ in 0..pad { w.write_byte(b' '); }
        // Display task name
        let name_len = info.name.iter().position(|&b| b == 0).unwrap_or(32);
        if name_len > 0 {
            w.write_str(core::str::from_utf8(&info.name[..name_len]).unwrap_or("?"));
        } else {
            w.write_str("-");
        }
        for _ in name_len..24 { w.write_byte(b' '); }
        w.write_u64(info.uid as u64);
        w.write_str("\t");
        w.write_u64(info.gid as u64);
        if info.id == scheduler::current_task_id() {
            w.write_str(" *");
        }
        w.write_str("\r\n");
    }
}
