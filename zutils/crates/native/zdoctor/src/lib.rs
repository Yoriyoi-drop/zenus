#![no_std]

use zutils_common::{Args, Writer};

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    w.write_str("=== Zenus Health Check ===\r\n");

    let (hits, misses) = zenus_fs::block_cache::bc_stats();
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

    let result = zenus_fs::ext2_fsck::fsck(0);
    w.write_str("Ext2 fsck:    ");
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
