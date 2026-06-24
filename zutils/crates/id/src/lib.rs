#![no_std]

use zutils_common::{Args, Writer};
use zenus_sched::scheduler;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let uid = scheduler::current_uid();
    let euid = scheduler::current_euid();
    let gid = scheduler::current_gid();
    let egid = scheduler::current_egid();
    w.write_str("uid=");
    w.write_u64(uid as u64);
    if euid != uid {
        w.write_str(" euid=");
        w.write_u64(euid as u64);
    }
    w.write_str(" gid=");
    w.write_u64(gid as u64);
    if egid != gid {
        w.write_str(" egid=");
        w.write_u64(egid as u64);
    }
    w.write_str("\r\n");
}
