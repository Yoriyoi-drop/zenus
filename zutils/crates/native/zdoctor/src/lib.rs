#![no_std>

extern crate alloc;

use zutils_common::{Args, Writer};
use zenus_fs::ext2_fsck;
use zenus_fs::block_cache;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("=== Zenus Health Check ===\r\n");

    let (hits, misses) = block_cache::bc_stats();
    let total_io = hits + misses;
    w.write_str("Block cache:  ");
    w.write_u64(hits);
    w.write_str(" hits, ");
    w.write_u64(misses);
    w.write_str(" misses");
    if total_io > 0 {
        w.write_str(" (");
        w.write_u64(hits * 100 / total_io);
        w.write_str("% hit rate)");
    }
    w.write_str("\r\n");

    w.write_str("Ext2 fsck:    ");
    let result = ext2_fsck::fsck(0);
    if result.passed() {
        w.write_str("PASSED");
    } else {
        w.write_str("FAILED");
    }
    w.write_str(" (");
    w.write_u64(result.errors as u64);
    w.write_str(" errors, ");
    w.write_u64(result.warnings as u64);
    w.write_str(" warnings)\r\n");

    let tasks = zenus_sched::scheduler::list_tasks();
    let active = tasks.iter().flatten().filter(|t| t.state == zenus_sched::task::TaskState::Ready || t.state == zenus_sched::task::TaskState::Running).count();
    w.write_str("Active tasks: ");
    w.write_u64(active as u64);
    w.write_str("\r\n");

    if zenus_arch::watchdog::watchdog_is_active() {
        w.write_str("Watchdog:     OK\r\n");
    } else {
        w.write_str("Watchdog:     NOT ACTIVE\r\n");
    }

    w.write_str("Status:       ");
    if result.passed() {
        w.write_str("HEALTHY\r\n");
    } else {
        w.write_str("ISSUES FOUND\r\n");
    }
}
