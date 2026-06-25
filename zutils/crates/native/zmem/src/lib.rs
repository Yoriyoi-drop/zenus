#![no_std]

use zutils_common::{Args, Writer};
use zenus_mem::allocator::ALLOCATOR;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let total = ALLOCATOR.total_size();
    let free = ALLOCATOR.free_size();
    let used = total - free;

    w.write_str("Memory Information\r\n");
    w.write_str("=================\r\n");
    w.write_str("Heap (total):  ");
    w.write_u64(total as u64);
    w.write_str(" bytes (");
    w.write_u64(total as u64 / 1024);
    w.write_str(" KB)\r\n");
    w.write_str("Heap (used):   ");
    w.write_u64(used as u64);
    w.write_str(" bytes (");
    w.write_u64(used as u64 / 1024);
    w.write_str(" KB)\r\n");
    w.write_str("Heap (free):   ");
    w.write_u64(free as u64);
    w.write_str(" bytes (");
    w.write_u64(free as u64 / 1024);
    w.write_str(" KB)\r\n");

    if total > 0 {
        let pct = used * 100 / total;
        w.write_str("Usage:         ");
        w.write_u64(pct as u64);
        w.write_str("%\r\n");
    }
}
