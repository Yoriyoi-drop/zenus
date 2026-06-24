#![no_std]

use zutils_common::{Args, Writer};
use zenus_mem::frame_allocator;
use zenus_mem::allocator::ALLOCATOR;

pub fn execute<W: Writer + ?Sized>(_args: &Args, w: &mut W) {
    let free_head = ALLOCATOR.free_head_addr();
    w.write_str("Heap: 4MB free-list allocator\r\n");
    w.write_str("  Free list head: 0x");
    w.write_hex(free_head as u64);
    w.write_str("\r\n");

    let fa = frame_allocator::FRAME_ALLOCATOR.lock();
    w.write_str("Physical frames:\r\n");
    w.write_str("  Total: ");
    w.write_u64(fa.total_memory() / 4096);
    w.write_str(" frames (");
    w.write_u64(fa.total_memory() / (1024*1024));
    w.write_str(" MB)\r\n");
    w.write_str("  Used:  ");
    w.write_u64(fa.used_memory() / 4096);
    w.write_str(" frames (");
    w.write_u64(fa.used_memory() / (1024*1024));
    w.write_str(" MB)\r\n");
    w.write_str("  Free stack: ");
    w.write_u64(fa.free_frames_count() as u64);
    w.write_str(" frames\r\n");
}
